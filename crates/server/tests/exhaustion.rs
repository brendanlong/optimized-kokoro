//! Resource-exhaustion gaps found by the adversarial review of PR #13, one regression test each.
//! The reviewer's attack scripts are in `tools/security/`.

mod common;

use common::*;
use kokoro_server::StubPipeline;
use kokoro_server::config::ApiKey;
use kokoro_server::pipeline::{Chunk, Lang, Pipeline, PipelineError};
use kokoro_server::protocol::worker_side;
use kokoro_server::stitch::Plan;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn one_stream(c: &mut kokoro_server::Config) {
    c.max_concurrent_requests = 1;
    c.max_streams_per_key = 1;
}

/// The stub pipeline, except that the first request blocks before its second chunk until the
/// test releases it, as a long adversarial sentence keeps the phonemizer busy.
struct SlowSecondChunk {
    entered: AtomicBool,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl Pipeline for SlowSecondChunk {
    fn chunks<'a>(&'a self, lang: Lang, text: &'a str) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        Box::new(StubPipeline.chunks(lang, text).enumerate().map(move |(i, chunk)| {
            if i == 1
                && let Some(release) = self.release.lock().unwrap().take()
            {
                self.entered.store(true, Ordering::SeqCst);
                let _ = release.recv();
            }
            chunk
        }))
    }
}

/// Finding 1: a client that disconnects while its text is still being phonemized doesn't get
/// its permits back until the phonemizing stops.
#[tokio::test]
async fn permits_cover_phonemizing_after_the_client_leaves() {
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let pipeline = Arc::new(SlowSecondChunk { entered: AtomicBool::new(false), release: Mutex::new(Some(release_rx)) });
    let server = start(one_stream, pipeline.clone()).await;
    let _worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;

    let first = speak(server.addr, pcm("First part. Second part. Third part.")).await;
    assert_eq!(first.status(), 200);
    while !pipeline.entered.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(first);
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The client is gone, but its text is still being processed.
    assert_eq!(speak(server.addr, pcm("Again.")).await.status(), 429);

    release_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let response = speak(server.addr, pcm("Again.")).await;
        if response.status() == 200 {
            assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio("Again."));
            break;
        }
        assert!(Instant::now() < deadline, "permits never came back");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Finding 2: a client that stops reading is disconnected once a write has stalled for the
/// write timeout, which frees its permits.
#[tokio::test]
async fn a_client_that_stops_reading_is_disconnected() {
    let server = start(
        |c| {
            one_stream(c);
            c.response_write_timeout_ms = 300;
        },
        Arc::new(StubPipeline),
    )
    .await;
    let _worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut stalled = socket.connect(server.addr).await.unwrap();
    // A minute of silence (2.88 MB) can't fit in the socket buffers.
    let body = pcm("Hi. [pause:60s]").to_string();
    let head = format!(
        "POST /v1/audio/speech HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer {KEY}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stalled.write_all(format!("{head}{body}").as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(speak(server.addr, pcm("Second.")).await.status(), 429);

    tokio::time::sleep(Duration::from_millis(1000)).await;
    let response = speak(server.addr, pcm("Second.")).await;
    assert_eq!(response.status(), 200);
    assert_eq!(samples(&body_bytes(response).await.unwrap()), expected_audio("Second."));
    // And the stalled response was cut off, not completed.
    let mut received = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stalled.read_to_end(&mut received)).await;
    assert!(received.len() < 2_880_000, "the stalled response was written in full");
}

/// Finding 2: one key's stream limit can differ from another's.
#[tokio::test]
async fn keys_can_have_their_own_stream_limits() {
    let server = start(
        |c| c.api_keys = vec![ApiKey { key: KEY.into(), max_streams: Some(1), max_priority: None }, "other-key".into()],
        Arc::new(StubPipeline),
    )
    .await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    // Requests wait for their first audio, which this worker sends only when told.
    let held = tokio::spawn(speak(server.addr, pcm("Hold on.")));
    let first = worker.synthesize().await;
    let refused = speak(server.addr, pcm("Hi.")).await;
    assert_eq!(refused.status(), 429);
    assert!(json_body(refused).await["detail"]["message"].as_str().unwrap().contains("already has 1 speech"));
    let others: Vec<_> =
        (0..3).map(|_| tokio::spawn(request(server.addr, speech_request(Some("other-key"), pcm("Hi."))))).collect();
    let mut pending = vec![first];
    for _ in 0..3 {
        pending.push(worker.synthesize().await);
    }
    for (slot, _, _, _, ids) in pending {
        worker.send(respond(slot, &ids));
    }
    assert_eq!(held.await.unwrap().status(), 200);
    for other in others {
        assert_eq!(other.await.unwrap().status(), 200);
    }
}

/// Finding 3: hundreds of connections that never finish their headers don't starve `/health`.
#[tokio::test]
async fn trickling_connections_dont_starve_health() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let _worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;
    let mut idle = Vec::new();
    for _ in 0..600 {
        let mut stream = TcpStream::connect(server.addr).await.unwrap();
        stream.write_all(b"GET /hea").await.unwrap();
        idle.push(stream);
    }
    let health = tokio::time::timeout(Duration::from_secs(2), request(server.addr, get("/health", None))).await;
    assert_eq!(health.expect("/health answered").status(), 200);
}

/// The first frame the server sends within 200 ms, if any.
async fn no_frames(worker: &mut ScriptedWorker) -> Option<Event> {
    loop {
        match worker.quiet(Duration::from_millis(200)).await {
            Some(Event::Connected) => continue,
            other => return other,
        }
    }
}

/// Finding 4: the ids a request sends the worker are capped, counting stitched pieces with their
/// overlapping context, which is more than the chunk's own ids. Over the cap before any audio:
/// 400; after some audio: the response ends with an error.
#[tokio::test]
async fn phoneme_ids_per_request_are_capped_counting_stitched_context() {
    let long = (0..120).map(|i| format!("word{}", char::from(b'a' + (i % 26) as u8))).collect::<Vec<_>>().join(" ");
    let chunk = stub_chunks(&long).remove(0);
    let plan = Plan::new(chunk.ids.clone());
    let stitched: usize = (0..plan.pieces()).map(|i| plan.context_ids(i).len()).sum();
    assert!(plan.pieces() > 1 && stitched > chunk.ids.len());

    // The chunk's ids alone fit, the stitched pieces don't.
    let (server, mut worker) = {
        let server = start(|c| c.max_request_ids = chunk.ids.len(), Arc::new(StubPipeline)).await;
        let worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
        server.worker.connected().await;
        (server, worker)
    };
    let refused = speak(server.addr, pcm(&long)).await;
    assert_eq!(refused.status(), 400);
    assert!(json_body(refused).await["detail"]["message"].as_str().unwrap().contains("phonemes of speech"));
    assert_eq!(no_frames(&mut worker).await, None, "nothing reached the worker");

    // After a short first sentence, the stream ends with an error where the cap is crossed.
    // The pause keeps the short sentence a chunk of its own.
    let text = format!("Short one. [pause:0.1s] {long}");
    let chunks = stub_chunks(&text);
    let short = &chunks[0];
    assert_eq!(chunks.last().unwrap().ids, chunk.ids);
    let server = start(|c| c.max_request_ids = short.ids.len() + stitched - 1, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let client = tokio::spawn(speak(server.addr, pcm(&text)));
    let (slot, _, _, _, ids) = worker.synthesize().await;
    assert_eq!(ids, short.ids);
    worker.send(respond(slot, &ids));
    let response = client.await.unwrap();
    assert_eq!(response.status(), 200);
    assert!(body_bytes(response).await.is_err(), "the stream must end with an error");
    assert_eq!(no_frames(&mut worker).await, None, "no stitched piece was sent");

    // With room for every piece, it's spoken.
    let server = start(|c| c.max_request_ids = short.ids.len() + stitched, Arc::new(StubPipeline)).await;
    let _worker = AutoWorker::listen(&server.socket, Duration::from_millis(1));
    server.worker.connected().await;
    let response = speak(server.addr, pcm(&text)).await;
    assert_eq!(response.status(), 200);
    body_bytes(response).await.unwrap();
}

/// Finding 4 with the reviewer's input and the real pipeline: 20 KB of long negative numbers
/// (about 481k ids, 7 hours of audio) stops at the default cap.
#[tokio::test]
async fn the_reviewers_worst_input_stops_at_the_id_cap() {
    let body: serde_json::Value = serde_json::from_slice(
        &std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/security/worst.json")).unwrap(),
    )
    .unwrap();
    let pipelines = Arc::new(kokoro_server::kokoro::KokoroPipelines::load().unwrap());
    let server = start(|_| {}, pipelines).await;
    let worker = AutoWorker::listen(&server.socket, Duration::ZERO);
    server.worker.connected().await;
    let response = speak(server.addr, body).await;
    // Read without commas (the number reader puts none inside numbers), the input is one
    // sentence with no clause to cut it at, so its ids pass the cap before any audio: a 400.
    // With commas, as the reference read it, it streamed and then ended with an error.
    let rendered = worker.log.lock().unwrap().len();
    if response.status() == 400 {
        assert!(rendered * 510 <= 60_000, "{rendered} chunks rendered");
        return;
    }
    assert_eq!(response.status(), 200);
    assert!(body_bytes(response).await.is_err(), "the stream must end with an error");
    let rendered = worker.log.lock().unwrap().len();
    assert!((60_000 / 510..200).contains(&rendered), "{rendered} chunks rendered");
}

/// Finding 5: a worker that answers other slots but never one of them is disconnected once that
/// slot is overdue, and the request waiting on it fails.
#[tokio::test]
async fn a_slot_the_worker_never_answers_resets_the_connection() {
    let server = start(|c| c.worker_slot_timeout_ms = 300, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;
    let starved = tokio::spawn(speak(server.addr, pcm("Never answered.")));
    worker.synthesize().await;
    let started = Instant::now();
    // Keep answering other requests, so the worker never looks stalled.
    for _ in 0..3 {
        let other = tokio::spawn(speak(server.addr, pcm("Answered.")));
        let (slot, _, _, _, ids) = worker.synthesize().await;
        worker.send(respond(slot, &ids));
        assert_eq!(other.await.unwrap().status(), 200);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    loop {
        if worker.next().await == Event::Closed {
            break;
        }
    }
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(starved.await.unwrap().status(), 503);
}

/// Finding 7: the request head limit is what the docs say, however the bytes arrive.
#[tokio::test]
async fn request_heads_over_the_limit_are_refused() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    for (size, status) in [(8_000, "401"), (20_000, "431"), (100_000, "431")] {
        let mut stream = TcpStream::connect(server.addr).await.unwrap();
        let padding = "a".repeat(size);
        let head = format!("GET /v1/audio/voices HTTP/1.1\r\nHost: test\r\nX-Pad: {padding}\r\n\r\n");
        // One write, so the whole head can arrive in a single read.
        let _ = stream.write_all(head.as_bytes()).await;
        let mut response = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with(&format!("HTTP/1.1 {status}")), "{size} bytes: {response:.80}");
    }
}
