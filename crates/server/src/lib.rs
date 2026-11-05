//! The HTTP front of optimized-kokoro: it owns every byte from the network (request parsing,
//! text processing, scheduling, PCM streaming) and sends the sandboxed GPU worker only validated
//! integer frames over a Unix socket.
//!
//! **CPU requirement:** the repo builds for `x86-64-v3` (`.cargo/config.toml`), so the binary
//! needs AVX2 and FMA: Intel Haswell or AMD Zen 1 or newer. Older x86-64 CPUs stop it with an
//! illegal-instruction signal. This is Brendan's decision (2026-11-03, PR #13): it lets the
//! spaCy tagger's matmul be safe Rust as fast as the `unsafe` sgemm it replaced, so none of our
//! crates contains `unsafe`.
#![forbid(unsafe_code)]

pub mod audio;
pub mod config;
pub mod http;
pub mod kokoro;
pub mod load;
pub mod log;
pub mod merge;
pub mod pipeline;
pub mod protocol;
pub mod request;
pub mod serve;
pub mod stitch;
pub mod stream;
pub mod worker;

pub use config::Config;
pub use pipeline::{Chunk, Pipeline, PipelineError, StubPipeline};
pub use worker::Worker;
