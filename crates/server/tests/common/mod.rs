//! A fake model worker speaking the worker protocol, and an HTTP client, for end-to-end tests.

#![allow(dead_code)]

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response};
use kokoro_server::pipeline::{Chunk, Lang, Pipeline, PipelineError};
use kokoro_server::protocol::worker_side::{self, FrontFrame, read_front};
use kokoro_server::protocol::{FailCode, Hello};
use kokoro_server::{Config, StubPipeline, Worker, http, serve};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UnixListener};
use tokio::sync::mpsc;

pub const KEY: &str = "test-key";
pub const VOICES: [&str; 7] = ["af_heart", "af_bella", "af_sky", "am_adam", "bf_emma", "jf_alpha", "zf_xiaobei"];

pub fn hello() -> Hello {
    Hello {
        vocab_size: 178,
        max_phonemes: 510,
        max_batch: 1,
        slots: 1024,
        max_chunk_samples: 2_880_000,
        voices: VOICES.map(String::from).to_vec(),
    }
}

/// The fake model: one duration unit per id, so `600 * (n + 2)` samples, all one loud value
/// derived from the ids. Trimming then removes exactly 24 samples from each end.
pub fn render(ids: &[u8]) -> (Vec<u16>, Vec<i16>) {
    let durations = vec![1; ids.len() + 2];
    let value = 1000 + (ids.iter().map(|&i| i as u32).sum::<u32>() % 20000) as i16;
    (durations, vec![value; 600 * (ids.len() + 2)])
}

/// What the server should send for `ids` rendered by [`render`].
pub fn expected_chunk(ids: &[u8]) -> Vec<i16> {
    let (_, pcm) = render(ids);
    pcm[24..pcm.len() - 24].to_vec()
}

pub fn respond(slot: u16, ids: &[u8]) -> Vec<u8> {
    let (durations, pcm) = render(ids);
    worker_side::result(slot, &durations, &pcm)
}

/// The audio the server should produce for `text` in one voice, using the stub pipeline.
pub fn expected_audio(text: &str) -> Vec<i16> {
    let mut out = Vec::new();
    for chunk in StubPipeline.chunks(Lang::AmericanEnglish, text) {
        let chunk = chunk.unwrap();
        match chunk.pause {
            Some(p) => out.extend(std::iter::repeat_n(0, (p * 24000.0) as usize)),
            None => out.extend(expected_chunk(&chunk.ids)),
        }
    }
    out
}

pub fn stub_chunks(text: &str) -> Vec<Chunk> {
    StubPipeline.chunks(Lang::AmericanEnglish, text).map(Result::unwrap).collect()
}

/// A worker whose every move the test makes: it reports each frame the server sends, and
/// writes whatever bytes the test gives it.
pub struct ScriptedWorker {
    pub frames: mpsc::UnboundedReceiver<Event>,
    /// `style_len` of each Synthesize taken by [`Self::synthesize`].
    pub style_lens: Vec<u16>,
    out: mpsc::UnboundedSender<Vec<u8>>,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Connected,
    Frame(FrontFrame),
    /// The server closed the connection (or sent something unreadable).
    Closed,
}

impl ScriptedWorker {
    /// Listens at `path`, greeting each connection with `hello`. Bytes sent go to the latest
    /// connection.
    pub fn listen(path: &Path, hello_frame: Vec<u8>) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let out_rx = Arc::new(tokio::sync::Mutex::new(out_rx));
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                let (mut reader, mut writer) = stream.into_split();
                if writer.write_all(&hello_frame).await.is_err() {
                    continue;
                }
                let _ = event_tx.send(Event::Connected);
                let events = event_tx.clone();
                let read = async move {
                    loop {
                        match read_front(&mut reader).await {
                            Ok(frame) => {
                                let _ = events.send(Event::Frame(frame));
                            }
                            Err(_) => {
                                let _ = events.send(Event::Closed);
                                return;
                            }
                        }
                    }
                };
                let out_rx = out_rx.clone();
                let events = event_tx.clone();
                let write = async move {
                    let mut out_rx = out_rx.lock().await;
                    while let Some(bytes) = out_rx.recv().await {
                        if bytes.is_empty() {
                            return;
                        }
                        // The server hung up while a large frame was still going out.
                        if writer.write_all(&bytes).await.is_err() {
                            let _ = events.send(Event::Closed);
                            return;
                        }
                    }
                };
                tokio::select! {
                    _ = read => {}
                    _ = write => {}
                }
            }
        });
        Self { frames: event_rx, style_lens: Vec::new(), out: out_tx }
    }

    pub fn send(&self, bytes: Vec<u8>) {
        self.out.send(bytes).unwrap();
    }

    /// Closes the current connection from the worker's side.
    pub fn hang_up(&self) {
        self.out.send(Vec::new()).unwrap();
    }

    pub async fn next(&mut self) -> Event {
        tokio::time::timeout(Duration::from_secs(10), self.frames.recv())
            .await
            .expect("the server sent nothing for 10 s")
            .expect("worker task runs")
    }

    /// The next Synthesize: `(slot, priority, speed in hundredths, voice, ids)`.
    pub async fn synthesize(&mut self) -> (u16, u8, u16, u16, Vec<u8>) {
        loop {
            match self.next().await {
                Event::Frame(FrontFrame::Synthesize { slot, priority, speed, voice, ids, style_len }) => {
                    self.style_lens.push(style_len);
                    return (slot, priority, speed, voice, ids);
                }
                Event::Connected => continue,
                other => panic!("expected a Synthesize frame, got {other:?}"),
            }
        }
    }

    /// Nothing more arrives within `wait`.
    pub async fn quiet(&mut self, wait: Duration) -> Option<Event> {
        tokio::time::timeout(wait, self.frames.recv()).await.ok().flatten()
    }
}

/// A worker that renders one request at a time, highest priority first, then oldest, and logs
/// what it rendered.
pub struct AutoWorker {
    pub log: Arc<Mutex<Vec<Rendered>>>,
    pub most_queued: Arc<AtomicUsize>,
}

#[derive(Debug, Clone)]
pub struct Rendered {
    pub slot: u16,
    pub priority: u8,
    pub first_id: u8,
    /// Other requests rendered between this one's arrival and its own rendering.
    pub waited: usize,
}

impl AutoWorker {
    pub fn listen(path: &Path, render_time: Duration) -> Self {
        let listener = UnixListener::bind(path).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let most_queued = Arc::new(AtomicUsize::new(0));
        let (log2, most2, log3) = (log.clone(), most_queued.clone(), log.clone());
        tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else { return };
            let (mut reader, mut writer) = stream.into_split();
            writer.write_all(&worker_side::hello(&hello())).await.unwrap();
            // (priority, arrival order, slot, ids, requests rendered before it arrived)
            let queue = Arc::new(Mutex::new(Vec::<(u8, u64, u16, Vec<u8>, usize)>::new()));
            let wake = Arc::new(tokio::sync::Notify::new());
            let (q2, w2) = (queue.clone(), wake.clone());
            let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel::<u16>();
            tokio::spawn(async move {
                let mut arrivals = 0;
                while let Ok(frame) = read_front(&mut reader).await {
                    match frame {
                        FrontFrame::Synthesize { slot, priority, ids, .. } => {
                            let rendered = log3.lock().unwrap().len();
                            let mut q = q2.lock().unwrap();
                            q.push((priority, arrivals, slot, ids, rendered));
                            arrivals += 1;
                            most2.fetch_max(q.len(), Ordering::SeqCst);
                        }
                        FrontFrame::Cancel { slot } => {
                            let _ = cancel_tx.send(slot);
                        }
                    }
                    w2.notify_one();
                }
            });
            loop {
                while let Ok(slot) = cancel_rx.try_recv() {
                    let removed = {
                        let mut q = queue.lock().unwrap();
                        let before = q.len();
                        q.retain(|(_, _, s, _, _)| *s != slot);
                        before != q.len()
                    };
                    if removed && writer.write_all(&worker_side::failed(slot, FailCode::Cancelled)).await.is_err() {
                        return;
                    }
                }
                let next = {
                    let mut q = queue.lock().unwrap();
                    let best = q.iter().enumerate().min_by_key(|(_, (p, order, ..))| (*p, *order)).map(|(i, _)| i);
                    best.map(|i| q.remove(i))
                };
                let Some((priority, _, slot, ids, arrived)) = next else {
                    wake.notified().await;
                    continue;
                };
                tokio::time::sleep(render_time).await;
                {
                    let mut log = log2.lock().unwrap();
                    let waited = log.len() - arrived;
                    log.push(Rendered { slot, priority, first_id: ids[0], waited });
                }
                if writer.write_all(&respond(slot, &ids)).await.is_err() {

                    return;
                }
            }
        });
        Self { log, most_queued }
    }
}

/// Counts calls, to show a refused request never reached text processing.
#[derive(Default)]
pub struct CountingPipeline {
    pub calls: AtomicUsize,
}

impl Pipeline for CountingPipeline {
    fn chunks<'a>(&'a self, lang: Lang, text: &'a str) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        StubPipeline.chunks(lang, text)

    }
}

pub struct Server {
    pub addr: SocketAddr,
    pub worker: Worker,
    pub socket: PathBuf,
    pub dir: tempfile::TempDir,
}

pub fn config(socket: &Path) -> Config {
    Config {
        worker_socket: socket.to_owned(),
        api_keys: vec![KEY.into(), "other-key".into()],
        reconnect_interval_ms: 20,
        ..Config::default()
    }
}

/// Starts the server on a random port. The worker socket is `server.socket`, not yet listened on.
pub async fn start(adjust: impl FnOnce(&mut Config), pipeline: Arc<dyn Pipeline>) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("worker.sock");
    let mut config = config(&socket);
    adjust(&mut config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let worker = Worker::spawn(config.worker_settings());
    let limits = serve::Limits::from(&config);
    let app = http::router(config, worker.clone(), pipeline);
    tokio::spawn(serve::serve(listener, app, limits, std::future::pending()));
    Server { addr, worker, socket, dir }
}

pub async fn request(addr: SocketAddr, req: Request<Full<Bytes>>) -> Response<Incoming> {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.unwrap();
    tokio::spawn(conn);
    sender.send_request(req).await.unwrap()
}

pub fn speech_request(key: Option<&str>, body: serde_json::Value) -> Request<Full<Bytes>> {
    let mut builder = Request::post("/v1/audio/speech").header("host", "test").header("content-type", "application/json");
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    builder.body(Full::new(Bytes::from(body.to_string()))).unwrap()
}

pub fn get(path: &str, key: Option<&str>) -> Request<Full<Bytes>> {
    let mut builder = Request::get(path).header("host", "test");
    if let Some(key) = key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    builder.body(Full::new(Bytes::new())).unwrap()
}

pub async fn speak(addr: SocketAddr, body: serde_json::Value) -> Response<Incoming> {
    request(addr, speech_request(Some(KEY), body)).await
}

pub fn pcm(text: &str) -> serde_json::Value {
    serde_json::json!({ "model": "kokoro", "voice": "af_heart", "input": text, "response_format": "pcm", "stream": true })
}

pub async fn body_bytes(response: Response<Incoming>) -> Result<Vec<u8>, hyper::Error> {
    let body = tokio::time::timeout(Duration::from_secs(10), response.into_body().collect())
        .await
        .expect("the response body stalled for 10 s");
    Ok(body?.to_bytes().to_vec())
}

pub async fn json_body(response: Response<Incoming>) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await.unwrap()).unwrap()
}

pub fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}

/// Reads body frames until at least `n` bytes have arrived.
pub async fn read_at_least(body: &mut Incoming, n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while out.len() < n {
        let frame = tokio::time::timeout(Duration::from_secs(10), body.frame())
            .await
            .expect("body stalled")
            .expect("body ended early")
            .expect("body failed");
        if let Ok(data) = frame.into_data() {
            out.extend_from_slice(&data);
        }
    }
    out
}

/// The stub pipeline, failing (or panicking) when asked for chunk `fail_at`.
pub struct BrokenPipeline {
    pub fail_at: usize,
    pub panic: bool,
}

impl Pipeline for BrokenPipeline {
    fn chunks<'a>(&'a self, lang: Lang, text: &'a str) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        Box::new(StubPipeline.chunks(lang, text)
.enumerate().map(|(i, chunk)| {
            if i < self.fail_at {
                chunk
            } else if self.panic {
                panic!("pipeline bug")
            } else {
                Err(PipelineError("cannot phonemize".into()))
            }
        }))
    }
}
