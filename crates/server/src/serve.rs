//! The HTTP/1 connection loop, with limits on what each client connection may hold: how many
//! connections are open, how long headers may take (which also ends idle keep-alive
//! connections), and how many requests one connection serves.

use crate::config::Config;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, header};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tower::ServiceExt;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_connections: usize,
    pub max_requests_per_connection: usize,
    pub header_read_timeout: Duration,
}

impl From<&Config> for Limits {
    fn from(config: &Config) -> Self {
        Self {
            max_connections: config.max_connections,
            max_requests_per_connection: config.max_requests_per_connection,
            header_read_timeout: config.header_read_timeout(),
        }
    }
}

/// Serves `app` until `shutdown` resolves, then lets open connections finish their requests.
pub async fn serve(listener: TcpListener, app: Router, limits: Limits, shutdown: impl Future<Output = ()>) {
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let (closing_tx, closing_rx) = watch::channel(false);
    tokio::pin!(shutdown);
    loop {
        // Waiting for a slot before accepting leaves extra connections in the listen backlog.
        let permit = tokio::select! {
            _ = &mut shutdown => break,
            permit = slots.clone().acquire_owned() => permit.expect("never closed"),
        };
        let stream = tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // Typically out of file descriptors; accepting again at once would spin.
                    tracing::warn!("accepting a connection: {e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            },
        };
        let app = app.clone();
        let mut closing = closing_rx.clone();
        tokio::spawn(async move {
            let served = Arc::new(AtomicUsize::new(0));
            let service = hyper::service::service_fn(move |request: Request<Incoming>| {
                let app = app.clone();
                let last = served.fetch_add(1, Ordering::Relaxed) + 1 >= limits.max_requests_per_connection;
                async move {
                    let mut response = app.oneshot(request.map(Body::new)).await?;
                    if last {
                        response.headers_mut().insert(header::CONNECTION, HeaderValue::from_static("close"));
                    }
                    Ok::<_, Infallible>(response)
                }
            });
            let mut builder = http1::Builder::new();
            builder.timer(TokioTimer::new()).header_read_timeout(limits.header_read_timeout);
            let connection = builder.serve_connection(TokioIo::new(stream), service);
            tokio::pin!(connection);
            let result = tokio::select! {
                result = connection.as_mut() => result,
                _ = async { closing.wait_for(|&c| c).await.map(|_| ()) } => {
                    connection.as_mut().graceful_shutdown();
                    connection.await
                }
            };
            if let Err(e) = result {
                tracing::debug!("connection ended: {e}");
            }
            drop(permit);
        });
    }
    closing_tx.send_replace(true);
    let _ = slots.acquire_many(limits.max_connections as u32).await;
}
