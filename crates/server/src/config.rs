//! Settings, from an optional TOML file plus `KOKORO_API_KEYS` (comma-separated) from the
//! environment.

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    /// Chunks per request queued on or rendered by the worker, ahead of what the client has
    /// received. The first is the one the client is waiting for.
    pub lookahead: usize,
    /// Wait between attempts to (re)connect to the worker.
    pub reconnect_interval_ms: u64,
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
        if self.lookahead == 0 || self.max_concurrent_requests == 0 {
            return Err("lookahead and max_concurrent_requests must be at least 1".into());
        }
        Ok(())
    }

    pub fn body_limit(&self) -> usize {
        self.max_body_bytes.unwrap_or(self.max_input_chars * 12 + 16 * 1024)
    }

    pub fn reconnect_interval(&self) -> Duration {
        Duration::from_millis(self.reconnect_interval_ms)
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
