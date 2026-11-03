//! misaki's Japanese G2P as kokoro uses it, `JAG2P()` (`version='cutlet'`): misaki's copy of
//! cutlet, on MeCab 0.996 with UniDic 3.1.0 (as fugashi runs it), with `num2kana` for numbers
//! and `ja_words.txt` to join MeCab's words.
//!
//! The dictionary is UniDic's compiled MeCab dictionary, converted by [`dict::build`] (see
//! `tools/fetch_unidic.sh`); it isn't in the repository. [`default_dictionary`] maps the file at
//! `$KOKORO_UNIDIC` the first time it's needed, once per process.
//!
//! Differences from the reference (all listed in `docs/divergences.md`): numbers of more than
//! nine digits, digits other than ASCII ones, and digit surfaces, where misaki raises, are read
//! as numbers; a kana before a small kana that cutlet has no reading for (`ーゃ`) is skipped
//! where misaki raises; and NULs, where MeCab stops reading, are dropped.
//!
//! The dictionary file is memory-mapped, so only the pages real text reads are resident. That
//! takes the crate's one `unsafe` call, `dict::map_read_only` (Brendan's exception to the
//! workspace's no-`unsafe` rule, 2026-11-03): the crate denies `unsafe_code` and allows it on that
//! function alone.
#![deny(unsafe_code)]

pub mod cutlet;
pub mod dict;
pub mod mecab;
pub mod num2kana;
mod tables;

pub use dict::{Dictionary, UNIDIC_VERSION};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, OnceLock};

/// Where [`default_dictionary`] looks: `$KOKORO_UNIDIC`. There is no built-in path, so a
/// deployed binary never depends on the build tree; `cargo run` and `cargo test` set it to
/// `crates/g2p-ja/data/unidic.dic` (`.cargo/config.toml`), where `tools/fetch_unidic.sh` puts it.
pub fn default_dictionary_path() -> Option<PathBuf> {
    std::env::var_os("KOKORO_UNIDIC").filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// The dictionary at [`default_dictionary_path`], loaded the first time it's asked for and kept
/// for the life of the process. A failure is kept too (installing the dictionary later takes a
/// restart), so a missing or broken file costs one attempt, not one per sentence; concurrent first
/// callers wait for the one load.
pub fn default_dictionary() -> Result<Arc<Dictionary>, dict::Error> {
    static DICT: OnceLock<Result<Arc<Dictionary>, String>> = OnceLock::new();
    DICT.get_or_init(|| {
        let path = default_dictionary_path().ok_or(
            "KOKORO_UNIDIC isn't set: the Japanese G2P needs UniDic (crates/g2p-ja/tools/fetch_unidic.sh writes it)",
        )?;
        Dictionary::load(&path).map(Arc::new).map_err(|e| match e {
            dict::Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound => format!(
                "{e}: the Japanese G2P needs UniDic at $KOKORO_UNIDIC (crates/g2p-ja/tools/fetch_unidic.sh writes it)"
            ),
            e => e.to_string(),
        })
    })
    .clone()
    .map_err(dict::Error::Format)
}

static WORDS: LazyLock<cutlet::Words> = LazyLock::new(cutlet::Words::bundled);

/// `JAG2P()`. It keeps no state between calls.
#[derive(Clone)]
pub struct JaG2p {
    dict: Arc<Dictionary>,
}

impl JaG2p {
    pub fn new(dict: Arc<Dictionary>) -> JaG2p {
        LazyLock::force(&WORDS);
        JaG2p { dict }
    }

    /// The dictionary at [`default_dictionary_path`].
    pub fn load_default() -> Result<JaG2p, dict::Error> {
        Ok(JaG2p::new(default_dictionary()?))
    }

    /// `JAG2P()(text)[0]`: the phonemes. Fails only where MeCab does, when a path's cost
    /// overflows (text of hundreds of thousands of words).
    pub fn phonemize(&self, text: &str) -> Result<String, mecab::Error> {
        cutlet::phonemize(&self.dict, &WORDS, text)
    }

    /// MeCab's words for `text` as given (without cutlet's normalization), for testing the
    /// analyzer.
    pub fn morphs<'a>(&'a self, text: &'a str) -> Result<Vec<mecab::Morph<'a>>, mecab::Error> {
        mecab::analyze(&self.dict, text)
    }

    pub fn dictionary(&self) -> &Dictionary {
        &self.dict
    }
}
