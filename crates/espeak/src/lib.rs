//! misaki 0.9.4's two espeak-ng wrappers (`misaki/espeak.py`), on `kokoro-espeak-ng` (a port of
//! espeak-ng 1.52's text-to-phoneme engine) instead of C espeak-ng:
//! - [`EspeakFallback`] reads English words missing from misaki's lexicons;
//! - [`EspeakG2P`] reads whole segments of es, fr-fr, hi, it, pt-br, and the runs of other scripts
//!   the English pipelines hand to espeak (kokoro's `foreign.py`: ar, el, he, hi, ko and ru; here
//!   also the languages espeak switches to for other alphabets, [`alphabet_languages`]).
//!
//! Both match the deployed pipeline, which passes no `version` to either, except that the fallback
//! drops espeak's language-switch flags as `EspeakG2P` does (`docs/divergences.md`).
#![forbid(unsafe_code)]

pub mod phonemizer;

use phonemizer::{Backend, py_strip};
use std::path::Path;
use std::sync::{Arc, OnceLock};

pub use kokoro_espeak_ng::{EspeakData, Error, Result, alphabet_languages, default_data_dir};

/// The data in [`default_data_dir`], loaded once per process.
pub fn default_data() -> Result<Arc<EspeakData>> {
    static DATA: OnceLock<Arc<EspeakData>> = OnceLock::new();
    if let Some(d) = DATA.get() {
        return Ok(d.clone());
    }
    let d = EspeakData::load(default_data_dir())?;
    Ok(DATA.get_or_init(|| d).clone())
}

/// `misaki.espeak.EspeakFallback`, the last resort for English words. Unlike misaki's, it drops
/// espeak's language-switch flags (`(ka)…(en-us)`) instead of reading them out as phonemes.
pub struct EspeakFallback {
    british: bool,
    backend: Backend,
    e2m: Vec<(&'static str, &'static str)>,
}

impl EspeakFallback {
    pub fn new(british: bool) -> Result<Self> {
        Self::with_data(british, default_data()?)
    }

    pub fn with_data_dir(british: bool, data_dir: &Path) -> Result<Self> {
        Self::with_data(british, EspeakData::load(data_dir)?)
    }

    pub fn with_data(british: bool, data: Arc<EspeakData>) -> Result<Self> {
        let language = if british { "en-gb" } else { "en-us" };
        let backend = Backend::new(language, data)?;
        // Python sorts the dict items by descending length, keeping insertion order among equals.
        let mut e2m = vec![
            ("ʔˌn\u{329}", "ʔn"),
            ("ʔn\u{329}", "ʔn"),
            ("a^ɪ", "I"),
            ("a^ʊ", "W"),
            ("d^ʒ", "ʤ"),
            ("e^ɪ", "A"),
            ("e", "A"),
            ("t^ʃ", "ʧ"),
            ("ɔ^ɪ", "Y"),
            ("ə^l", "ᵊl"),
            ("ʲo", "jo"),
            ("ʲə", "jə"),
            ("ʲ", ""),
            ("ɚ", "əɹ"),
            ("r", "ɹ"),
            ("x", "k"),
            ("ç", "k"),
            ("ɐ", "ə"),
            ("ɬ", "l"),
            ("\u{303}", ""),
        ];
        e2m.sort_by_key(|(old, _)| std::cmp::Reverse(old.chars().count()));
        Ok(Self { british, backend, e2m })
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Phonemes for one token's text, or `None` when espeak has nothing to say.
    pub fn phonemize(&self, word: &str) -> Result<Option<String>> {
        self.phonemize_with(word, |chunk| self.backend.espeak(chunk))
    }

    /// [`phonemize`](Self::phonemize) with `espeak` standing in for the engine.
    pub fn phonemize_with(
        &self,
        word: &str,
        espeak: impl FnMut(&str) -> Result<String>,
    ) -> Result<Option<String>> {
        let Some(first) = self.backend.phonemize_with(word, espeak)?.into_iter().next() else {
            return Ok(None);
        };
        let mut ps = py_strip(&first).to_string();
        for (old, new) in &self.e2m {
            ps = ps.replace(old, new);
        }
        ps = syllabic_to_schwa(&ps).replace('\u{329}', "");
        if self.british {
            ps = ps.replace("e^ə", "ɛː").replace("iə", "ɪə").replace("ə^ʊ", "Q");
        } else {
            ps = ps
                .replace("o^ʊ", "O")
                .replace("ɜːɹ", "ɜɹ")
                .replace("ɜː", "ɜɹ")
                .replace("ɪə", "iə")
                .replace('ː', "");
        }
        ps = ps.replace('o', "ɔ").replace('ɾ', "T").replace('ʔ', "t").replace('^', "");
        Ok(Some(ps))
    }
}

/// `re.sub(r'(\S)̩', r'ᵊ\1', ps)`.
fn syllabic_to_schwa(ps: &str) -> String {
    let mut out = String::with_capacity(ps.len() + 4);
    let mut chars = ps.chars().peekable();
    while let Some(c) = chars.next() {
        if !phonemizer::py_isspace(c) && chars.peek() == Some(&'\u{329}') {
            chars.next();
            out.push('ᵊ');
        }
        out.push(c);
    }
    out
}

/// `misaki.espeak.EspeakG2P(language)`, which reads whole segments.
pub struct EspeakG2P {
    backend: Backend,
    e2m: Vec<(&'static str, &'static str)>,
}

impl EspeakG2P {
    pub fn new(language: &str) -> Result<Self> {
        Self::with_data(language, default_data()?)
    }

    pub fn with_data_dir(language: &str, data_dir: &Path) -> Result<Self> {
        Self::with_data(language, EspeakData::load(data_dir)?)
    }

    pub fn with_data(language: &str, data: Arc<EspeakData>) -> Result<Self> {
        let backend = Backend::new(language, data)?;
        let mut e2m = vec![
            ("a^ɪ", "I"),
            ("a^ʊ", "W"),
            ("d^z", "ʣ"),
            ("d^ʒ", "ʤ"),
            ("e^ɪ", "A"),
            ("o^ʊ", "O"),
            ("ə^ʊ", "Q"),
            ("s^s", "S"),
            ("t^s", "ʦ"),
            ("t^ʃ", "ʧ"),
            ("ɔ^ɪ", "Y"),
        ];
        e2m.sort();
        Ok(Self { backend, e2m })
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    pub fn phonemize(&self, text: &str) -> Result<String> {
        self.phonemize_with(text, |chunk| self.backend.espeak(chunk))
    }

    /// [`phonemize`](Self::phonemize) with `espeak` standing in for the engine.
    pub fn phonemize_with(&self, text: &str, espeak: impl FnMut(&str) -> Result<String>) -> Result<String> {
        let text = text
            .replace('«', "\u{201c}")
            .replace('»', "\u{201d}")
            .replace('(', "«")
            .replace(')', "»");
        let Some(first) = self.backend.phonemize_with(&text, espeak)?.into_iter().next() else {
            return Ok(String::new());
        };
        let mut ps = py_strip(&first).to_string();
        for (old, new) in &self.e2m {
            ps = ps.replace(old, new);
        }
        Ok(ps.replace(['^', '-'], "").replace('«', "(").replace('»', ")"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_switch_flags_are_dropped_and_parentheses_kept() {
        let fallback = EspeakFallback::new(false).unwrap();
        let georgian = fallback.phonemize("გამარჯობა").unwrap().unwrap();
        assert!(!georgian.contains(['(', ')']), "{georgian}");
        assert!(fallback.phonemize("(qzx)").unwrap().unwrap().starts_with('('));
        let russian = EspeakG2P::new("ru").unwrap();
        let word = russian.phonemize("Привет").unwrap();
        assert_eq!(russian.phonemize("(Привет)").unwrap(), format!("({word})"));
    }
}
