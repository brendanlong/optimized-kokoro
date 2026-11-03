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

use crate::audio::{apply_volume, silence, to_le_bytes, trim_range};
use crate::pipeline::{Chunk, Pipeline, PipelineError};
use crate::protocol::Priority;
use crate::request::{Segment, Speech};
use crate::stitch::{self, STITCH_ABOVE, Stitcher};
use crate::worker::{Connection, SynthError, SynthRequest, Synthesized};
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::FuturesOrdered;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, mpsc};

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

fn units(chunk: Chunk, segment: &Segment) -> Vec<Unit> {
    if let Some(pause) = chunk.pause {
        return if pause > 0.0 { vec![Unit::Silence(silence(pause))] } else { Vec::new() };
    }
    if chunk.ids.is_empty() {
        return Vec::new();
    }
    let text: Arc<str> = chunk.text.into();
    let request = |ids: Vec<u8>| SynthRequest { speed: segment.speed as f32, voices: segment.voices.clone(), ids };
    let finish = |stitch| Finish { text: text.clone(), speed: segment.speed, stitch };
    if chunk.phonemes.chars().count() <= STITCH_ABOVE {
        return vec![Unit::Speech(request(chunk.ids), finish(None))];
    }
    let plan = Arc::new(stitch::Plan::new(chunk.ids));
    (0..plan.pieces())
        .map(|i| Unit::Speech(request(plan.context_ids(i).to_vec()), finish(Some((plan.clone(), i)))))
        .collect()
}

fn produce(pipeline: &dyn Pipeline, speech: &Speech, out: &mpsc::Sender<Produced>) {
    for segment in &speech.segments {
        for chunk in pipeline.chunks(&segment.lang, &segment.text, &speech.options) {
            let units = match chunk {
                Ok(chunk) => units(chunk, segment),
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
    }
    let _ = out.blocking_send(Produced::End);
}

enum Output {
    Audio(Bytes),
    Failed(StreamError),
    End,
}

/// A request's PCM, in order. Dropping it stops the request and cancels its queued chunks.
pub struct AudioStream {
    rx: mpsc::Receiver<Output>,
    over: bool,
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

/// Starts producing a request's audio. `permit` is held until the request ends.
pub fn start(
    speech: Speech,
    connection: Arc<Connection>,
    pipeline: Arc<dyn Pipeline>,
    lookahead: usize,
    permit: OwnedSemaphorePermit,
) -> AudioStream {
    let (out_tx, out_rx) = mpsc::channel(1);
    let (unit_tx, unit_rx) = mpsc::channel(1);
    let volume = speech.volume;
    tokio::task::spawn_blocking(move || produce(pipeline.as_ref(), &speech, &unit_tx));
    tokio::spawn(async move {
        let result = drive(connection, unit_rx, &out_tx, volume, lookahead).await;
        let _ = out_tx.send(result.map_or_else(Output::Failed, |()| Output::End)).await;
        drop(permit);
    });
    AudioStream { rx: out_rx, over: false }
}

struct ClientGone;

async fn drive(
    connection: Arc<Connection>,
    mut units: mpsc::Receiver<Produced>,
    out: &mpsc::Sender<Output>,
    volume: f64,
    lookahead: usize,
) -> Result<(), StreamError> {
    let mut window: FuturesOrdered<Pending> = FuturesOrdered::new();
    let mut speech_in_window = 0;
    let mut exhausted = false;
    let mut stitcher = Stitcher::default();
    let send = |pcm: Vec<i16>| async move {
        if pcm.is_empty() {
            return Ok(());
        }
        out.send(Output::Audio(Bytes::from(to_le_bytes(&pcm)))).await.map_err(|_| ClientGone)
    };
    loop {
        if exhausted && window.is_empty() {
            return Ok(());
        }
        let sent = tokio::select! {
            biased;
            _ = out.closed() => Err(ClientGone),
            done = window.next(), if !window.is_empty() => match done.expect("window isn't empty") {
                Done::Silence(n) => send(vec![0; n]).await,
                Done::Speech(result, finish) => {
                    speech_in_window -= 1;
                    let synth = result.map_err(StreamError::Synth)?;
                    let mut pcm = match &finish.stitch {
                        Some((plan, index)) => stitcher.piece(plan, *index, &synth.pcm, &synth.durations),
                        None => synth.pcm,
                    };
                    apply_volume(&mut pcm, volume);
                    let range = trim_range(&pcm, &finish.text, finish.speed);
                    pcm.truncate(range.end);
                    pcm.drain(..range.start);
                    send(pcm).await
                }
            },
            produced = units.recv(), if !exhausted && window.len() < lookahead => {
                match produced.ok_or(StreamError::Aborted)? {
                    Produced::End => exhausted = true,
                    Produced::Failed(e) => return Err(StreamError::Pipeline(e)),
                    Produced::Unit(Unit::Silence(n)) => window.push_back(Box::pin(async move { Done::Silence(n) })),
                    Produced::Unit(Unit::Speech(request, finish)) => {
                        let priority = if speech_in_window == 0 { Priority::Waiting } else { Priority::Prefetch };
                        let mut ticket = connection.submit(priority, &request).map_err(StreamError::Synth)?;
                        speech_in_window += 1;
                        window.push_back(Box::pin(async move { Done::Speech(ticket.result().await, finish) }));
                    }
                }
                Ok(())
            }
        };
        if sent.is_err() {
            return Ok(());
        }
    }
}
