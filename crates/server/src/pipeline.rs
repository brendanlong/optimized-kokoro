//! The text side as the server sees it: text in, chunks of phoneme ids out.
//!
//! `kokoro-pipeline` (normalization, G2P, chunking) is being written separately. This trait is
//! the narrow surface the server needs from it, so swapping it in means one adapter.

use kokoro_text::{Limits, Options, Piece, smart_split};
use std::fmt;

/// One chunk of a request's text: either speech, or a pause from a `[pause:Xs]` tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub text: String,
    pub phonemes: String,
    /// Model token ids for `phonemes`, without the boundary ids the worker adds.
    pub ids: Vec<u8>,
    /// Seconds of silence, for a pause chunk.
    pub pause: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineError {
    /// A sentence in a language with no G2P yet (Chinese or Japanese). The server skips it.
    Unsupported(String),
    /// Anything else, which fails the request.
    Failed(String),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(why) | Self::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for PipelineError {}

pub trait Pipeline: Send + Sync + 'static {
    /// Whether `lang` (a Kokoro language code such as `a` or `en-us`) has a pipeline.
    fn supports(&self, lang: &str) -> bool;

    /// The chunks of `text`, phonemized lazily as they're taken. Runs on a blocking thread. An
    /// error stands in for the sentence it happened in; later sentences still follow.
    fn chunks<'a>(
        &'a self,
        lang: &'a str,
        text: &'a str,
        options: &'a Options,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a>;
}

/// Kokoro's language codes and their aliases (`kokoro.pipeline.LANG_CODES`).
pub const LANG_CODES: &[&str] =
    &["a", "b", "e", "f", "h", "i", "p", "j", "z", "en-us", "en-gb", "es", "fr-fr", "hi", "it", "pt-br", "ja", "zh"];

/// A test pipeline: Kokoro-FastAPI's chunking (from `kokoro-text`) with a fake G2P that keeps
/// ASCII letters, spaces and basic punctuation as their own phonemes. The audio it produces is
/// not speech; the server uses [`crate::kokoro::KokoroPipelines`].
pub struct StubPipeline;

const STUB_PUNCTUATION: &str = ";:,.!?";
const SPACE_ID: u8 = 16;

fn stub_g2p(text: &str) -> String {
    text.chars()
        .map(|c| c.to_ascii_lowercase())
        .filter(|&c| c.is_ascii_lowercase() || c == ' ' || STUB_PUNCTUATION.contains(c))
        .collect()
}

fn stub_id(c: char) -> u8 {
    match c {
        ' ' => SPACE_ID,
        'a'..='z' => 43 + (c as u8 - b'a'),
        _ => 1 + STUB_PUNCTUATION.find(c).expect("stub phonemes only") as u8,
    }
}

impl Pipeline for StubPipeline {
    fn supports(&self, lang: &str) -> bool {
        LANG_CODES.contains(&lang)
    }

    fn chunks<'a>(
        &'a self,
        lang: &'a str,
        text: &'a str,
        options: &'a Options,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        static LIMITS: std::sync::LazyLock<Limits> = std::sync::LazyLock::new(Limits::default);
        let split = smart_split(text, lang, options, &LIMITS, kokoro_text::DEFAULT_MAX_TOKENS, stub_g2p);
        Box::new(split.map(|chunk| {
            Ok(match chunk {
                kokoro_text::Chunk::Speech(Piece { text, phonemes }) => {
                    let ids = phonemes.chars().map(stub_id).collect();
                    Chunk { text, phonemes, ids, pause: None }
                }
                kokoro_text::Chunk::Pause(seconds) => {
                    Chunk { text: String::new(), phonemes: String::new(), ids: Vec::new(), pause: Some(seconds) }
                }
            })
        }))
    }
}
