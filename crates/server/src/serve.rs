//! The HTTP/1 connection loop, with limits on what each client connection may hold: how many
//! connections are open, how long headers may take (which also ends idle keep-alive
//! connections), how long a response write may stall, how many requests one connection serves,
//! and how large a request head and hyper's buffers may be.

use crate::config::Config;
use crate::http::App;
use hyper::Request;
use hyper::header::{self, HeaderValue};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, ready};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Sleep;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

/// hyper's read buffer per connection (`max_buf_size`). hyper stops reading once the buffer holds
/// this much, but one read may fill whatever capacity the buffer already has, so a connection can
/// briefly buffer up to about twice this. It bounds how much of a body hyper reads ahead of the
/// app, including an unauthenticated one.
pub const MAX_BUF: usize = 64 * 1024;

/// The largest request head (request line and headers) accepted; larger ones get 431. hyper
/// checks the parsed head's length, so this holds exactly, whatever the read sizes.
pub const MAX_HEAD: usize = 16 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_connections: usize,
    pub max_requests_per_connection: usize,
    pub header_read_timeout: Duration,
    pub write_timeout: Duration,
    pub shutdown_grace: Duration,
}

impl From<&Config> for Limits {
    fn from(config: &Config) -> Self {
        Self {
            max_connections: config.max_connections,
            max_requests_per_connection: config.max_requests_per_connection,
            header_read_timeout: config.header_read_timeout(),
            write_timeout: config.response_write_timeout(),
            shutdown_grace: config.shutdown_grace(),
        }
    }
}

/// Serves `app` until `shutdown` resolves, then gives open connections `shutdown_grace` to finish
/// their requests before dropping them.
pub async fn serve(listener: TcpListener, app: App, limits: Limits, shutdown: impl Future<Output = ()>) {
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let (closing_tx, closing_rx) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        // Waiting for a slot before accepting leaves extra connections in the listen backlog.
        let permit = tokio::select! {
            _ = &mut shutdown => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => continue,
            permit = slots.clone().acquire_owned() => permit.expect("never closed"),
        };
        let stream = tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // Typically out of file descriptors; accepting again at once would spin.
                    crate::warn!("accepting a connection: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
        };
        let app = app.clone();
        let mut closing = closing_rx.clone();
        connections.spawn(async move {
            let served = Arc::new(AtomicUsize::new(0));
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let app = app.clone();
                let last = served.fetch_add(1, Ordering::Relaxed) + 1 >= limits.max_requests_per_connection;
                async move {
                    let mut response = app.handle(request).await;
                    if last {
                        response.headers_mut().insert(header::CONNECTION, HeaderValue::from_static("close"));
                    }
                    Ok::<_, Infallible>(response)
                }
            });
            let mut builder = http1::Builder::new();
            builder
                .timer(TokioTimer::new())
                .header_read_timeout(limits.header_read_timeout)
                .max_buf_size(MAX_BUF)
                .max_header_size(MAX_HEAD);
            let io = TokioIo::new(WriteDeadline::new(stream, limits.write_timeout));
            let connection = builder.serve_connection(io, service);
            tokio::pin!(connection);
            let result = tokio::select! {
                result = connection.as_mut() => result,
                _ = async { closing.wait_for(|&c| c).await.map(|_| ()) } => {
                    connection.as_mut().graceful_shutdown();
                    connection.await
                }
            };
            if let Err(e) = result {
                crate::debug!("connection ended: {e}");
            }
            drop(permit);
        });
    }
    closing_tx.send_replace(true);
    let drained = tokio::time::timeout(limits.shutdown_grace, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        crate::warn!("dropping {} connections still open after the shutdown grace period", connections.len());
        connections.shutdown().await;
    }
}

/// A client socket whose writes fail with `TimedOut` once one has made no progress for `timeout`:
/// a client that stops reading can't keep its response, and the permits and audio behind it,
/// forever. Any progress restarts the clock, so slow readers are fine.
struct WriteDeadline {
    stream: TcpStream,
    timeout: Duration,
    /// Running while a write or flush is waiting for the client.
    stalled: Option<Pin<Box<Sleep>>>,
}

impl WriteDeadline {
    fn new(stream: TcpStream, timeout: Duration) -> Self {
        Self { stream, timeout, stalled: None }
    }

    /// `poll` is the inner write; times it out if it has been pending for too long.
    fn deadline<T>(&mut self, cx: &mut Context<'_>, poll: Poll<io::Result<T>>) -> Poll<io::Result<T>> {
        if poll.is_ready() {
            self.stalled = None;
            return poll;
        }
        let timeout = self.timeout;
        let stalled = self.stalled.get_or_insert_with(|| Box::pin(tokio::time::sleep(timeout)));
        ready!(stalled.as_mut().poll(cx));
        Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "the client stopped reading the response")))
    }
}

impl AsyncRead for WriteDeadline {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for WriteDeadline {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.stream).poll_write(cx, buf);
        self.deadline(cx, poll)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let poll = Pin::new(&mut self.stream).poll_write_vectored(cx, bufs);
        self.deadline(cx, poll)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let poll = Pin::new(&mut self.stream).poll_flush(cx);
        self.deadline(cx, poll)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
