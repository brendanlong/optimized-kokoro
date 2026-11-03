//! The HTTP front of optimized-kokoro: it owns every byte from the network (request parsing,
//! text processing, scheduling, PCM streaming) and sends the sandboxed GPU worker only validated
//! integer frames over a Unix socket.

pub mod audio;
pub mod config;
pub mod http;
pub mod pipeline;
pub mod protocol;
pub mod request;
pub mod stitch;
pub mod stream;
pub mod voices;
pub mod worker;

pub use config::Config;
pub use pipeline::{Chunk, Pipeline, PipelineError, StubPipeline};
pub use worker::Worker;
