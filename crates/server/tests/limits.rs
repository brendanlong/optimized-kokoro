//! Limits on HTTP clients: header and body timeouts, connections, requests per connection and
//! streams per key.

mod common;

use common::*;
use kokoro_server::StubPipeline;
use kokoro_server::protocol::worker_side;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const HEALTH: &[u8] = b"GET /health HTTP/1.1\r\nHost: test\r\n\r\n";

/// Reads until the server closes the connection, returning what it sent.
async fn read_to_close(stream: &mut TcpStream, within: Duration) -> Vec<u8> {
    let mut out = Vec::new();
    tokio::time::timeout(within, stream.read_to_end(&mut out)).await.expect("connection stayed open").ok();
    out
}

async fn read_response(stream: &mut TcpStream) -> String {
    let mut buf = vec![0; 4096];
    let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await.unwrap().unwrap();
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[tokio::test]
async fn slow_headers_and_idle_connections_are_closed() {
    let server = start(|c| c.header_read_timeout_ms = 300, Arc::new(StubPipeline)).await;

    let mut slow = TcpStream::connect(server.addr).await.unwrap();
    slow.write_all(b"GET /health HTTP/1.1\r\nHost: te").await.unwrap();
    let started = Instant::now();
    read_to_close(&mut slow, Duration::from_secs(3)).await;
    assert!(started.elapsed() >= Duration::from_millis(250));

    let mut idle = TcpStream::connect(server.addr).await.unwrap();
    idle.write_all(HEALTH).await.unwrap();
    assert!(read_response(&mut idle).await.starts_with("HTTP/1.1 503"));
    let started = Instant::now();
    assert!(read_to_close(&mut idle, Duration::from_secs(3)).await.is_empty());
    assert!(started.elapsed() >= Duration::from_millis(250));
}

#[tokio::test]
async fn slow_bodies_time_out() {
    let (server, _worker) = {
        let server = start(|c| c.request_body_timeout_ms = 300, Arc::new(StubPipeline)).await;
        let worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
        server.worker.connected().await;
        (server, worker)
    };
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    let head = format!(
        "POST /v1/audio/speech HTTP/1.1\r\nHost: test\r\nAuthorization: Bearer {KEY}\r\nContent-Length: 100\r\n\r\n{{"
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    assert!(read_response(&mut stream).await.starts_with("HTTP/1.1 408"));
}

#[tokio::test]
async fn connections_beyond_the_cap_wait() {
    let server = start(|c| c.max_connections = 1, Arc::new(StubPipeline)).await;
    let mut first = TcpStream::connect(server.addr).await.unwrap();
    first.write_all(HEALTH).await.unwrap();
    assert!(read_response(&mut first).await.starts_with("HTTP/1.1"));

    let mut second = TcpStream::connect(server.addr).await.unwrap();
    second.write_all(HEALTH).await.unwrap();
    let mut buf = [0; 16];
    assert!(tokio::time::timeout(Duration::from_millis(300), second.read(&mut buf)).await.is_err());
    drop(first);
    assert!(read_response(&mut second).await.starts_with("HTTP/1.1"));
}

#[tokio::test]
async fn connections_close_after_their_request_limit() {
    let server = start(|c| c.max_requests_per_connection = 2, Arc::new(StubPipeline)).await;
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    stream.write_all(HEALTH).await.unwrap();
    let first = read_response(&mut stream).await;
    assert!(!first.to_ascii_lowercase().contains("connection: close"));
    stream.write_all(HEALTH).await.unwrap();
    let rest = read_to_close(&mut stream, Duration::from_secs(3)).await;
    let rest = String::from_utf8_lossy(&rest).to_ascii_lowercase();
    assert!(rest.starts_with("http/1.1 503") && rest.contains("connection: close"), "{rest}");
}

#[tokio::test]
async fn each_key_has_its_own_stream_limit() {
    let server = start(|c| c.max_streams_per_key = 1, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    server.worker.connected().await;

    let held = tokio::spawn(speak(server.addr, pcm("Hold on.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    let refused = speak(server.addr, pcm("Hi.")).await;
    assert_eq!(refused.status(), 429);
    assert_eq!(json_body(refused).await["detail"]["error"], "rate_limited");

    let other = tokio::spawn(request(server.addr, speech_request(Some("other-key"), pcm("Hi."))));
    let (other_id, _, _, _, other_ids) = worker.synthesize().await;
    worker.send(respond(other_id, &other_ids));
    assert_eq!(other.await.unwrap().status(), 200);

    worker.send(respond(id, &ids));
    let response = held.await.unwrap();
    body_bytes(response).await.unwrap();
    let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
    let (id, _, _, _, ids) = worker.synthesize().await;
    worker.send(respond(id, &ids));
    assert_eq!(client.await.unwrap().status(), 200);
}
