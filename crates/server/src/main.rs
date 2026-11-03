#![forbid(unsafe_code)]

use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::{Config, Worker, http, serve};
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
    kokoro_server::log::init_from_env();
    let keys = std::env::var("KOKORO_API_KEYS").ok();
    let config = match config_path().and_then(|path| Config::load(path.as_deref(), keys.as_deref())) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match std::fs::read_to_string("/proc/self/limits").ok().as_deref().and_then(kokoro_server::config::soft_open_file_limit) {
        Some(soft) => {
            if let Err(e) = config.check_fd_limit(soft) {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
        None => kokoro_server::warn!("couldn't read the open-file limit; make sure it covers max_connections"),
    }
    let listener = match tokio::net::TcpListener::bind(config.listen).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("listening on {}: {e}", config.listen);
            return ExitCode::FAILURE;
        }
    };
    let pipelines = match KokoroPipelines::load() {
        Ok(pipelines) => pipelines,
        Err(e) => {
            eprintln!("loading the text pipelines: {e}");
            return ExitCode::FAILURE;
        }
    };
    let worker = Worker::spawn(config.worker_settings());
    kokoro_server::info!("listening on {}, worker socket {}", config.listen, config.worker_socket.display());
    let limits = serve::Limits::from(&config);
    let app = http::router(config, worker, Arc::new(pipelines));
    serve::serve(listener, app, limits, shutdown_signal()).await;
    ExitCode::SUCCESS
}
