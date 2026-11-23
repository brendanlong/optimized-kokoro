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

/// Sounds misaki's rewrites of espeak's phonemes write as one of Kokoro's symbols (`a^ɪ` as `I`).
/// While the rewrites run, each is a private-use stand-in, so the later ones (`o` → `ɔ`, `e` →
/// e͡ɪ) can't reach into its letters; [`ipa`] then writes it with a tie bar.
mod merged {
    pub const AI: &str = "\u{e000}";
    pub const AU: &str = "\u{e001}";
    pub const DZ: &str = "\u{e002}";
    pub const DZH: &str = "\u{e003}";
    pub const EI: &str = "\u{e004}";
    pub const OU: &str = "\u{e005}";
    pub const EU: &str = "\u{e006}";
    pub const SS: &str = "\u{e007}";
    pub const TS: &str = "\u{e008}";
    pub const TSH: &str = "\u{e009}";
    pub const OI: &str = "\u{e00a}";
    const IPA: [&str; 11] = ["a͡ɪ", "a͡ʊ", "d͡z", "d͡ʒ", "e͡ɪ", "o͡ʊ", "ə͡ʊ", "s͡s", "t͡s", "t͡ʃ", "ɔ͡ɪ"];

    pub fn ipa(ps: &str) -> String {
        let mut out = String::with_capacity(ps.len() + 8);
        for c in ps.chars() {
            match (c as usize).checked_sub(0xe000).and_then(|i| IPA.get(i)) {
                Some(ipa) => out.push_str(ipa),
                None => out.push(c),
            }
        }
        out
    }
}
use merged::*;
use std::sync::{Arc, OnceLock};

pub use kokoro_espeak_ng::{Error, EspeakData, Result, alphabet_languages};
pub use phonemizer::compose_marks;

/// The data in [`kokoro_espeak_ng::default_data_dir`], loaded once per process.
pub fn default_data() -> Result<Arc<EspeakData>> {
    static DATA: OnceLock<Arc<EspeakData>> = OnceLock::new();
    if let Some(d) = DATA.get() {
        return Ok(d.clone());
    }
    let d = EspeakData::load(kokoro_espeak_ng::default_data_dir())?;
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

    pub fn with_data(british: bool, data: Arc<EspeakData>) -> Result<Self> {
        let language = if british { "en-gb" } else { "en-us" };
        let backend = Backend::new(language, data)?;
        // Python sorts the dict items by descending length, keeping insertion order among equals.
        let mut e2m = vec![
            ("ʔˌn\u{329}", "ʔn"),
            ("ʔn\u{329}", "ʔn"),
            ("a^ɪ", AI),
            ("a^ʊ", AU),
            ("d^ʒ", DZH),
            ("e^ɪ", EI),
            ("e", EI),
            ("t^ʃ", TSH),
            ("ɔ^ɪ", OI),
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

    pub fn backend(&mut self) -> &mut Backend {
        &mut self.backend
    }

    /// Phonemes for one token's text, or `None` when espeak has nothing to say.
    pub fn phonemize(&mut self, word: &str) -> Option<String> {
        let raw = phonemizer::phonemize(word, |chunk| self.backend.espeak(chunk))?;
        Some(self.espeak_to_ipa(&raw))
    }

    /// [`phonemize`](Self::phonemize) with `espeak` standing in for the engine.
    pub fn phonemize_with(&self, word: &str, espeak: impl FnMut(&str) -> String) -> Option<String> {
        let raw = phonemizer::phonemize(word, espeak)?;
        Some(self.espeak_to_ipa(&raw))
    }

    fn espeak_to_ipa(&self, raw: &str) -> String {
        let mut ps = py_strip(raw).to_string();
        for (old, new) in &self.e2m {
            ps = ps.replace(old, new);
        }
        ps = syllabic_to_schwa(&ps).replace('\u{329}', "");
        if self.british {
            ps = ps.replace("e^ə", "ɛː").replace("iə", "ɪə").replace("ə^ʊ", EU);
        } else {
            ps = ps.replace("o^ʊ", OU).replace("ɜːɹ", "ɜɹ").replace("ɜː", "ɜɹ").replace("ɪə", "iə").replace('ː', "");
        }
        merged::ipa(&ps.replace('o', "ɔ").replace('ɾ', "t̬").replace('ʔ', "t").replace('^', ""))
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

    pub fn with_data(language: &str, data: Arc<EspeakData>) -> Result<Self> {
        let backend = Backend::new(language, data)?;
        let mut e2m = vec![
            ("a^ɪ", AI),
            ("a^ʊ", AU),
            ("d^z", DZ),
            ("d^ʒ", DZH),
            ("e^ɪ", EI),
            ("o^ʊ", OU),
            ("ə^ʊ", EU),
            ("s^s", SS),
            ("t^s", TS),
            ("t^ʃ", TSH),
            ("ɔ^ɪ", OI),
        ];
        e2m.sort();
        Ok(Self { backend, e2m })
    }

    pub fn backend(&mut self) -> &mut Backend {
        &mut self.backend
    }

    /// misaki swaps `()` for `«»` before phonemizer and back after, so that phonemizer's flag
    /// removal can't take the text's parentheses for language-switch flags. Both are punctuation
    /// phonemizer keeps out of what espeak reads and puts back as they were, so the swap changes
    /// nothing but `«»` in the text, which come out as `“”`; that is all this does.
    pub fn phonemize(&mut self, text: &str) -> String {
        let raw = phonemizer::phonemize(&swap_guillemets(text), |chunk| self.backend.espeak(chunk));
        self.espeak_to_ipa(&raw.unwrap_or_default())
    }

    /// [`phonemize`](Self::phonemize) with `espeak` standing in for the engine.
    pub fn phonemize_with(&self, text: &str, espeak: impl FnMut(&str) -> String) -> String {
        let raw = phonemizer::phonemize(&swap_guillemets(text), espeak);
        self.espeak_to_ipa(&raw.unwrap_or_default())
    }

    fn espeak_to_ipa(&self, raw: &str) -> String {
        let mut ps = py_strip(raw).to_string();
        for (old, new) in &self.e2m {
            ps = ps.replace(old, new);
        }
        merged::ipa(&ps.replace(['^', '-'], ""))
    }
}

fn swap_guillemets(text: &str) -> String {
    text.replace('«', "\u{201c}").replace('»', "\u{201d}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_switch_flags_are_dropped_and_parentheses_kept() {
        let mut fallback = EspeakFallback::new(false).unwrap();
        let georgian = fallback.phonemize("გამარჯობა").unwrap();
        assert!(!georgian.contains(['(', ')']), "{georgian}");
        assert!(fallback.phonemize("(qzx)").unwrap().starts_with('('));
        let mut russian = EspeakG2P::new("ru").unwrap();
        let word = russian.phonemize("Привет");
        assert_eq!(russian.phonemize("(Привет)"), format!("({word})"));
    }

    #[test]
    fn letters_espeak_lacks_are_read_without_their_accents() {
        let mut fallback = EspeakFallback::new(false).unwrap();
        assert_eq!(fallback.phonemize("Nguyễn"), fallback.phonemize("Nguyen"));
        let mut spanish = EspeakG2P::new("es").unwrap();
        assert_eq!(spanish.phonemize("Phạm Văn Đồng"), spanish.phonemize("Pham Văn Đong"));
        let mut russian = EspeakG2P::new("ru").unwrap();
        assert_eq!(russian.phonemize("\u{45d}"), russian.phonemize("\u{438}"));
    }
}
