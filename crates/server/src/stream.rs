//! One speech request from text to PCM: phonemize chunks in order on a blocking thread, keep a
//! few of them on the worker, and send the audio back in order as soon as the next one is ready.
//!
//! Every chunk goes to the worker at the request's priority level. The chunk the client is
//! waiting for is marked waiting, later ones prefetch, and at most `lookahead` chunks are ahead of
//! the client at a time, so one long request can't fill the worker's queue. Phonemizing stops when the window is full, and everything stops when
//! the client goes away: dropping a queued chunk's ticket cancels it on the worker.
//!
//! Both stages mark their end explicitly, so a stage that dies (a panic) fails the response
//! instead of ending it early as if it were complete.
//!
//! The request's admission permits are shared by the response body and the phonemizing thread,
//! and released only when both are done: a client that disconnects while a long sentence is being
//! phonemized still counts against the limits until that work stops. A request may send at most
//! [`Limits::max_ids`] phoneme ids to the worker in all.
//!
//! Until its first audio, a request may be shed (`crate::load`): the stream then fails with
//! [`StreamError::Shed`] and its queued chunks are cancelled. Once the first audio is handed out
//! it never is.

use crate::audio::{Joins, final_clause_mark, keep_range, silence};
use crate::load::{Admission, Shed};
use crate::pack::{SOFT_MAX_PHONEMES, pack};
use crate::pipeline::{Chunk, DEFAULT_MAX_TOKENS, Pipeline, PipelineError};
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
    /// The text needs more phoneme ids than one request may send to the worker.
    TooMuchSpeech {
        max_ids: usize,
    },
    /// A stage stopped without finishing or reporting an error.
    Aborted,
    /// Shed before any audio, to keep first-audio latency within budget.
    Shed(Shed),
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pipeline(e) => write!(f, "text processing failed: {e}"),
            Self::Synth(e) => e.fmt(f),
            Self::TooMuchSpeech { max_ids } => {
                write!(f, "the input makes more than {max_ids} phonemes of speech; send it as several requests")
            }
            Self::Aborted => f.write_str("speech generation stopped unexpectedly"),
            Self::Shed(shed) if shed.estimate.is_zero() => f.write_str(
                "the server is at its request limit and a more urgent request took this one's place; use another provider for this request",
            ),
            Self::Shed(shed) => write!(
                f,
                "the server is overloaded: estimated time to first audio {} ms; use another provider for this request",
                shed.estimate.as_millis()
            ),
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
    /// Where it meets the chunks its sentence was split into at clause punctuation, if any.
    joins: Joins,
}

#[derive(Debug)]
enum Unit {
    Silence(usize),
    Speech(SynthRequest, Finish),
}

enum Produced {
    Unit(Unit),
    Failed(StreamError),
    End,
}

/// The most phonemes a sentence may have before the chunker cuts it at clause punctuation: the
/// reference's 450, or fewer if one worker request may have fewer ids (`render_ids`, from
/// [`stitch::max_render_ids`]). An id is a phoneme the vocabulary has, so a chunk within this many
/// phonemes is rendered whole; only a clause longer than this by itself is stitched.
pub fn max_chunk_phonemes(render_ids: usize) -> usize {
    render_ids.min(DEFAULT_MAX_TOKENS)
}

/// A chunk's units. `render_ids` is the most ids one worker request may have
/// ([`stitch::max_render_ids`]): a chunk with more, or with more than `STITCH_ABOVE` phonemes, is
/// stitched. Where the chunker cut the chunk's sentence, the first unit starts and the last ends at
/// the cut ([`Joins`]).
fn units(chunk: Chunk, speech: &Speech, render_ids: usize) -> Vec<Unit> {
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
    let split = chunk.split;
    // `own` is where the piece's own ids start in the ids it is rendered with: a clause mark
    // before it is in the context, whose audio the stitcher drops, so the end is trimmed instead.
    let finish = |stitch, ids: &[u8], own: usize, first: bool, last: bool| Finish {
        text: text.clone(),
        speed: speech.speed.as_f64(),
        stitch,
        joins: Joins {
            start_at_speech: first && split.before,
            end_after_id: if last && split.after { final_clause_mark(&ids[own..]).map(|i| own + i) } else { None },
        },
    };
    if chunk.phonemes.chars().count() <= STITCH_ABOVE && chunk.ids.len() <= render_ids {
        let n = chunk.ids.len();
        let finish = finish(None, &chunk.ids, 0, true, true);
        return vec![Unit::Speech(request(chunk.ids, n), finish)];
    }
    let plan = Arc::new(stitch::Plan::new(chunk.ids, render_ids));
    (0..plan.pieces())
        .map(|i| {
            let ids = plan.context_ids(i).to_vec();
            let finish = finish(Some((plan.clone(), i)), &ids, plan.own_start(i), i == 0, i + 1 == plan.pieces());
            Unit::Speech(request(ids, plan.style_len().into()), finish)
        })
        .collect()
}

/// Phonemizes the request and sends its units (packed, [`pack`]), then how it ended. Runs of text with no G2P yet
/// are logged once per request, as a count, since the text itself mustn't reach the logs.
/// `max_chunk_samples` is the worker's chunk cap.
fn produce(
    pipeline: &dyn Pipeline,
    speech: &Speech,
    max_ids: usize,
    max_chunk_samples: u32,
    out: &mpsc::Sender<Produced>,
) {
    let mut unread = 0usize;
    let end = produce_units(pipeline, speech, max_ids, max_chunk_samples, out, &mut unread);
    if unread > 0 {
        crate::warn!("read {unread} runs of text with no G2P for their language or script as their name");
    }
    if let Some(end) = end {
        let _ = out.blocking_send(end);
    }
}

/// How the request ended, or `None` if the stream is gone.
fn produce_units(
    pipeline: &dyn Pipeline,
    speech: &Speech,
    max_ids: usize,
    max_chunk_samples: u32,
    out: &mpsc::Sender<Produced>,
    unread: &mut usize,
) -> Option<Produced> {
    let render_ids = stitch::max_render_ids(max_chunk_samples, speech.speed);
    let mut ids = 0usize;
    let max_phonemes = max_chunk_phonemes(render_ids);
    // Once nobody is listening, text processing stops at its next G2P call.
    let gone = || out.is_closed();
    let chunks = pipeline.chunks(speech.lang, &speech.text, max_phonemes, SOFT_MAX_PHONEMES, &gone);
    for chunk in pack(chunks, SOFT_MAX_PHONEMES, max_phonemes) {
        let units = match chunk {
            Ok(chunk) => {
                *unread += chunk.unread;
                units(chunk, speech, render_ids)
            }
            Err(e) => return Some(Produced::Failed(StreamError::Pipeline(e))),
        };
        // What the worker renders, stitched pieces' overlapping context included.
        let chunk_ids: usize =
            units.iter().map(|unit| if let Unit::Speech(request, _) = unit { request.ids.len() } else { 0 }).sum();
        ids = ids.saturating_add(chunk_ids);
        if ids > max_ids {
            return Some(Produced::Failed(StreamError::TooMuchSpeech { max_ids }));
        }
        for unit in units {
            out.blocking_send(Produced::Unit(unit)).ok()?;
        }
    }
    Some(Produced::End)
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

/// A request's PCM, in order. It shares the request's admission permits with the phonemizing
/// thread, so they're released only when the response body is finished or dropped *and* that
/// thread has stopped. Dropping it stops the request and cancels its queued chunks.
pub struct AudioStream {
    rx: mpsc::Receiver<Output>,
    over: bool,
    /// Whether audio has been handed out, after which the request can't be shed.
    started: bool,
    admission: Arc<Admission>,
    _permits: Arc<dyn Send + Sync>,
}

impl AudioStream {
    /// The next piece of audio; `None` once the request is complete. After an error, nothing more.
    /// Before the first audio, a shed request gets [`StreamError::Shed`] at once.
    pub async fn next(&mut self) -> Option<Result<Bytes, StreamError>> {
        if self.over {
            return None;
        }
        let received = if self.started {
            self.rx.recv().await
        } else {
            tokio::select! {
                biased;
                received = self.rx.recv() => received,
                () = self.admission.shed() => None,
            }
        };
        let item = match received {
            Some(Output::Audio(bytes)) if self.started || self.admission.start() => {
                self.started = true;
                return Some(Ok(bytes));
            }
            _ if !self.started && self.admission.is_shed() => {
                Some(Err(StreamError::Shed(Shed { estimate: self.admission.shed_estimate() })))
            }
            Some(Output::End) => None,
            Some(Output::Failed(e)) => Some(Err(e)),
            // Audio here would mean `start` failed without the request being shed, which it can't.
            Some(Output::Audio(_)) | None => Some(Err(StreamError::Aborted)),
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

/// How much of the worker one request may use.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Chunks on the worker or waiting for the client, ahead of what the client has received.
    pub lookahead: usize,
    /// Phoneme ids the request may send to the worker in all.
    pub max_ids: usize,
}

/// Starts producing a request's audio. `permits` are held until both the returned stream and the
/// phonemizing thread are done; `admission` places the request's chunks in the latency estimate.
pub fn start(
    speech: Speech,
    connection: Arc<Connection>,
    pipeline: Arc<dyn Pipeline>,
    limits: Limits,
    admission: Arc<Admission>,
    permits: impl Send + Sync + 'static,
) -> AudioStream {
    let (out_tx, out_rx) = mpsc::channel(1);
    let (unit_tx, unit_rx) = mpsc::channel(1);
    let permits: Arc<dyn Send + Sync> = Arc::new(permits);
    let producer_permits = permits.clone();
    let max_chunk_samples = connection.max_chunk_samples();
    tokio::task::spawn_blocking(move || {
        produce(pipeline.as_ref(), &speech, limits.max_ids, max_chunk_samples, &unit_tx);
        drop(producer_permits);
    });
    let drive_admission = admission.clone();
    tokio::spawn(async move {
        let result = drive(connection, &drive_admission, unit_rx, &out_tx, limits.lookahead).await;
        let _ = out_tx.send(result.map_or_else(Output::Failed, |()| Output::End)).await;
    });
    AudioStream { rx: out_rx, over: false, started: false, admission, _permits: permits }
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
    admission: &Admission,
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
                    let cuts = finish.joins.cuts(&synth.durations, synth.pcm.len());
                    let pcm = match &finish.stitch {
                        Some((plan, index)) => stitcher.piece(plan, *index, synth.pcm, &synth.durations),
                        None => synth.pcm,
                    };
                    let range = keep_range(&pcm, &finish.text, finish.speed, cuts);
                    send(pcm_bytes(&pcm[range], synth.held)).await
                }
            },
            produced = units.recv(), if !exhausted && window.len() < lookahead => {
                match produced.ok_or(StreamError::Aborted)? {
                    Produced::End => {
                        exhausted = true;
                        Ok(())
                    }
                    Produced::Failed(e) => return Err(e),
                    Produced::Unit(Unit::Silence(n)) => {
                        window.push_back(Box::pin(async move { Done::Silence(n) }));
                        Ok(())
                    }
                    Produced::Unit(Unit::Speech(request, finish)) => {
                        let work = admission.work(speech_in_window == 0, request.ids.len());
                        match unless_gone(out, connection.submit(&request, work)).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{Lang, Split};
    use crate::protocol::Speed;

    /// `n` chunks, each holding two runs read as their name.
    struct AllUnread(usize);

    impl Pipeline for AllUnread {
        fn chunks<'a>(
            &'a self,
            _: Lang,
            text: &'a str,
            _: usize,
            _: usize,
            _: &'a (dyn Fn() -> bool + Sync),
        ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
            Box::new((0..self.0).map(move |_| {
                Ok(Chunk {
                    text: text.into(),
                    phonemes: String::new(),
                    ids: Vec::new(),
                    pause: None,
                    unread: 2,
                    split: Split::default(),
                })
            }))
        }
    }

    #[test]
    fn the_chunk_limit_is_the_references_or_the_workers() {
        assert_eq!(max_chunk_phonemes(510), 450);
        assert_eq!(max_chunk_phonemes(450), 450);
        assert_eq!(max_chunk_phonemes(270), 270);
        assert_eq!(max_chunk_phonemes(0), 0);
    }

    /// A clause too long by itself is stitched; only its first piece starts and its last piece
    /// ends at the clause cuts.
    #[test]
    fn a_stitched_clause_joins_its_neighbours_at_its_outer_pieces() {
        let speech = Speech { lang: Lang::AmericanEnglish, text: String::new(), voice: 0, speed: Speed::NORMAL };
        let mut ids: Vec<u8> = (0..300).map(|i| if i % 6 == 5 { stitch::SPACE_ID } else { 43 }).collect();
        ids.push(3);
        let joins = |split| -> Vec<Joins> {
            let chunk = Chunk {
                text: "words,".into(),
                phonemes: "x".repeat(ids.len()),
                ids: ids.clone(),
                pause: None,
                unread: 0,
                split,
            };
            units(chunk, &speech, 120)
                .into_iter()
                .map(|unit| match unit {
                    Unit::Speech(_, finish) => finish.joins,
                    Unit::Silence(_) => unreachable!(),
                })
                .collect()
        };
        let both = joins(Split { before: true, after: true });
        assert!(both.len() >= 3, "{} pieces", both.len());
        let plan = stitch::Plan::new(ids.clone(), 120);
        let last = plan.context_ids(plan.pieces() - 1).len() - 1;
        assert_eq!(both[0], Joins { start_at_speech: true, end_after_id: None });
        assert!(both[1..both.len() - 1].iter().all(|j| *j == Joins::default()));
        assert_eq!(both[both.len() - 1], Joins { start_at_speech: false, end_after_id: Some(last) });
        assert!(joins(Split::default()).iter().all(|j| *j == Joins::default()));
    }

    #[test]
    fn unread_runs_are_logged_once_without_their_text() {
        let speech = Speech { lang: Lang::AmericanEnglish, text: "SECRET".into(), voice: 0, speed: Speed::NORMAL };
        let (tx, mut rx) = mpsc::channel(1);
        let lines = crate::log::capture(|| produce(&AllUnread(3333), &speech, 60_000, 1_440_000, &tx));
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("6666") && !lines[0].contains("SECRET"), "{}", lines[0]);
        assert!(matches!(rx.try_recv(), Ok(Produced::End)));
    }
}
