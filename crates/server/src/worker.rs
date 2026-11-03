//! The front's side of the worker connection: one Unix socket, multiplexed across requests,
//! re-established whenever the worker goes away.
//!
//! Every frame the worker sends is checked against what the front asked for. Anything unexpected
//! closes the connection, and every request in flight on it fails; nothing is resubmitted.
//!
//! Resources are charged before they're used and released only when the worker is done:
//! - each request holds an admission slot from submission until its final Audio or Failed frame,
//!   so cancelled requests the worker hasn't acknowledged still count, and the outgoing frame
//!   queue (at most a Synthesize and a Cancel per slot) is bounded;
//! - each request reserves the most audio it could produce from a server-wide byte budget, cut
//!   to the exact size when its durations arrive, and the reservation travels with the audio
//!   until the HTTP layer has written it.
//!
//! A worker that doesn't say hello in time, or makes no progress while it has work, is
//! disconnected.

use crate::protocol::{
    self, FailCode, Hello, Priority, ReadError, SAMPLES_PER_FRAME, Synthesize, WorkerFrame, encode_cancel,
    encode_synthesize,
};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::OwnedReadHalf;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthError {
    /// The connection closed (worker restart, protocol violation or stall) before the result came.
    ConnectionLost,
    /// The worker reported an internal error for this request.
    WorkerFailed,
    /// The text pipeline produced a chunk outside the worker's limits or the audio budget.
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

/// Bytes of audio memory, held from before the audio is allocated until it's dropped.
#[derive(Debug)]
pub struct Reservation(OwnedSemaphorePermit);

impl Reservation {
    pub fn bytes(&self) -> usize {
        self.0.num_permits()
    }

    fn shrink_to(&mut self, bytes: usize) {
        if let Some(extra) = self.0.split(self.bytes().saturating_sub(bytes)) {
            drop(extra);
        }
    }
}

/// The server-wide budget for audio held in memory: rendered, reordered or being written.
#[derive(Debug, Clone)]
pub struct AudioBudget {
    bytes: Arc<Semaphore>,
    total: usize,
}

impl AudioBudget {
    pub fn new(total: usize) -> Self {
        Self { bytes: Arc::new(Semaphore::new(total)), total }
    }

    pub fn available(&self) -> usize {
        self.bytes.available_permits()
    }

    /// Waits for `bytes`; `None` if they exceed the whole budget and never could be granted.
    pub async fn reserve(&self, bytes: usize) -> Option<Reservation> {
        if bytes > self.total {
            return None;
        }
        let bytes = u32::try_from(bytes).ok()?;
        let permit = self.bytes.clone().acquire_many_owned(bytes).await.expect("the budget is never closed");
        Some(Reservation(permit))
    }
}

/// The most audio, in bytes, a request for `n_ids` ids at `speed` can produce.
pub fn max_audio_bytes(n_ids: usize, speed: f32) -> usize {
    (n_ids + 2) * usize::from(protocol::max_duration(speed)) * SAMPLES_PER_FRAME as usize * 2
}

#[derive(Debug)]
pub struct Synthesized {
    pub pcm: Vec<i16>,
    /// Predicted frames per id, with the two boundary ids at either end.
    pub durations: Vec<u16>,
    /// Covers `pcm`; keep it until the audio is gone.
    pub reservation: Reservation,
}

/// What the scheduler asks the worker to render.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub speed: f32,
    pub voices: Arc<[(u16, f32)]>,
    pub ids: Vec<u8>,
    /// The style row to use, plus one: `ids.len()`, or the whole clause's length for a piece of a
    /// stitched chunk.
    pub style_len: u16,
}

type Reply = oneshot::Sender<Result<Synthesized, SynthError>>;

struct Pending {
    n_ids: usize,
    max_duration: u16,
    durations: Option<Vec<u16>>,
    /// `None` once cancelled: the worker still owes a final Audio or Failed frame.
    reply: Option<Reply>,
    audio: Reservation,
    _slot: OwnedSemaphorePermit,
}

struct State {
    open: bool,
    pending: HashMap<u64, Pending>,
    /// When the worker last finished a frame, or was last given work while idle.
    last_progress: Instant,
}

/// Limits on the worker and how long it may take.
#[derive(Debug, Clone)]
pub struct Settings {
    pub socket: PathBuf,
    pub retry_interval: Duration,
    pub hello_timeout: Duration,
    /// Longest the worker may go without sending a whole frame while it has work.
    pub progress_timeout: Duration,
    /// Requests submitted and not yet finished by the worker, cancelled ones included.
    pub max_outstanding: usize,
    pub audio_budget_bytes: usize,
}

pub struct Connection {
    hello: Hello,
    voice_index: HashMap<String, u16>,
    outgoing: mpsc::Sender<Vec<u8>>,
    slots: Arc<Semaphore>,
    budget: AudioBudget,
    state: Mutex<State>,
    reset: Notify,
    next_id: AtomicU64,
}

impl Connection {
    pub(crate) fn new(
        hello: Hello,
        outgoing: mpsc::Sender<Vec<u8>>,
        max_outstanding: usize,
        budget: AudioBudget,
    ) -> Self {
        let voice_index = hello.voices.iter().enumerate().map(|(i, v)| (v.clone(), i as u16)).collect();
        Self {
            hello,
            voice_index,
            outgoing,
            slots: Arc::new(Semaphore::new(max_outstanding)),
            budget,
            state: Mutex::new(State { open: true, pending: HashMap::new(), last_progress: Instant::now() }),
            reset: Notify::new(),
            next_id: AtomicU64::new(0),
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

    pub fn budget(&self) -> &AudioBudget {
        &self.budget
    }

    /// Requests the worker hasn't finished, cancelled ones included.
    pub fn outstanding(&self) -> usize {
        self.state.lock().unwrap().pending.len()
    }

    fn valid(&self, r: &SynthRequest) -> bool {
        let ids_ok = !r.ids.is_empty()
            && r.ids.len() <= usize::from(self.hello.max_phonemes)
            && (1..=self.hello.max_phonemes).contains(&r.style_len)
            && r.ids.iter().all(|&id| id >= 1 && u16::from(id) < self.hello.vocab_size);
        let voices_ok = (1..=protocol::MAX_VOICES_PER_MIX).contains(&r.voices.len())
            && r.voices
                .iter()
                .all(|&(i, w)| usize::from(i) < self.hello.voices.len() && w.is_finite() && w > 0.0);
        let speed_ok = (protocol::SPEED_MIN..=protocol::SPEED_MAX).contains(&r.speed);
        ids_ok && voices_ok && speed_ok
    }

    /// Queues a request on the worker, first waiting for an admission slot and for the audio it
    /// may produce to fit the budget. Dropping the future while it waits gives both back.
    pub async fn submit(self: &Arc<Self>, priority: Priority, request: &SynthRequest) -> Result<Ticket, SynthError> {
        if !self.valid(request) {
            return Err(SynthError::Unrenderable);
        }
        let slot = self.slots.clone().acquire_owned().await.map_err(|_| SynthError::ConnectionLost)?;
        let audio = self
            .budget
            .reserve(max_audio_bytes(request.ids.len(), request.speed))
            .await
            .ok_or(SynthError::Unrenderable)?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.lock().unwrap();
            if !state.open {
                return Err(SynthError::ConnectionLost);
            }
            let frame = encode_synthesize(&Synthesize {
                id,
                priority,
                speed: request.speed,
                voices: &request.voices,
                ids: &request.ids,
                style_len: request.style_len,
            });
            if self.outgoing.try_send(frame).is_err() {
                drop(state);
                self.close();
                return Err(SynthError::ConnectionLost);
            }
            if state.pending.is_empty() {
                state.last_progress = Instant::now();
            }
            let pending = Pending {
                n_ids: request.ids.len(),
                max_duration: protocol::max_duration(request.speed),
                durations: None,
                reply: Some(tx),
                audio,
                _slot: slot,
            };
            state.pending.insert(id, pending);
        }
        Ok(Ticket { id, connection: self.clone(), reply: rx, finished: false })
    }

    fn cancel(&self, id: u64) {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return;
        }
        if let Some(pending) = state.pending.get_mut(&id)
            && pending.reply.take().is_some()
            && self.outgoing.try_send(encode_cancel(id)).is_err()
        {
            drop(state);
            self.close();
        }
    }

    fn expected_samples(&self, id: u64) -> Option<u64> {
        let state = self.state.lock().unwrap();
        let durations = state.pending.get(&id)?.durations.as_ref()?;
        Some(durations.iter().map(|&d| u64::from(d)).sum::<u64>() * SAMPLES_PER_FRAME)
    }

    fn dispatch(&self, frame: WorkerFrame) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.last_progress = Instant::now();
        match frame {
            WorkerFrame::Hello(_) => Err("second hello".into()),
            WorkerFrame::Durations { id, durations } => {
                let pending = state.pending.get_mut(&id).ok_or(format!("durations for unknown request {id}"))?;
                if pending.durations.is_some() {
                    return Err(format!("second durations for request {id}"));
                }
                if durations.len() != pending.n_ids + 2 {
                    return Err(format!("request {id}: {} durations for {} ids", durations.len(), pending.n_ids));
                }
                if durations.iter().any(|&d| d > pending.max_duration) {
                    return Err(format!("request {id}: a duration over {}", pending.max_duration));
                }
                let samples: usize = durations.iter().map(|&d| usize::from(d)).sum::<usize>() * SAMPLES_PER_FRAME as usize;
                pending.audio.shrink_to(2 * samples);
                pending.durations = Some(durations);
                Ok(())
            }
            WorkerFrame::Audio { id, pcm } => {
                let pending = state.pending.remove(&id).expect("read_frame only accepts audio we expect");
                if let Some(reply) = pending.reply {
                    let durations = pending.durations.expect("audio is only expected after durations");
                    let _ = reply.send(Ok(Synthesized { pcm, durations, reservation: pending.audio }));
                }
                Ok(())
            }
            WorkerFrame::Failed { id, code } => {
                let pending = state.pending.get(&id).ok_or(format!("failure for unknown request {id}"))?;
                if code == FailCode::Cancelled && pending.reply.is_some() {
                    return Err(format!("request {id} reported cancelled, but it wasn't"));
                }
                let pending = state.pending.remove(&id).expect("just found");
                if let Some(reply) = pending.reply {
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

    /// Whether the worker has had work and sent no whole frame for `timeout`.
    fn stalled(&self, timeout: Duration) -> bool {
        let state = self.state.lock().unwrap();
        !state.pending.is_empty() && state.last_progress.elapsed() >= timeout
    }

    /// Fails everything in flight and tells the connection task to hang up.
    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.open = false;
        self.slots.close();
        for (_, pending) in state.pending.drain() {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Err(SynthError::ConnectionLost));
            }
        }
        self.reset.notify_one();
    }
}

/// A submitted request. Dropping it before its result arrives cancels it on the worker.
pub struct Ticket {
    id: u64,
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
            self.connection.cancel(self.id);
        }
    }
}

/// The current worker connection, kept up by a background task.
#[derive(Clone)]
pub struct Worker {
    current: watch::Receiver<Option<Arc<Connection>>>,
    budget: AudioBudget,
}

impl Worker {
    /// Connects to the worker's socket, and again `retry_interval` after every disconnect or
    /// failed attempt, for as long as the returned handle (or a clone) exists.
    pub fn spawn(settings: Settings) -> Self {
        let (tx, rx) = watch::channel(None);
        let budget = AudioBudget::new(settings.audio_budget_bytes);
        let task_budget = budget.clone();
        tokio::spawn(async move {
            loop {
                match UnixStream::connect(&settings.socket).await {
                    Ok(stream) => {
                        if let Err(e) = serve(stream, &tx, &settings, &task_budget).await {
                            tracing::warn!("{e}");
                        }
                        tx.send_replace(None);
                    }
                    Err(e) => tracing::debug!("connecting to the worker at {}: {e}", settings.socket.display()),
                }
                if tx.is_closed() {
                    return;
                }
                tokio::time::sleep(settings.retry_interval).await;
            }
        });
        Self { current: rx, budget }
    }

    pub fn current(&self) -> Option<Arc<Connection>> {
        self.current.borrow().clone().filter(|c| c.is_open())
    }

    pub fn budget(&self) -> &AudioBudget {
        &self.budget
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
    budget: &AudioBudget,
) -> Result<(), ReadError> {
    let (mut reader, mut writer) = stream.into_split();
    let hello = match tokio::time::timeout(settings.hello_timeout, protocol::read_frame(&mut reader, |_| None)).await {
        Err(_) => return Err(ReadError::Violation("no hello in time".into())),
        Ok(frame) => match frame? {
            WorkerFrame::Hello(hello) => hello,
            _ => return Err(ReadError::Violation("first frame isn't a hello".into())),
        },
    };
    tracing::info!("worker connected: {} voices, max batch {}", hello.voices.len(), hello.max_batch);
    let (out_tx, mut out_rx) = mpsc::channel::<Vec<u8>>(2 * settings.max_outstanding);
    let connection = Arc::new(Connection::new(hello, out_tx, settings.max_outstanding, budget.clone()));
    publish.send_replace(Some(connection.clone()));

    let read = read_loop(&mut reader, &connection, settings.progress_timeout);
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

async fn read_loop(reader: &mut OwnedReadHalf, connection: &Connection, timeout: Duration) -> Result<(), ReadError> {
    let check_every = timeout / 4;
    loop {
        let read = protocol::read_frame(reader, |id| connection.expected_samples(id));
        tokio::pin!(read);
        let frame = loop {
            tokio::select! {
                frame = &mut read => break frame?,
                _ = tokio::time::sleep(check_every) => {
                    if connection.stalled(timeout) {
                        return Err(ReadError::Violation(format!("no progress for {timeout:?} with work outstanding")));
                    }
                }
            }
        };
        connection.dispatch(frame).map_err(ReadError::Violation)?;
    }
}
