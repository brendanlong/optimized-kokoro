//! Settings, from an optional TOML file plus `KOKORO_API_KEYS` (comma-separated) from the
//! environment.

use crate::worker;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Address the HTTP server listens on.
    pub listen: SocketAddr,
    /// The model worker's Unix socket.
    pub worker_socket: PathBuf,
    /// Bearer tokens accepted on `/v1/*`. At least one is required.
    pub api_keys: Vec<String>,
    /// Voice used when a request doesn't name one.
    pub default_voice: String,
    /// Longest `input`, in characters, checked before any text processing.
    pub max_input_chars: usize,
    /// Largest request body. Defaults to enough for `max_input_chars` characters, each written as
    /// a JSON escaped surrogate pair, plus 16 KiB.
    pub max_body_bytes: Option<usize>,
    /// Speech requests served at once; more get 503.
    pub max_concurrent_requests: usize,
    /// Speech requests one API key may have in progress; more get 429.
    pub max_streams_per_key: usize,
    /// Open client connections. Further connections wait in the listen backlog.
    pub max_connections: usize,
    /// Requests served on one connection before it's closed.
    pub max_requests_per_connection: usize,
    /// Time a client has to send a request's headers, which also closes idle keep-alive
    /// connections.
    pub header_read_timeout_ms: u64,
    /// Time a client has to send a request's body.
    pub request_body_timeout_ms: u64,
    /// Chunks per request queued on or rendered by the worker, ahead of what the client has
    /// received. The first is the one the client is waiting for.
    pub lookahead: usize,
    /// Wait between attempts to (re)connect to the worker.
    pub reconnect_interval_ms: u64,
    /// Time the worker has to send its hello after connecting.
    pub worker_hello_timeout_ms: u64,
    /// Longest the worker may go without sending a whole frame while it has work; then it's
    /// disconnected and its requests fail.
    pub worker_progress_timeout_ms: u64,
    /// Requests on the worker at once, counting cancelled ones until the worker acknowledges
    /// them. Defaults to `max_concurrent_requests × lookahead`.
    pub worker_max_outstanding: Option<usize>,
    /// Audio held in memory across all requests (rendered, reordered or being written), in
    /// bytes. Each chunk reserves the most it could produce before it's sent to the worker.
    pub audio_buffer_bytes: usize,
    /// On shutdown, how long open connections get to finish before they're dropped.
    pub shutdown_grace_ms: u64,
    /// Whether requests may set `allow_voice_tags`.
    pub enable_voice_tags: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8880)),
            worker_socket: PathBuf::from("/run/kokoro/worker.sock"),
            api_keys: Vec::new(),
            default_voice: "af_heart".into(),
            max_input_chars: 20_000,
            max_body_bytes: None,
            max_concurrent_requests: 64,
            max_streams_per_key: 32,
            max_connections: 256,
            max_requests_per_connection: 1000,
            header_read_timeout_ms: 10_000,
            request_body_timeout_ms: 10_000,
            worker_hello_timeout_ms: 30_000,
            worker_progress_timeout_ms: 30_000,
            worker_max_outstanding: None,
            audio_buffer_bytes: 512 << 20,
            shutdown_grace_ms: 10_000,
            lookahead: 3,
            reconnect_interval_ms: 500,
            enable_voice_tags: true,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>, env_keys: Option<&str>) -> Result<Self, String> {
        let mut config: Config = match path {
            Some(path) => {
                let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
                toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
            }
            None => Config::default(),
        };
        if let Some(keys) = env_keys {
            config.api_keys.extend(keys.split(',').map(str::trim).filter(|k| !k.is_empty()).map(String::from));
        }
        config.check()?;
        Ok(config)
    }

    pub fn check(&self) -> Result<(), String> {
        if self.api_keys.is_empty() {
            return Err("no API keys: set api_keys in the config file or KOKORO_API_KEYS".into());
        }
        if self.api_keys.iter().any(String::is_empty) {
            return Err("an API key is empty".into());
        }
        let counts = [
            ("lookahead", self.lookahead),
            ("max_concurrent_requests", self.max_concurrent_requests),
            ("max_streams_per_key", self.max_streams_per_key),
            ("max_connections", self.max_connections),
            ("max_requests_per_connection", self.max_requests_per_connection),
            ("worker_max_outstanding", self.worker_max_outstanding()),
            ("audio_buffer_bytes", self.audio_buffer_bytes),
        ];
        if let Some((name, _)) = counts.iter().find(|(_, n)| *n == 0) {
            return Err(format!("{name} must be at least 1"));
        }
        if let Some((name, _)) = counts.iter().find(|(_, n)| *n > Semaphore::MAX_PERMITS) {
            return Err(format!("{name} is too large"));
        }
        let timeouts = [
            self.header_read_timeout_ms,
            self.request_body_timeout_ms,
            self.worker_hello_timeout_ms,
            self.worker_progress_timeout_ms,
        ];
        if timeouts.contains(&0) {
            return Err("timeouts must be positive".into());
        }
        Ok(())
    }

    pub fn body_limit(&self) -> usize {
        self.max_body_bytes.unwrap_or(self.max_input_chars * 12 + 16 * 1024)
    }

    pub fn header_read_timeout(&self) -> Duration {
        Duration::from_millis(self.header_read_timeout_ms)
    }

    pub fn request_body_timeout(&self) -> Duration {
        Duration::from_millis(self.request_body_timeout_ms)
    }

    pub fn reconnect_interval(&self) -> Duration {
        Duration::from_millis(self.reconnect_interval_ms)
    }

    pub fn worker_max_outstanding(&self) -> usize {
        self.worker_max_outstanding.unwrap_or(self.max_concurrent_requests.saturating_mul(self.lookahead))
    }

    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_millis(self.shutdown_grace_ms)
    }

    pub fn worker_settings(&self) -> worker::Settings {
        worker::Settings {
            socket: self.worker_socket.clone(),
            retry_interval: self.reconnect_interval(),
            hello_timeout: Duration::from_millis(self.worker_hello_timeout_ms),
            progress_timeout: Duration::from_millis(self.worker_progress_timeout_ms),
            max_outstanding: self.worker_max_outstanding(),
            audio_budget_bytes: self.audio_buffer_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_toml_and_env_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        std::fs::write(&path, "listen = \"0.0.0.0:9000\"\napi_keys = [\"a\"]\nlookahead = 2\n").unwrap();
        let config = Config::load(Some(&path), Some("b, c,")).unwrap();
        assert_eq!(config.listen.port(), 9000);
        assert_eq!(config.api_keys, ["a", "b", "c"]);
        assert_eq!(config.lookahead, 2);
        assert_eq!(config.body_limit(), 20_000 * 12 + 16 * 1024);
    }

    #[test]
    fn refuses_unknown_keys_and_missing_api_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        std::fs::write(&path, "api_keys = [\"a\"]\nlisten_addr = \"x\"\n").unwrap();
        assert!(Config::load(Some(&path), None).is_err());
        assert!(Config::load(None, None).is_err());
        assert!(Config::load(None, Some(" , ")).is_err());
    }
}
