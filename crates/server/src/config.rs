//! Settings, from an optional TOML file plus `KOKORO_API_KEYS` (comma-separated) from the
//! environment.
//!
//! The limits here bound what one client can make the server hold or do. They are not a defence
//! against a flood of unauthenticated connections: when the server is reachable from the
//! internet, put it behind an edge that authenticates or filters first (Cloudflare Access, or the
//! relay in `docs/plan.md`), so only our clients reach it. Without that, the connection cap and
//! the header timeout only make such a flood expensive (see [`Config::max_connections`]).

use crate::worker;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Semaphore;

/// hyper's default write-buffer limit: it polls the body for more frames until this much is queued.
const HYPER_QUEUE_BYTES: usize = 8192 + 4096 * 100;

/// The soft "Max open files" limit from Linux's `/proc/self/limits`; `u64::MAX` if unlimited.
pub fn soft_open_file_limit(limits: &str) -> Option<u64> {
    let line = limits.lines().find(|l| l.starts_with("Max open files"))?;
    match line["Max open files".len()..].split_whitespace().next()? {
        "unlimited" => Some(u64::MAX),
        soft => soft.parse().ok(),
    }
}

/// An API key, how many speech requests it may have in progress at once, and the highest
/// priority its requests may run at.
///
/// In the TOML file, either a string or a table:
/// `api_keys = ["k1", { key = "k2", max_streams = 4, max_priority = 100 }]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKey {
    pub key: String,
    /// Overrides [`Config::max_streams_per_key`] for this key.
    pub max_streams: Option<usize>,
    /// Overrides [`Config::max_priority_per_key`] for this key.
    pub max_priority: Option<u8>,
}

impl From<&str> for ApiKey {
    fn from(key: &str) -> Self {
        key.to_owned().into()
    }
}

impl From<String> for ApiKey {
    fn from(key: String) -> Self {
        Self { key, max_streams: None, max_priority: None }
    }
}

impl<'de> Deserialize<'de> for ApiKey {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Table {
            key: String,
            max_streams: Option<usize>,
            max_priority: Option<u8>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Plain(String),
            Table(Table),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Plain(key) => key.into(),
            Raw::Table(Table { key, max_streams, max_priority }) => Self { key, max_streams, max_priority },
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    /// Address the HTTP server listens on.
    pub listen: SocketAddr,
    /// The model worker's Unix socket.
    pub worker_socket: PathBuf,
    /// Bearer tokens accepted on `/v1/*`, each optionally with its own stream limit. At least one
    /// is required.
    pub api_keys: Vec<ApiKey>,
    /// Voice used when a request doesn't name one.
    pub default_voice: String,
    /// Longest `input`, in characters, checked before any text processing.
    pub max_input_chars: usize,
    /// Largest request body. Defaults to enough for `max_input_chars` characters, each written as
    /// a JSON escaped surrogate pair, plus 16 KiB.
    pub max_body_bytes: Option<usize>,
    /// Speech requests served at once; more get 503, unless a more urgent request can displace a
    /// queued one. With `lookahead`, this bounds the audio the server can hold: see
    /// [`Config::audio_bound`]. The default, 48 (16 until 2026-11-04, Brendan's decision on PR
    /// #38), leans on latency-based shedding to keep first audio fast, and keeps the worst case
    /// inside the server container's 3 GiB with room to spare.
    pub max_concurrent_requests: usize,
    /// Speech requests one API key may have in progress, unless the key sets its own
    /// `max_streams`; more get 429. The default equals `max_concurrent_requests`: a key belongs to
    /// one of our own clients (Lion Reader, clawed-abode), which we trust not to attack the
    /// server, so this only keeps one client's bug from taking every stream. Lower it per key to
    /// reserve capacity for the other client.
    pub max_streams_per_key: usize,
    /// The priority of a speech request that doesn't set `priority` (0..=255, higher more
    /// urgent), or the key's maximum if that is lower. Default 128, the middle, so a client can
    /// mark requests as more or less urgent than one that doesn't say.
    pub default_priority: u8,
    /// The highest `priority` one API key's requests run at, unless the key sets its own
    /// `max_priority`; a request above it runs at it. The default, 255, lets our own
    /// clients use every level; lower it per key to keep one client below another.
    pub max_priority_per_key: u8,
    /// Shed load when the estimated time to a new request's first audio is over this many
    /// milliseconds: requests that haven't sent audio yet are refused (503 with `Retry-After`),
    /// lowest priority first and newest first within a priority (`crate::load`). The default,
    /// 800 ms, is under the ~1 s first-audio latency of the clients' fallback provider, so a
    /// shed client still gets audio sooner elsewhere. 0 turns latency-based shedding off.
    pub max_estimated_latency_ms: u64,
    /// Open client connections. Further connections wait in the listen backlog. An idle
    /// connection costs a file descriptor and a few KB, so the default is high (4096): a client
    /// holding connections open without sending a request (each for up to the header timeout,
    /// then reconnecting) needs this many sockets to starve `/health` and the API. The server
    /// refuses to start if the open-file limit can't cover it.
    pub max_connections: usize,
    /// Requests served on one connection before it's closed.
    pub max_requests_per_connection: usize,
    /// Time a client has to send a request's headers, which also closes idle keep-alive
    /// connections.
    pub header_read_timeout_ms: u64,
    /// Time a client has to send a request's body.
    pub request_body_timeout_ms: u64,
    /// Longest a response write may make no progress (the client not reading) before the
    /// connection is closed, which frees the request's permits and audio. A client that reads
    /// slowly, even at playback speed, keeps making progress and isn't affected.
    pub response_write_timeout_ms: u64,
    /// Most phoneme ids one speech request may send to the worker, stitched pieces' overlapping
    /// context included. The input limit counts characters, not work: 20,000 characters of long
    /// numbers become hundreds of thousands of ids (hours of audio). Past this, the response ends
    /// with an error. The default, 60,000, is roughly an hour of speech; ordinary text of
    /// `max_input_chars` characters (about one id per character) stays well under it.
    pub max_request_ids: usize,
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
    /// Longest one slot may stay outstanding after its Synthesize, queueing included, before it's
    /// cancelled and its request fails (a low-priority chunk can starve behind more urgent work,
    /// and a worker answering other slots mustn't hold one forever). If the slot is still
    /// outstanding a progress timeout later, the connection is reset. Legitimate waits at
    /// one priority are bounded by the slots ahead of it times the longest render (a 60 s chunk
    /// renders in a few seconds), so the default, 5 minutes, leaves a wide margin.
    pub worker_slot_timeout_ms: u64,
    /// Worker slots the server uses: requests on the worker at once, counting cancelled ones until
    /// the worker acknowledges them. Defaults to `max_concurrent_requests × lookahead`; the
    /// worker's hello may lower it.
    pub worker_slots: Option<u16>,
    /// The largest per-chunk audio cap (samples, at any speed) a worker may announce in its hello.
    /// It bounds the audio the server holds (see [`Config::audio_bound`]); a worker announcing
    /// more is refused. Default 60 s, the worker's own default since 2026-11-04 (it was 120 s).
    pub worker_max_chunk_samples: u32,
    /// On shutdown, how long open connections get to finish before they're dropped.
    pub shutdown_grace_ms: u64,
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
            max_concurrent_requests: 48,
            max_streams_per_key: 48,
            default_priority: 128,
            max_priority_per_key: 255,
            max_estimated_latency_ms: 800,
            max_connections: 4096,
            max_requests_per_connection: 1000,
            header_read_timeout_ms: 5_000,
            request_body_timeout_ms: 10_000,
            response_write_timeout_ms: 30_000,
            max_request_ids: 60_000,
            worker_hello_timeout_ms: 30_000,
            worker_progress_timeout_ms: 30_000,
            worker_slot_timeout_ms: 300_000,
            worker_slots: None,
            worker_max_chunk_samples: 1_440_000,
            shutdown_grace_ms: 10_000,
            lookahead: 3,
            reconnect_interval_ms: 500,
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
            config.api_keys.extend(keys.split(',').map(str::trim).filter(|k| !k.is_empty()).map(ApiKey::from));
        }
        config.check()?;
        Ok(config)
    }

    pub fn check(&self) -> Result<(), String> {
        if self.api_keys.is_empty() {
            return Err("no API keys: set api_keys in the config file or KOKORO_API_KEYS".into());
        }
        if self.api_keys.iter().any(|k| k.key.is_empty()) {
            return Err("an API key is empty".into());
        }
        if let Some(i) =
            (0..self.api_keys.len()).find(|&i| !(1..=Semaphore::MAX_PERMITS).contains(&self.max_streams(i)))
        {
            return Err(format!("API key {} (by position): max_streams must be at least 1", i + 1));
        }
        let counts = [
            ("lookahead", self.lookahead),
            ("max_concurrent_requests", self.max_concurrent_requests),
            ("max_streams_per_key", self.max_streams_per_key),
            ("max_connections", self.max_connections),
            ("max_requests_per_connection", self.max_requests_per_connection),
            ("worker_slots", self.worker_slots().into()),
            ("worker_max_chunk_samples", self.worker_max_chunk_samples as usize),
            ("max_request_ids", self.max_request_ids),
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
            self.response_write_timeout_ms,
            self.worker_hello_timeout_ms,
            self.worker_progress_timeout_ms,
            self.worker_slot_timeout_ms,
        ];
        if timeouts.contains(&0) {
            return Err("timeouts must be positive".into());
        }
        Ok(())
    }

    /// Speech requests the `i`th API key may have in progress.
    pub fn max_streams(&self, i: usize) -> usize {
        self.api_keys[i].max_streams.unwrap_or(self.max_streams_per_key)
    }

    /// The highest priority the `i`th API key's requests run at.
    pub fn max_priority(&self, i: usize) -> u8 {
        self.api_keys[i].max_priority.unwrap_or(self.max_priority_per_key)
    }

    /// The priority of the `i`th API key's requests that don't set one.
    pub fn default_priority(&self, i: usize) -> u8 {
        self.default_priority.min(self.max_priority(i))
    }

    /// The latency budget that sheds load; `None` if shedding is off.
    pub fn latency_budget(&self) -> Option<Duration> {
        (self.max_estimated_latency_ms > 0).then(|| Duration::from_millis(self.max_estimated_latency_ms))
    }

    pub fn response_write_timeout(&self) -> Duration {
        Duration::from_millis(self.response_write_timeout_ms)
    }

    /// File descriptors the server needs besides client connections: the listener, the worker
    /// socket, stdio, the runtime's own, with room to spare.
    pub const RESERVED_FDS: u64 = 64;

    /// Checks that the open-file limit (`soft`, from `RLIMIT_NOFILE`) covers every connection.
    pub fn check_fd_limit(&self, soft: u64) -> Result<(), String> {
        let needed = self.max_connections as u64 + Self::RESERVED_FDS;
        if soft < needed {
            return Err(format!(
                "max_connections = {} needs about {needed} open files, but the limit is {soft}: raise it \
                 (`ulimit -n`, or LimitNOFILE= in the systemd unit) or lower max_connections",
                self.max_connections
            ));
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

    pub fn worker_slots(&self) -> u16 {
        self.worker_slots.unwrap_or_else(|| {
            let wanted = self.max_concurrent_requests.saturating_mul(self.lookahead);
            u16::try_from(wanted).unwrap_or(u16::MAX)
        })
    }

    /// The most audio, in bytes, the server can hold at once when the worker's chunk cap is
    /// `max_chunk_samples` (at most `worker_max_chunk_samples`), at any speed. There is no byte
    /// budget; this bound comes from counts.
    ///
    /// Each speech request holds at most `lookahead + 2` chunks: `lookahead - 1` finished ones
    /// waiting in its window for an earlier one, one waiting to enter the response channel, one in
    /// the channel, and one hyper is writing (plus up to `HYPER_QUEUE_BYTES` of smaller frames
    /// hyper queues before it stops polling the body). A chunk is at most the chunk cap, or one
    /// pause. The worker connection reads one more Result at a time; audio for cancelled
    /// requests is dropped as it arrives. Worker slots don't add to this: a slot is freed when its
    /// Result arrives, and its audio then counts against its request.
    ///
    /// Defaults (48 requests, lookahead 3, the 60 s ceiling): a chunk is at most 2.88 MB, so
    /// 48 × (5 × 2.88 MB + 0.42 MB) + 2.88 MB = 714,134,016 bytes. (Until protocol v4 the cap
    /// scaled with 1/speed, and the worst case, at speed 0.25, was 2.80 GB.) The server's other
    /// memory is about 160 MB (measured: the nine text pipelines loaded, 32 requests in flight), so
    /// the worst case is about 875 MB, well inside the container's 3 GiB (`deploy/compose.yaml`).
    /// Real sentences are around 5 s (240 KB), so typical use holds a few MB.
    pub fn audio_bound(&self, max_chunk_samples: u32) -> usize {
        // A pause is one chunk of silence, at most the text limits' longest.
        let max_pause = crate::audio::silence(kokoro_text::Limits::default().max_pause_duration_s) as u64;
        let chunk_samples = u64::from(max_chunk_samples).max(max_pause);
        let chunk = usize::try_from(2 * chunk_samples).unwrap_or(usize::MAX);
        let per_stream = (self.lookahead + 2).saturating_mul(chunk).saturating_add(HYPER_QUEUE_BYTES);
        self.max_concurrent_requests.saturating_mul(per_stream).saturating_add(chunk)
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
            slots: self.worker_slots(),
            max_chunk_samples: self.worker_max_chunk_samples,
            slot_timeout: Duration::from_millis(self.worker_slot_timeout_ms),
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
        assert_eq!(config.api_keys, ["a", "b", "c"].map(ApiKey::from));
        assert_eq!(config.lookahead, 2);
        assert_eq!(config.body_limit(), 20_000 * 12 + 16 * 1024);
    }

    #[test]
    fn keys_may_set_their_own_stream_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let toml = "max_streams_per_key = 6\napi_keys = [\"plain\", { key = \"lion\", max_streams = 2 }]\n";
        std::fs::write(&path, toml).unwrap();
        let config = Config::load(Some(&path), Some("env")).unwrap();
        assert_eq!(config.api_keys[1], ApiKey { key: "lion".into(), max_streams: Some(2), max_priority: None });
        assert_eq!((0..3).map(|i| config.max_streams(i)).collect::<Vec<_>>(), [6, 2, 6]);
        for bad in [
            "api_keys = [{ key = \"k\", max_streams = 0 }]",
            "api_keys = [{ key = \"k\", streams = 1 }]",
            "api_keys = [{ key = \"k\", max_priority = 256 }]",
            "api_keys = [{ key = \"k\", max_priority = -1 }]",
        ] {
            std::fs::write(&path, bad).unwrap();
            assert!(Config::load(Some(&path), None).is_err(), "{bad}");
        }
    }

    #[test]
    fn keys_may_cap_their_priority() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.toml");
        let toml = "max_priority_per_key = 200\napi_keys = [\"plain\", { key = \"demo\", max_streams = 4, max_priority = 64 }, { key = \"top\", max_priority = 255 }]\n";
        std::fs::write(&path, toml).unwrap();
        let config = Config::load(Some(&path), None).unwrap();
        assert_eq!((0..3).map(|i| config.max_priority(i)).collect::<Vec<_>>(), [200, 64, 255]);
        assert_eq!((0..3).map(|i| config.default_priority(i)).collect::<Vec<_>>(), [128, 64, 128]);
        assert_eq!(Config::default().latency_budget(), Some(Duration::from_millis(800)));
        assert_eq!(Config { max_estimated_latency_ms: 0, ..Config::default() }.latency_budget(), None);
    }

    #[test]
    fn the_worst_case_audio_fits_the_server_container() {
        let config = Config::default();
        let worst = config.audio_bound(config.worker_max_chunk_samples);
        assert_eq!(worst, 714_134_016);
        // deploy/compose.yaml gives the server 3 GiB; the rest of the process is about 160 MB.
        const CONTAINER: usize = 3 << 30;
        const REST: usize = 160_000_000;
        assert!(worst + REST + 200_000_000 < CONTAINER, "{worst}");
    }

    #[test]
    fn defaults_resist_connection_exhaustion() {
        let config = Config::default();
        assert!(config.max_connections >= 4096);
        assert!(config.header_read_timeout() <= Duration::from_secs(5));
        assert!(config.check_fd_limit(1024).is_err());
        assert!(config.check_fd_limit(524_288).is_ok());
        let limits = "Limit                     Soft Limit           Hard Limit           Units\n\
                      Max processes             254641               254641               processes\n\
                      Max open files            1024                 524288               files\n";
        assert_eq!(soft_open_file_limit(limits), Some(1024));
        assert_eq!(soft_open_file_limit(&limits.replace("1024 ", "unlimited")), Some(u64::MAX));
        assert_eq!(soft_open_file_limit("nothing here"), None);
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
