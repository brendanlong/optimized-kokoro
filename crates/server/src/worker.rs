//! The front's side of the worker connection: one Unix socket, multiplexed across requests,
//! re-established whenever the worker goes away.
//!
//! Every frame the worker sends is checked against what the front asked for. Anything unexpected
//! closes the connection, and every request in flight on it fails; nothing is resubmitted.
//!
//! Requests live in slots (`docs/worker-protocol.md`): a fixed table, as large as the worker's
//! Hello and the configured limit allow. A request takes a free slot when it's submitted and
//! gives it back only when the worker's Result or Failed for that slot arrives, so cancelled
//! requests the worker hasn't acknowledged still hold theirs, and the outgoing frame queue is
//! bounded: at most three frames per slot (a stale Cancel for its previous request, which the
//! worker ignores, then a Synthesize and a Cancel).
//!
//! A request's audio is at most the worker's chunk cap at its speed (durations implying more are
//! a violation), so memory is bounded by how many chunks can be in flight, which the request and
//! lookahead limits fix. An [`AudioGauge`] counts what's held.
//!
//! Each slot also holds its chunk's [`Work`], which counts the chunk in its request's latency
//! estimate until the reply arrives, and learns the cost per id from each Result
//! (`crate::load`).
//!
//! A worker that doesn't say hello in time, makes no progress while it has work, or leaves any
//! one slot outstanding for too long (while answering others), is disconnected.

use crate::load::Work;
use crate::protocol::{
    self, Expected, FailCode, Hello, ReadError, Speed, Synthesize, WorkerFrame, encode_cancel, encode_synthesize,
};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::OwnedReadHalf;
use tokio::sync::{Notify, Semaphore, mpsc, oneshot, watch};
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthError {
    /// The connection closed (worker restart, protocol violation or stall) before the result came.
    ConnectionLost,
    /// The worker reported an internal error for this request.
    WorkerFailed,
    /// The text pipeline produced a chunk outside the worker's limits, or the worker found it too long.
    Unrenderable,
}

impl fmt::Display for SynthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConnectionLost => f.write_str("lost the connection to the model worker"),
            Self::WorkerFailed => f.write_str("the model worker failed to synthesize a chunk"),
            Self::Unrenderable => f.write_str("a chunk is outside the model worker's limits"),
        }
    }
}

impl std::error::Error for SynthError {}

/// Audio bytes in memory, counted while a [`HeldAudio`] for them exists. A gauge for tests and
/// monitoring, not a limit: what bounds memory is the number of chunks in flight (see
/// `docs/worker-protocol.md` and [`crate::config::Config::audio_bound`]).
#[derive(Debug, Clone, Default)]
pub struct AudioGauge(Arc<AtomicUsize>);

impl AudioGauge {
    pub fn held(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }

    pub fn hold(&self, bytes: usize) -> HeldAudio {
        self.0.fetch_add(bytes, Ordering::SeqCst);
        HeldAudio { bytes, gauge: self.0.clone() }
    }
}

/// Counts `bytes` in an [`AudioGauge`] until dropped; it travels with the audio it covers.
#[derive(Debug)]
pub struct HeldAudio {
    bytes: usize,
    gauge: Arc<AtomicUsize>,
}

impl Drop for HeldAudio {
    fn drop(&mut self) {
        self.gauge.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct Synthesized {
    pub pcm: Vec<i16>,
    /// Predicted frames per id, with the two boundary ids at either end.
    pub durations: Vec<u16>,
    /// Counts `pcm` in the server's gauge; keep it with the audio until the audio is gone.
    pub held: HeldAudio,
}

/// What the scheduler asks the worker to render.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub speed: Speed,
    pub voice: u16,
    pub ids: Vec<u8>,
    /// The style row to use, plus one: `ids.len()`, or the whole clause's length for a piece of a
    /// stitched chunk.
    pub style_len: u16,
}

type Reply = oneshot::Sender<Result<Synthesized, SynthError>>;

/// A busy slot.
struct Pending {
    /// Which submission holds the slot, so a late cancel from an earlier holder can't touch it.
    ticket: u64,
    expected: Expected,
    /// `None` once cancelled: the worker still owes a Result or Failed for the slot.
    reply: Option<Reply>,
    /// When the Synthesize was queued.
    submitted: Instant,
    /// The chunk in its request's estimate.
    work: Work,
}

struct State {
    open: bool,
    slots: Vec<Option<Pending>>,
    free: Vec<u16>,
    next_ticket: u64,
    /// When the worker last finished a frame, or was last given work while idle.
    last_progress: Instant,
    /// When the worker last sent a Result, or was last given work while idle: a Result's busy
    /// time is measured from here.
    busy_since: Instant,
}

impl State {
    fn busy(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    fn pending(&mut self, slot: u16) -> Option<&mut Pending> {
        self.slots.get_mut(usize::from(slot))?.as_mut()
    }
}

/// Limits on the worker and how long it may take.
#[derive(Debug, Clone)]
pub struct Settings {
    pub socket: PathBuf,
    pub retry_interval: Duration,
    pub hello_timeout: Duration,
    /// Longest the worker may go without sending a whole frame while it has work.
    pub progress_timeout: Duration,
    /// The most slots the front uses, whatever the worker announces.
    pub slots: u16,
    /// The largest `max_chunk_samples` a worker may announce; a hello above it is refused, since
    /// the cap is what bounds the audio the server holds.
    pub max_chunk_samples: u32,
    /// Longest one slot may stay outstanding after its Synthesize, queueing included.
    pub slot_timeout: Duration,
}

pub struct Connection {
    hello: Hello,
    voice_index: HashMap<String, u16>,
    outgoing: mpsc::Sender<Vec<u8>>,
    /// One permit per free slot, so submitting waits for one.
    free_slots: Semaphore,
    gauge: AudioGauge,
    state: Mutex<State>,
    reset: Notify,
}

impl Connection {
    /// A connection using the first `slots` of the worker's slots (at most `hello.slots`).
    pub(crate) fn new(hello: Hello, outgoing: mpsc::Sender<Vec<u8>>, slots: u16, gauge: AudioGauge) -> Self {
        let slots = slots.min(hello.slots);
        let voice_index = hello.voices.iter().enumerate().map(|(i, v)| (v.clone(), i as u16)).collect();
        let state = State {
            open: true,
            slots: (0..slots).map(|_| None).collect(),
            free: (0..slots).rev().collect(),
            next_ticket: 0,
            last_progress: Instant::now(),
            busy_since: Instant::now(),
        };
        Self {
            hello,
            voice_index,
            outgoing,
            free_slots: Semaphore::new(slots.into()),
            gauge,
            state: Mutex::new(state),
            reset: Notify::new(),
        }
    }

    pub fn voices(&self) -> &[String] {
        &self.hello.voices
    }

    pub fn voice_index(&self, name: &str) -> Option<u16> {
        self.voice_index.get(name).copied()
    }

    pub fn is_open(&self) -> bool {
        self.state.lock().unwrap().open
    }

    pub fn gauge(&self) -> &AudioGauge {
        &self.gauge
    }

    /// The slots the front uses on this connection.
    /// The most requests the worker renders at once.
    pub fn max_batch(&self) -> u16 {
        self.hello.max_batch
    }

    pub fn slots(&self) -> usize {
        self.state.lock().unwrap().slots.len()
    }

    /// Slots holding a request the worker hasn't finished, cancelled ones included.
    pub fn busy_slots(&self) -> usize {
        self.state.lock().unwrap().busy()
    }

    fn valid(&self, r: &SynthRequest) -> bool {
        !r.ids.is_empty()
            && r.ids.len() <= usize::from(self.hello.max_phonemes)
            && (1..=self.hello.max_phonemes).contains(&r.style_len)
            && r.ids.iter().all(|&id| id >= 1 && u16::from(id) < self.hello.vocab_size)
            && usize::from(r.voice) < self.hello.voices.len()
    }

    /// Queues a request on the worker at `work`'s priority, first waiting for a free slot.
    /// Dropping the future while it waits takes no slot. `work` stays counted until the reply.
    pub async fn submit(self: &Arc<Self>, request: &SynthRequest, work: Work) -> Result<Ticket, SynthError> {
        if !self.valid(request) {
            return Err(SynthError::Unrenderable);
        }
        self.free_slots.acquire().await.map_err(|_| SynthError::ConnectionLost)?.forget();
        let (tx, rx) = oneshot::channel();
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(SynthError::ConnectionLost);
        }
        let slot = state.free.pop().expect("a permit means a free slot");
        let frame = encode_synthesize(&Synthesize {
            slot,
            priority: work.priority(),
            speed: request.speed,
            voice: request.voice,
            ids: &request.ids,
            style_len: request.style_len,
        });
        if self.outgoing.try_send(frame).is_err() {
            drop(state);
            self.close();
            return Err(SynthError::ConnectionLost);
        }
        if state.busy() == 1 {
            state.last_progress = Instant::now();
            state.busy_since = Instant::now();
        }
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        let expected = Expected {
            n_ids: request.ids.len() as u16,
            speed: request.speed,
            max_chunk_samples: self.hello.max_chunk_samples,
        };
        state.slots[usize::from(slot)] =
            Some(Pending { ticket, expected, reply: Some(tx), submitted: Instant::now(), work });
        drop(state);
        Ok(Ticket { slot, ticket, connection: self.clone(), reply: rx, finished: false })
    }

    fn cancel(&self, slot: u16, ticket: u64) {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return;
        }
        if let Some(pending) = state.pending(slot)
            && pending.ticket == ticket
            && pending.reply.take().is_some()
            && self.outgoing.try_send(encode_cancel(slot)).is_err()
        {
            drop(state);
            self.close();
        }
    }

    fn expected(&self, slot: u16) -> Option<Expected> {
        self.state.lock().unwrap().pending(slot).map(|p| p.expected)
    }

    /// Ends the request in `slot` and frees the slot.
    fn finish(&self, state: &mut State, slot: u16) -> Result<Pending, String> {
        let pending = state
            .slots
            .get_mut(usize::from(slot))
            .and_then(Option::take)
            .ok_or(format!("a reply for slot {slot}, which is free"))?;
        state.free.push(slot);
        self.free_slots.add_permits(1);
        Ok(pending)
    }

    fn dispatch(&self, frame: WorkerFrame) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.last_progress = Instant::now();
        match frame {
            WorkerFrame::Hello(_) => Err("second hello".into()),
            WorkerFrame::Result { slot, durations, pcm } => {
                let pending = self.finish(&mut state, slot)?;
                let now = Instant::now();
                pending.work.rendered(now.saturating_duration_since(state.busy_since));
                state.busy_since = now;
                if let Some(reply) = pending.reply {
                    let held = self.gauge.hold(2 * pcm.len());
                    let _ = reply.send(Ok(Synthesized { pcm, durations, held }));
                }
                Ok(())
            }
            WorkerFrame::Failed { slot, code } => {
                let cancelled = state.pending(slot).is_some_and(|p| p.reply.is_none());
                if code == FailCode::Cancelled && !cancelled {
                    return Err(format!("slot {slot} reported cancelled, but it wasn't"));
                }
                // A cancelled request never ran; one that failed did, for a time nobody should be
                // charged for, so the next Result's busy time starts now.
                if code != FailCode::Cancelled {
                    state.busy_since = Instant::now();
                }
                if let Some(reply) = self.finish(&mut state, slot)?.reply {
                    let error = match code {
                        FailCode::TooLong => SynthError::Unrenderable,
                        FailCode::Cancelled | FailCode::Internal => SynthError::WorkerFailed,
                    };
                    let _ = reply.send(Err(error));
                }
                Ok(())
            }
        }
    }

    /// Why the worker should be dropped, if it should: it has had work and sent no whole frame
    /// for `progress_timeout`, or one slot has been outstanding for `slot_timeout`.
    fn overdue(&self, progress_timeout: Duration, slot_timeout: Duration) -> Option<String> {
        let state = self.state.lock().unwrap();
        if state.busy() > 0 && state.last_progress.elapsed() >= progress_timeout {
            return Some(format!("no progress for {progress_timeout:?} with work outstanding"));
        }
        let oldest = state
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((i, p.as_ref()?.submitted)))
            .min_by_key(|(_, t)| *t);
        match oldest {
            Some((slot, submitted)) if submitted.elapsed() >= slot_timeout => {
                Some(format!("slot {slot} outstanding for {slot_timeout:?}"))
            }
            _ => None,
        }
    }

    /// Fails everything in flight and tells the connection task to hang up.
    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.open = false;
        self.free_slots.close();
        for pending in state.slots.iter_mut().filter_map(Option::take) {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Err(SynthError::ConnectionLost));
            }
        }
        state.free = (0..state.slots.len() as u16).rev().collect();
        self.reset.notify_one();
    }
}

/// A submitted request. Dropping it before its result arrives cancels it on the worker.
pub struct Ticket {
    slot: u16,
    ticket: u64,
    connection: Arc<Connection>,
    reply: oneshot::Receiver<Result<Synthesized, SynthError>>,
    finished: bool,
}

impl Ticket {
    pub async fn result(&mut self) -> Result<Synthesized, SynthError> {
        let result = (&mut self.reply).await.unwrap_or(Err(SynthError::ConnectionLost));
        self.finished = true;
        result
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if !self.finished {
            self.connection.cancel(self.slot, self.ticket);
        }
    }
}

/// The current worker connection, kept up by a background task.
#[derive(Clone)]
pub struct Worker {
    current: watch::Receiver<Option<Arc<Connection>>>,
    gauge: AudioGauge,
}

impl Worker {
    /// Connects to the worker's socket, and again `retry_interval` after every disconnect or
    /// failed attempt, for as long as the returned handle (or a clone) exists.
    pub fn spawn(settings: Settings) -> Self {
        let (tx, rx) = watch::channel(None);
        let gauge = AudioGauge::default();
        let task_gauge = gauge.clone();
        tokio::spawn(async move {
            // A worker that keeps failing would log a line per reconnect attempt.
            let mut warnings = crate::log::RateLimit::new(Duration::from_secs(60));
            loop {
                match UnixStream::connect(&settings.socket).await {
                    Ok(stream) => {
                        if let Err(e) = serve(stream, &tx, &settings, &task_gauge).await
                            && let Some(suppressed) = warnings.allow()
                        {
                            match suppressed {
                                0 => crate::warn!("worker connection: {e}"),
                                n => crate::warn!("worker connection: {e} ({n} similar warnings suppressed)"),
                            }
                        }
                        tx.send_replace(None);
                    }
                    Err(e) => crate::debug!("connecting to the worker at {}: {e}", settings.socket.display()),
                }
                if tx.is_closed() {
                    return;
                }
                tokio::time::sleep(settings.retry_interval).await;
            }
        });
        Self { current: rx, gauge }
    }

    pub fn current(&self) -> Option<Arc<Connection>> {
        self.current.borrow().clone().filter(|c| c.is_open())
    }

    /// Audio bytes the server holds right now, across every connection.
    pub fn gauge(&self) -> &AudioGauge {
        &self.gauge
    }

    /// Waits until a connection is up, for tests and startup logging.
    pub async fn connected(&self) -> Arc<Connection> {
        let mut rx = self.current.clone();
        let connection = rx.wait_for(|c| c.as_ref().is_some_and(|c| c.is_open())).await.expect("worker task runs");
        connection.clone().expect("checked")
    }
}

async fn serve(
    stream: UnixStream,
    publish: &watch::Sender<Option<Arc<Connection>>>,
    settings: &Settings,
    gauge: &AudioGauge,
) -> Result<(), ReadError> {
    let (mut reader, mut writer) = stream.into_split();
    let hello = match tokio::time::timeout(settings.hello_timeout, protocol::read_frame(&mut reader, |_| None)).await {
        Err(_) => return Err(ReadError::Violation("no hello in time".into())),
        Ok(frame) => match frame? {
            WorkerFrame::Hello(hello) => hello,
            _ => return Err(ReadError::Violation("first frame isn't a hello".into())),
        },
    };
    if hello.max_chunk_samples > settings.max_chunk_samples {
        return Err(ReadError::Violation(format!(
            "the worker's chunk cap {} is over the configured ceiling {}",
            hello.max_chunk_samples, settings.max_chunk_samples
        )));
    }
    let slots = settings.slots.min(hello.slots);
    crate::info!("worker connected: {} voices, max batch {}, {slots} slots", hello.voices.len(), hello.max_batch);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(3 * usize::from(slots));

    let connection = Arc::new(Connection::new(hello, out_tx, slots, gauge.clone()));
    publish.send_replace(Some(connection.clone()));

    let read = read_loop(&mut reader, &connection, settings.progress_timeout, settings.slot_timeout);
    let write = async {
        while let Some(frame) = out_rx.recv().await {
            writer.write_all(&frame).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let result = tokio::select! {
        r = read => r,
        w = write => w.map_err(ReadError::Io),
        _ = connection.reset.notified() => Err(ReadError::Violation("the front reset the connection".into())),
    };
    connection.close();
    result
}

async fn read_loop(
    reader: &mut OwnedReadHalf,
    connection: &Connection,
    timeout: Duration,
    slot_timeout: Duration,
) -> Result<(), ReadError> {
    let check_every = timeout.min(slot_timeout) / 4;
    loop {
        let read = protocol::read_frame(reader, |slot| connection.expected(slot));
        tokio::pin!(read);
        let frame = loop {
            tokio::select! {
                frame = &mut read => break frame?,
                _ = tokio::time::sleep(check_every) => {
                    if let Some(why) = connection.overdue(timeout, slot_timeout) {
                        return Err(ReadError::Violation(why));
                    }
                }
            }
        };
        connection.dispatch(frame).map_err(ReadError::Violation)?;
    }
}
