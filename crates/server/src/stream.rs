//! One speech request from text to PCM: phonemize chunks in order on a blocking thread, keep a
//! few of them on the worker, and send the audio back in order as soon as the next one is ready.
//!
//! The chunk the client is waiting for goes to the worker at priority 0, later ones at priority 1,
//! and at most `lookahead` chunks are ahead of the client at a time, so one long request can't
//! fill the worker's queue. Phonemizing stops when the window is full, and everything stops when
//! the client goes away: dropping a queued chunk's ticket cancels it on the worker.
//!
//! Both stages mark their end explicitly, so a stage that dies (a panic) fails the response
//! instead of ending it early as if it were complete.

use crate::audio::{silence, trim_range};
use crate::pipeline::{Chunk, Pipeline, PipelineError};
use crate::protocol::Priority;
use crate::request::Speech;
use crate::stitch::{self, STITCH_ABOVE, Stitcher};
use crate::worker::{Connection, HeldAudio, SynthError, SynthRequest, Synthesized};
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::FuturesOrdered;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamError {
    Pipeline(PipelineError),
    Synth(SynthError),
    /// A stage stopped without finishing or reporting an error.
    Aborted,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(e) => write!(f, "text processing failed: {e}"),
            Self::Synth(e) => e.fmt(f),
            Self::Aborted => f.write_str("speech generation stopped unexpectedly"),
        }
    }
}

impl std::error::Error for StreamError {}

/// How a rendered piece of speech becomes output.
#[derive(Debug, Clone)]
struct Finish {
    /// The chunk's text and pace, which set how much trailing silence is kept.
    text: Arc<str>,
    speed: f64,
    stitch: Option<(Arc<stitch::Plan>, usize)>,
}

#[derive(Debug)]
enum Unit {
    Silence(usize),
    Speech(SynthRequest, Finish),
}

enum Produced {
    Unit(Unit),
    Failed(PipelineError),
    End,
}

fn units(chunk: Chunk, speech: &Speech) -> Vec<Unit> {
    if let Some(pause) = chunk.pause {
        return if pause > 0.0 { vec![Unit::Silence(silence(pause))] } else { Vec::new() };
    }
    if chunk.ids.is_empty() {
        return Vec::new();
    }
    let text: Arc<str> = chunk.text.into();
    let request = |ids: Vec<u8>, style_len: usize| SynthRequest {
        speed: speech.speed,
        voice: speech.voice,
        ids,
        style_len: style_len.min(usize::from(u16::MAX)) as u16,
    };
    let finish = |stitch| Finish { text: text.clone(), speed: speech.speed.as_f64(), stitch };
    if chunk.phonemes.chars().count() <= STITCH_ABOVE {
        let n = chunk.ids.len();
        return vec![Unit::Speech(request(chunk.ids, n), finish(None))];
    }
    let plan = Arc::new(stitch::Plan::new(chunk.ids));
    (0..plan.pieces())
        .map(|i| {
            let ids = plan.context_ids(i).to_vec();
            Unit::Speech(request(ids, plan.style_len().into()), finish(Some((plan.clone(), i))))
        })
        .collect()
}

fn produce(pipeline: &dyn Pipeline, speech: &Speech, out: &mpsc::Sender<Produced>) {
    for chunk in pipeline.chunks(speech.lang, &speech.text) {
        let units = match chunk {
            Ok(chunk) => units(chunk, speech),
            Err(PipelineError::Unsupported(why)) => {
                crate::warn!("skipping a sentence: {why}");
                continue;
            }
            Err(e) => {
                let _ = out.blocking_send(Produced::Failed(e));
                return;
            }
        };
        for unit in units {
            if out.blocking_send(Produced::Unit(unit)).is_err() {
                return;
            }
        }
    }
    let _ = out.blocking_send(Produced::End);
}

enum Output {
    Audio(Bytes),
    Failed(StreamError),
    End,
}

/// 16-bit little-endian PCM that stays counted in the server's audio gauge until the bytes are
/// dropped, which for a response body is once hyper has written them.
struct Pcm {
    bytes: Vec<u8>,
    _held: HeldAudio,
}

impl AsRef<[u8]> for Pcm {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

fn pcm_bytes(samples: &[i16], held: HeldAudio) -> Bytes {
    let mut bytes = Vec::with_capacity(2 * samples.len());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    Bytes::from_owner(Pcm { bytes, _held: held })
}

/// A request's PCM, in order. It holds the request's admission permits, so they're released only
/// when the response body is finished or dropped. Dropping it stops the request and cancels its
/// queued chunks.
pub struct AudioStream {
    rx: mpsc::Receiver<Output>,
    over: bool,
    _permits: Box<dyn Send + Sync>,
}

impl AudioStream {
    /// The next piece of audio; `None` once the request is complete. After an error, nothing more.
    pub async fn next(&mut self) -> Option<Result<Bytes, StreamError>> {
        if self.over {
            return None;
        }
        let item = match self.rx.recv().await {
            Some(Output::Audio(bytes)) => return Some(Ok(bytes)),
            Some(Output::End) => None,
            Some(Output::Failed(e)) => Some(Err(e)),
            None => Some(Err(StreamError::Aborted)),
        };
        self.over = true;
        item
    }
}

enum Done {
    Silence(usize),
    Speech(Result<Synthesized, SynthError>, Finish),
}

type Pending = Pin<Box<dyn Future<Output = Done> + Send>>;

/// Starts producing a request's audio. `permits` are held by the returned stream.
pub fn start(
    speech: Speech,
    connection: Arc<Connection>,
    pipeline: Arc<dyn Pipeline>,
    lookahead: usize,
    permits: impl Send + Sync + 'static,
) -> AudioStream {
    let (out_tx, out_rx) = mpsc::channel(1);
    let (unit_tx, unit_rx) = mpsc::channel(1);
    tokio::task::spawn_blocking(move || produce(pipeline.as_ref(), &speech, &unit_tx));
    tokio::spawn(async move {
        let result = drive(connection, unit_rx, &out_tx, lookahead).await;
        let _ = out_tx.send(result.map_or_else(Output::Failed, |()| Output::End)).await;
    });
    AudioStream { rx: out_rx, over: false, _permits: Box::new(permits) }
}

/// The client went away; stop quietly.
struct ClientGone;

/// Waits for `work` unless the client leaves first.
async fn unless_gone<T>(out: &mpsc::Sender<Output>, work: impl Future<Output = T>) -> Result<T, ClientGone> {
    tokio::select! {
        _ = out.closed() => Err(ClientGone),
        value = work => Ok(value),
    }
}

async fn drive(
    connection: Arc<Connection>,
    mut units: mpsc::Receiver<Produced>,
    out: &mpsc::Sender<Output>,
    lookahead: usize,
) -> Result<(), StreamError> {
    let mut window: FuturesOrdered<Pending> = FuturesOrdered::new();
    let mut speech_in_window = 0;
    let mut exhausted = false;
    let mut stitcher = Stitcher::default();
    let send = |bytes: Bytes| async move {
        if bytes.is_empty() {
            return Ok(());
        }
        out.send(Output::Audio(bytes)).await.map_err(|_| ClientGone)
    };
    loop {
        if exhausted && window.is_empty() {
            return Ok(());
        }
        let step: Result<(), ClientGone> = tokio::select! {
            biased;
            _ = out.closed() => Err(ClientGone),
            done = window.next(), if !window.is_empty() => match done.expect("window isn't empty") {
                Done::Silence(n) => send(Bytes::from_owner(Pcm { bytes: vec![0; 2 * n], _held: connection.gauge().hold(2 * n) })).await,
                Done::Speech(result, finish) => {
                    speech_in_window -= 1;
                    let synth = result.map_err(StreamError::Synth)?;
                    let pcm = match &finish.stitch {
                        Some((plan, index)) => stitcher.piece(plan, *index, synth.pcm, &synth.durations),
                        None => synth.pcm,
                    };

                    let range = trim_range(&pcm, &finish.text, finish.speed);
                    send(pcm_bytes(&pcm[range], synth.held)).await
                }
            },
            produced = units.recv(), if !exhausted && window.len() < lookahead => {
                match produced.ok_or(StreamError::Aborted)? {
                    Produced::End => {
                        exhausted = true;
                        Ok(())
                    }
                    Produced::Failed(e) => return Err(StreamError::Pipeline(e)),
                    Produced::Unit(Unit::Silence(n)) => {
                        window.push_back(Box::pin(async move { Done::Silence(n) }));
                        Ok(())
                    }
                    Produced::Unit(Unit::Speech(request, finish)) => {
                        let priority = if speech_in_window == 0 { Priority::Waiting } else { Priority::Prefetch };
                        match unless_gone(out, connection.submit(priority, &request)).await {
                            Ok(ticket) => {
                                let mut ticket = ticket.map_err(StreamError::Synth)?;
                                speech_in_window += 1;
                                window.push_back(Box::pin(async move { Done::Speech(ticket.result().await, finish) }));
                                Ok(())
                            }
                            Err(gone) => Err(gone),
                        }
                    }
                }
            }
        };
        if step.is_err() {
            return Ok(());
        }
    }
}
