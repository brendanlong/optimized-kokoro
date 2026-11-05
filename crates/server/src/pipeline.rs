//! The text side as the server sees it: text in, chunks of phoneme ids out.
//!
//! [`crate::kokoro::KokoroPipelines`] is the real one; this trait is the narrow surface the server
//! needs from it, so tests can stand in their own.

use kokoro_text::{Limits, Options, Piece, smart_split};

pub use kokoro_text::{DEFAULT_MAX_TOKENS, Split};
use std::fmt;

pub use kokoro_pipeline::Lang;

/// The language a voice speaks, by Kokoro's convention that a voice name's first letter is its
/// language code (`af_heart` is American English, `ef_dora` Spanish, `jf_alpha` Japanese,
/// `zf_xiaobei` Chinese). Every Kokoro-82M voice's language has a pipeline.
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
    /// Runs of text with no G2P for their language or script yet, read as "<name> text".
    pub unread: usize,
    /// Which sides of the chunk were cut from its sentence at clause punctuation.
    pub split: Split,
}

/// Text processing failed, which fails the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineError(pub String);

impl fmt::Display for PipelineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PipelineError {}

pub trait Pipeline: Send + Sync + 'static {
    /// The chunks of `text`, normalized with the default options and phonemized lazily as they're
    /// taken. A sentence of more than `max_phonemes` phonemes is cut at clause punctuation
    /// (Kokoro-FastAPI's limit is [`DEFAULT_MAX_TOKENS`]). Runs on a blocking thread. An error
    /// stands in for the sentence it happened in; later sentences still follow.
    fn chunks<'a>(
        &'a self,
        lang: Lang,
        text: &'a str,
        max_phonemes: usize,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a>;
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
    fn chunks<'a>(
        &'a self,
        lang: Lang,
        text: &'a str,
        max_phonemes: usize,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        static LIMITS: std::sync::LazyLock<Limits> = std::sync::LazyLock::new(Limits::default);
        static OPTIONS: std::sync::LazyLock<Options> = std::sync::LazyLock::new(Options::default);
        let split = smart_split(text, lang.code(), &OPTIONS, &LIMITS, max_phonemes, stub_g2p);
        Box::new(split.map(|chunk| {
            Ok(match chunk {
                kokoro_text::Chunk::Speech(Piece { text, phonemes, split }) => {
                    let ids = phonemes.chars().map(stub_id).collect();
                    Chunk { text, phonemes, ids, pause: None, unread: 0, split }
                }
                kokoro_text::Chunk::Pause(seconds) => Chunk {
                    text: String::new(),
                    phonemes: String::new(),
                    ids: Vec::new(),
                    pause: Some(seconds),
                    unread: 0,
                    split: Split::default(),
                },
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
        assert_eq!(voice_lang("zf_xiaobei"), Some(Lang::Chinese));
        assert_eq!(voice_lang("jf_alpha"), Some(Lang::Japanese));
        assert_eq!(voice_lang("ef_dora"), Some(Lang::Spanish));
        assert_eq!(voice_lang("ff_siwis"), Some(Lang::French));
        assert_eq!(voice_lang("hm_omega"), Some(Lang::Hindi));
        assert_eq!(voice_lang("if_sara"), Some(Lang::Italian));
        assert_eq!(voice_lang("pm_santa"), Some(Lang::BrazilianPortuguese));
        assert_eq!(voice_lang("xf_nobody"), None);
        assert_eq!(voice_lang(""), None);
    }
}
