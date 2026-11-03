//! The front's side of the worker connection: one Unix socket, multiplexed across requests,
//! re-established whenever the worker goes away.
//!
//! Every frame the worker sends is checked against what the front asked for. Anything unexpected
//! closes the connection, and every request in flight on it fails; nothing is resubmitted.

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
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SynthError {
    /// The connection closed (worker restart or protocol violation) before the result came.
    ConnectionLost,
    /// The worker reported an internal error for this request.
    WorkerFailed,
    /// The text pipeline produced a chunk outside the worker's limits.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthesized {
    pub pcm: Vec<i16>,
    /// Predicted frames per id, with the two boundary ids at either end.
    pub durations: Vec<u16>,
}

/// What the scheduler asks the worker to render.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthRequest {
    pub speed: f32,
    pub voices: Arc<[(u16, f32)]>,
    pub ids: Vec<u8>,
}

type Reply = oneshot::Sender<Result<Synthesized, SynthError>>;

struct Pending {
    n_ids: usize,
    durations: Option<Vec<u16>>,
    /// `None` once cancelled: the worker still owes a final Audio or Failed frame.
    reply: Option<Reply>,
}

struct State {
    open: bool,
    pending: HashMap<u64, Pending>,
}

pub struct Connection {
    hello: Hello,
    voice_index: HashMap<String, u16>,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    state: Mutex<State>,
    next_id: AtomicU64,
}

impl Connection {
    pub(crate) fn new(hello: Hello, outgoing: mpsc::UnboundedSender<Vec<u8>>) -> Self {
        let voice_index = hello.voices.iter().enumerate().map(|(i, v)| (v.clone(), i as u16)).collect();
        Self {
            hello,
            voice_index,
            outgoing,
            state: Mutex::new(State { open: true, pending: HashMap::new() }),
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

    fn valid(&self, r: &SynthRequest) -> bool {
        let ids_ok = !r.ids.is_empty()
            && r.ids.len() <= usize::from(self.hello.max_phonemes)
            && r.ids.iter().all(|&id| id >= 1 && u16::from(id) < self.hello.vocab_size);
        let voices_ok = (1..=protocol::MAX_VOICES_PER_MIX).contains(&r.voices.len())
            && r.voices
                .iter()
                .all(|&(i, w)| usize::from(i) < self.hello.voices.len() && w.is_finite() && w > 0.0);
        let speed_ok = (protocol::SPEED_MIN..=protocol::SPEED_MAX).contains(&r.speed);
        ids_ok && voices_ok && speed_ok
    }

    /// Queues a request on the worker.
    pub fn submit(self: &Arc<Self>, priority: Priority, request: &SynthRequest) -> Result<Ticket, SynthError> {
        if !self.valid(request) {
            return Err(SynthError::Unrenderable);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.lock().unwrap();
            if !state.open {
                return Err(SynthError::ConnectionLost);
            }
            state.pending.insert(id, Pending { n_ids: request.ids.len(), durations: None, reply: Some(tx) });
            let frame = encode_synthesize(&Synthesize {
                id,
                priority,
                speed: request.speed,
                voices: &request.voices,
                ids: &request.ids,
            });
            if self.outgoing.send(frame).is_err() {
                state.pending.remove(&id);
                return Err(SynthError::ConnectionLost);
            }
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
        {
            let _ = self.outgoing.send(encode_cancel(id));
        }
    }

    fn expected_samples(&self, id: u64) -> Option<u64> {
        let state = self.state.lock().unwrap();
        let durations = state.pending.get(&id)?.durations.as_ref()?;
        Some(durations.iter().map(|&d| u64::from(d)).sum::<u64>() * SAMPLES_PER_FRAME)
    }

    fn dispatch(&self, frame: WorkerFrame) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
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
                pending.durations = Some(durations);
                Ok(())
            }
            WorkerFrame::Audio { id, pcm } => {
                let pending = state.pending.remove(&id).expect("read_frame only accepts audio we expect");
                if let Some(reply) = pending.reply {
                    let durations = pending.durations.expect("audio is only expected after durations");
                    let _ = reply.send(Ok(Synthesized { pcm, durations }));
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
                    let _ = reply.send(Err(SynthError::WorkerFailed));
                }
                Ok(())
            }
        }
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.open = false;
        for (_, pending) in state.pending.drain() {
            if let Some(reply) = pending.reply {
                let _ = reply.send(Err(SynthError::ConnectionLost));
            }
        }
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
}

impl Worker {
    /// Connects to the worker's socket, and again `retry_interval` after every disconnect or
    /// failed attempt, for as long as the returned handle (or a clone) exists.
    pub fn spawn(socket: PathBuf, retry_interval: Duration) -> Self {
        let (tx, rx) = watch::channel(None);
        tokio::spawn(async move {
            loop {
                match UnixStream::connect(&socket).await {
                    Ok(stream) => {
                        if let Err(e) = serve(stream, &tx).await {
                            tracing::warn!("{e}");
                        }
                        tx.send_replace(None);
                    }
                    Err(e) => tracing::debug!("connecting to the worker at {}: {e}", socket.display()),
                }
                if tx.is_closed() {
                    return;
                }
                tokio::time::sleep(retry_interval).await;
            }
        });
        Self { current: rx }
    }

    pub fn current(&self) -> Option<Arc<Connection>> {
        self.current.borrow().clone().filter(|c| c.is_open())
    }

    /// Waits until a connection is up, for tests and startup logging.
    pub async fn connected(&self) -> Arc<Connection> {
        let mut rx = self.current.clone();
        let connection = rx.wait_for(|c| c.as_ref().is_some_and(|c| c.is_open())).await.expect("worker task runs");
        connection.clone().expect("checked")
    }
}

async fn serve(stream: UnixStream, publish: &watch::Sender<Option<Arc<Connection>>>) -> Result<(), ReadError> {
    let (mut reader, mut writer) = stream.into_split();
    let hello = match protocol::read_frame(&mut reader, |_| None).await? {
        WorkerFrame::Hello(hello) => hello,
        _ => return Err(ReadError::Violation("first frame isn't a hello".into())),
    };
    tracing::info!("worker connected: {} voices, max batch {}", hello.voices.len(), hello.max_batch);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let connection = Arc::new(Connection::new(hello, out_tx));
    publish.send_replace(Some(connection.clone()));

    let read = read_loop(&mut reader, &connection);
    let write = async {
        while let Some(frame) = out_rx.recv().await {
            writer.write_all(&frame).await?;
        }
        Ok::<_, std::io::Error>(())
    };
    let result = tokio::select! {
        r = read => r,
        w = write => w.map_err(ReadError::Io),
    };
    connection.close();
    result
}

async fn read_loop(reader: &mut OwnedReadHalf, connection: &Connection) -> Result<(), ReadError> {
    loop {
        let frame = protocol::read_frame(reader, |id| connection.expected_samples(id)).await?;
        connection.dispatch(frame).map_err(ReadError::Violation)?;
    }
}
