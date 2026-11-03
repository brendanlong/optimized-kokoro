//! Resource limits and lifetimes: what a request, a cancelled request or a worker may hold, and
//! for how long. Several of these came from review of PR #10.

mod common;

use common::*;
use kokoro_server::pipeline::{Chunk, PipelineError};
use kokoro_server::protocol::worker_side::{self, FrontFrame};
use kokoro_server::protocol::{self, FailCode};
use kokoro_server::{Pipeline, StubPipeline};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UnixListener};
use tower::ServiceExt;

fn one_stream(c: &mut kokoro_server::Config) {
    c.max_concurrent_requests = 1;
    c.max_streams_per_key = 1;
}

/// Sends a speech request over a socket with a small receive buffer, and doesn't read.
async fn raw_speech(addr: std::net::SocketAddr, text: &str) -> TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stream = socket.connect(addr).await.unwrap();
    let body = pcm(text).to_string();
    let request = format!(
        "POST /v1/audio/speech HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer {KEY}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream
}

/// Speaks `text` with the auto worker, expecting success.
async fn speak_ok(addr: std::net::SocketAddr, text: &str) {
    let response = speak(addr, pcm(text)).await;
    assert_eq!(response.status(), 200);
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio(text));
}

/// Durations adding up to exactly `samples / 600` units for `n_ids` ids, none over `per_id`.
fn durations_for(n_ids: usize, samples: u64, per_id: u16) -> Vec<u16> {
    let units = samples / 600;
    let n = n_ids as u64 + 2;
    let mut durations = vec![(units / n) as u16; n as usize];
    for d in durations.iter_mut().take((units % n) as usize) {
        *d += 1;
    }
    assert!(durations.iter().all(|&d| (1..=per_id).contains(&d)));
    durations
}

/// There is no audio byte budget: memory is bounded by counts. Clients that never read, at the
/// request limit, with every chunk as large as the worker's cap allows, leave the server holding
/// no more than `Config::audio_bound`, and the bound isn't loose.
#[tokio::test]
async fn audio_held_for_clients_that_dont_read_stays_under_the_bound() {
    let mut used = None;
    let server = start(
        |c| {
            c.max_concurrent_requests = 3;
            c.max_streams_per_key = 3;
            c.lookahead = 3;
            used = Some(c.clone());
        },
        Arc::new(StubPipeline),
    )
    .await;
    let config = used.unwrap();
    let mut small = hello();
    small.max_chunk_samples = 24_000 * 60; // 60 s: a chunk (2.9 MB) is bigger than socket buffers
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&small));
    server.worker.connected().await;

    // Sentences long enough (about 60 ids) for a chunk at the cap to fit Kokoro's per-id maximum.
    let text = (1..=12)
        .map(|i| format!("Sentence number {i} is long enough to need a lot of audio for its words."))
        .collect::<Vec<_>>()
        .join(" ");
    let mut clients = Vec::new();
    for _ in 0..config.max_concurrent_requests {
        clients.push(raw_speech(server.addr, &text).await);
    }
    // Render every chunk the server asks for at the cap, until it stops asking.
    let mut rendered = 0;
    while let Some(event) = worker.quiet(Duration::from_millis(500)).await {
        if let Event::Frame(FrontFrame::Synthesize { id, speed, ids, .. }) = event {
            let samples = protocol::chunk_cap(small.max_chunk_samples, speed);
            let durations = durations_for(ids.len(), samples, protocol::max_duration(speed));
            worker.send(worker_side::durations(id, &durations));
            worker.send(worker_side::audio(id, &vec![12_000; samples as usize]));
            rendered += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let held = server.worker.gauge().held();
    let bound = config.audio_bound(small.max_chunk_samples, 1.0);
    assert!(rendered < 12 * clients.len(), "the server kept asking for chunks nobody reads");
    assert!(held <= bound, "holding {held} bytes, over the bound {bound}");
    // Kernel socket buffers take some of what hyper writes, so the bound isn't reached exactly.
    assert!(2 * held >= bound, "the bound is loose: {held} held of {bound}");
    drop(clients);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(server.worker.gauge().held(), 0, "audio outlived its clients");
}

#[tokio::test]
async fn cancelled_requests_hold_worker_admission_until_acknowledged() {
    let server = start(
        |c| {
            one_stream(c);
            c.worker_max_outstanding = Some(3);
        },
        Arc::new(StubPipeline),
    )
    .await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    let connection = server.worker.connected().await;

    let client = tokio::spawn(speak(server.addr, pcm("Alpha one. Bravo two. Charlie three. Delta four. Echo five.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.synthesize().await;
    worker.synthesize().await;
    worker.send(respond(id, &ids));
    let mut body = client.await.unwrap().into_body();
    read_at_least(&mut body, 2).await;
    worker.synthesize().await;
    drop(body);
    let mut cancelled = Vec::new();
    for _ in 0..3 {
        match worker.next().await {
            Event::Frame(FrontFrame::Cancel { id }) => cancelled.push(id),
            other => panic!("expected a cancellation, got {other:?}"),
        }
    }
    assert_eq!(connection.outstanding(), 3);

    // Until the worker acknowledges a cancellation, there's no room for new work.
    let next = tokio::spawn(speak(server.addr, pcm("Hi.")));
    assert_eq!(worker.quiet(Duration::from_millis(300)).await, None);
    worker.send(worker_side::failed(cancelled[0], FailCode::Cancelled));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.send(respond(id, &ids));
    assert_eq!(next.await.unwrap().status(), 200);
    assert!(connection.outstanding() <= 3);
}

#[tokio::test]
async fn a_silent_worker_is_disconnected_and_its_requests_fail() {
    let server = start(|c| c.worker_progress_timeout_ms = 300, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let started = Instant::now();
    let client = tokio::spawn(speak(server.addr, pcm("Hello there.")));
    worker.synthesize().await;
    assert_eq!(worker.next().await, Event::Closed);
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(client.await.unwrap().status(), 503);
    // An idle connection is left alone.
    assert_eq!(worker.next().await, Event::Connected);
    assert_eq!(worker.quiet(Duration::from_millis(600)).await, None);
}

#[tokio::test]
async fn a_worker_that_never_says_hello_is_dropped() {
    let server = start(|c| c.worker_hello_timeout_ms = 200, Arc::new(StubPipeline)).await;
    let listener = UnixListener::bind(&server.socket).unwrap();
    let (mut stream, _) = listener.accept().await.unwrap();
    let started = Instant::now();
    let mut buf = [0; 1];
    let n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf)).await.unwrap().unwrap();
    assert_eq!(n, 0, "the front should hang up");
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert!(server.worker.current().is_none());
    assert_eq!(request(server.addr, get("/health", None)).await.status(), 503);
}

#[tokio::test]
async fn hello_with_the_most_voices_parses_quickly() {
    let mut h = hello();
    h.voices = (0..u16::MAX).map(|i| format!("v{i:05}")).collect();
    let bytes = worker_side::hello(&h);
    let started = Instant::now();
    let parsed = protocol::read_frame(&mut &bytes[..], |_| None).await.unwrap();
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    assert_eq!(parsed, protocol::WorkerFrame::Hello(h));
}

/// Blocks planning (in `supports`) until released.
struct GatedPlan {
    calls: AtomicUsize,
    released: Mutex<bool>,
    wake: Condvar,
}

impl GatedPlan {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

impl Pipeline for GatedPlan {
    fn supports(&self, lang: &str) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
        StubPipeline.supports(lang)
    }

    fn chunks<'a>(
        &'a self,
        lang: &'a str,
        text: &'a str,
        options: &'a kokoro_text::Options,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        StubPipeline.chunks(lang, text, options)
    }
}

#[tokio::test]
async fn planning_keeps_its_permits_after_the_client_leaves() {
    let pipeline = Arc::new(GatedPlan { calls: AtomicUsize::new(0), released: Mutex::new(false), wake: Condvar::new() });
    let server = start(one_stream, pipeline.clone()).await;
    let worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;

    let first = raw_speech(server.addr, "First.").await;
    while pipeline.calls.load(Ordering::SeqCst) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    drop(first);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(speak(server.addr, pcm("Second.")).await.status(), 429);
    assert_eq!(pipeline.calls.load(Ordering::SeqCst), 1, "two planners ran under a limit of one");

    pipeline.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let response = speak(server.addr, pcm("Third.")).await;
        if response.status() == 200 {
            assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio("Third."));
            break;
        }
        assert_eq!(response.status(), 429);
        assert!(Instant::now() < deadline, "the first request's permits were never released");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(worker);
}

#[tokio::test]
async fn disconnecting_before_the_first_audio_cancels_the_work() {
    let server = start(one_stream, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let first = raw_speech(server.addr, "First.").await;
    let (id, ..) = worker.synthesize().await;
    drop(first);
    assert_eq!(worker.next().await, Event::Frame(FrontFrame::Cancel { id }));
    worker.send(worker_side::failed(id, FailCode::Cancelled));
    let client = tokio::spawn(speak(server.addr, pcm("Second.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.send(respond(id, &ids));
    assert_eq!(client.await.unwrap().status(), 200);
}

/// A client that stops reading keeps its permits until its response is written or dropped.
#[tokio::test]
async fn a_slow_reader_keeps_its_stream_permits() {
    let server = start(one_stream, Arc::new(StubPipeline)).await;
    let _worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;
    // A minute of silence (2.88 MB) can't fit in the socket buffers.
    let mut first = raw_speech(server.addr, "Hi. [pause:60s]").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(speak(server.addr, pcm("Second.")).await.status(), 429);

    let mut response = Vec::new();
    let mut buf = vec![0; 64 * 1024];
    tokio::time::timeout(Duration::from_secs(10), async {
        while !response.ends_with(b"0\r\n\r\n") {
            let n = first.read(&mut buf).await.unwrap();
            assert!(n > 0, "connection closed early");
            response.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .unwrap();
    assert!(response.len() > 2_880_000);
    speak_ok(server.addr, "Second.").await;
}

#[tokio::test]
async fn an_unread_response_body_keeps_its_permits() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let _worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;
    let mut config = config(&server.socket);
    one_stream(&mut config);
    let app = kokoro_server::http::router(config, server.worker.clone(), Arc::new(StubPipeline));
    let first = app.clone().oneshot(speech_request(Some(KEY), pcm("First.")).map(axum::body::Body::new)).await.unwrap();
    assert_eq!(first.status(), 200);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = app.clone().oneshot(speech_request(Some(KEY), pcm("Second.")).map(axum::body::Body::new)).await.unwrap();
    assert_eq!(second.status(), 429);
    let bytes = axum::body::to_bytes(first.into_body(), usize::MAX).await.unwrap();
    assert_eq!(samples(&bytes), expected_audio("First."));
    let third = app.oneshot(speech_request(Some(KEY), pcm("Third.")).map(axum::body::Body::new)).await.unwrap();
    assert_eq!(third.status(), 200);
}

#[tokio::test]
async fn shutdown_drops_connections_after_the_grace_period() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("worker.sock");
    let mut config = config(&socket);
    config.shutdown_grace_ms = 300;
    let _worker = AutoWorker::listen(&socket, Duration::from_millis(1));
    let worker = kokoro_server::Worker::spawn(config.worker_settings());
    worker.connected().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let limits = kokoro_server::serve::Limits::from(&config);
    let app = kokoro_server::http::router(config, worker, Arc::new(StubPipeline));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(kokoro_server::serve::serve(listener, app, limits, async {
        let _ = stopped.await;
    }));
    let _client = raw_speech(addr, "Hi. [pause:60s]").await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let started = Instant::now();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), server).await.expect("shutdown waited on a stuck client").unwrap();
    assert!(started.elapsed() >= Duration::from_millis(250));
}

/// Durations implying more audio than the worker's chunk cap close the connection, before any
/// audio is read.
#[tokio::test]
async fn durations_over_the_chunk_cap_close_the_connection() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let mut small = hello();
    small.max_chunk_samples = 600 * 3;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&small));
    server.worker.connected().await;
    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    assert!(ids.len() + 2 > 3);
    worker.send(worker_side::durations(id, &vec![1; ids.len() + 2]));
    assert_eq!(worker.next().await, Event::Closed);
    assert_eq!(client.await.unwrap().status(), 503);
}

/// The worker refusing a chunk as too long fails the request without closing the connection.
#[tokio::test]
async fn a_chunk_the_worker_finds_too_long_fails_the_request() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, ..) = worker.synthesize().await;
    worker.send(worker_side::failed(id, FailCode::TooLong));
    assert_eq!(client.await.unwrap().status(), 500);
    assert_eq!(worker.quiet(Duration::from_millis(100)).await, None, "the connection stays up");
    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.send(respond(id, &ids));
    assert_eq!(client.await.unwrap().status(), 200);
}
