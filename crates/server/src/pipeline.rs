//! The text side as the server sees it: text in, chunks of phoneme ids out.
//!
//! [`crate::kokoro::KokoroPipelines`] is the real one; this trait is the narrow surface the server
//! needs from it, so tests can stand in their own.

use kokoro_text::{Limits, Options, Piece, smart_split};
use std::fmt;

pub use kokoro_pipeline::Lang;

/// The language a voice speaks, by Kokoro's convention that a voice name's first letter is its
/// language code (`af_heart` is American English). Only English pipelines exist so far.
pub fn voice_lang(voice: &str) -> Option<Lang> {
    voice.get(..1).and_then(Lang::from_code)
}

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
    /// The chunks of `text`, normalized with the default options and phonemized lazily as they're
    /// taken. Runs on a blocking thread. An error stands in for the sentence it happened in; later
    /// sentences still follow.
    fn chunks<'a>(&'a self, lang: Lang, text: &'a str) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a>;
}

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
    fn chunks<'a>(&'a self, lang: Lang, text: &'a str) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        static LIMITS: std::sync::LazyLock<Limits> = std::sync::LazyLock::new(Limits::default);
        static OPTIONS: std::sync::LazyLock<Options> = std::sync::LazyLock::new(Options::default);
        let split = smart_split(text, lang.code(), &OPTIONS, &LIMITS, kokoro_text::DEFAULT_MAX_TOKENS, stub_g2p);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_speaks_the_language_of_its_first_letter() {
        assert_eq!(voice_lang("af_heart"), Some(Lang::AmericanEnglish));
        assert_eq!(voice_lang("bm_george"), Some(Lang::BritishEnglish));
        assert_eq!(voice_lang("jf_alpha"), None);
        assert_eq!(voice_lang(""), None);
    }
}
