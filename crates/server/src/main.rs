use kokoro_server::{Config, StubPipeline, Worker, http, serve};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

const USAGE: &str = "usage: kokoro-server [--config <file.toml>]\n\nAPI keys may also be given as KOKORO_API_KEYS (comma-separated).";

fn config_path() -> Result<Option<PathBuf>, String> {
    let mut args = std::env::args_os().skip(1);
    match (args.next(), args.next(), args.next()) {
        (None, _, _) => Ok(None),
        (Some(flag), Some(path), None) if flag == "--config" => Ok(Some(path.into())),
        _ => Err(USAGE.into()),
    }
}

async fn shutdown_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let keys = std::env::var("KOKORO_API_KEYS").ok();
    let config = match config_path().and_then(|path| Config::load(path.as_deref(), keys.as_deref())) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let listener = match tokio::net::TcpListener::bind(config.listen).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("listening on {}: {e}", config.listen);
            return ExitCode::FAILURE;
        }
    };
    tracing::warn!("using the stub text pipeline: audio will not be speech until kokoro-pipeline is wired in");
    let worker = Worker::spawn(config.worker_socket.clone(), config.reconnect_interval());
    tracing::info!("listening on {}, worker socket {}", config.listen, config.worker_socket.display());
    let limits = serve::Limits::from(&config);
    let app = http::router(config, worker, Arc::new(StubPipeline));
    serve::serve(listener, app, limits, shutdown_signal()).await;
    ExitCode::SUCCESS
}
