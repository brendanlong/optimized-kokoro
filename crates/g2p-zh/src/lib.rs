//! misaki 0.9.4's Chinese G2P as Kokoro-82M v1.0 uses it: `ZHG2P(version=None)`, the legacy path
//! of `misaki/zh.py` (not `ZHFrontend`, which only v1.1 uses).
//!
//! [`ZhG2p::phonemize`] is `ZHG2P.__call__`: cn2an's `transform(text, "an2cn")` spells out
//! numbers, `map_punctuation` makes CJK punctuation Western, and `legacy_call` reads each run of
//! U+4E00-U+9FFF as jieba's words, each word as pypinyin's readings (TONE3, neutral tone as 5) in
//! misaki's IPA with tone arrows (`pinyin_to_ipa`, `retone`), words joined by spaces. Everything
//! else passes through unchanged.
//!
//! The dictionaries are the reference's own (jieba 0.42.1's `dict.txt` and HMM, pypinyin 0.55.0's
//! `pinyin_dict` and `phrases_dict`), converted by `reference/gen_zh_data.py`; see
//! `data/PROVENANCE.md`.
#![forbid(unsafe_code)]

mod cn2an;
mod jieba;
mod pinyin;

use std::fmt;

pub use cn2an::transform as an2cn;

/// misaki raises: a character in U+4E00-U+9FFF that pypinyin has no reading for (`兙`), where
/// `pinyin_to_ipa` finds no final.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub character: char,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no pinyin for {:?} (U+{:04X})", self.character, self.character as u32)
    }
}

impl std::error::Error for Error {}

/// The loaded dictionaries (about 0.1 s in a release build), shared across threads; calls keep no
/// state.
pub struct ZhG2p {
    jieba: jieba::Jieba,
    pinyin: pinyin::Pinyin,
}

impl Default for ZhG2p {
    fn default() -> Self {
        Self::new()
    }
}

fn is_zh(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c)
}

impl ZhG2p {
    pub fn new() -> ZhG2p {
        ZhG2p { jieba: jieba::Jieba::new(), pinyin: pinyin::Pinyin::new() }
    }

    /// Whether pypinyin has a reading for `c` (kokoro's `foreign.py` tests `lazy_pinyin(c)[0] != c`).
    pub fn reads(&self, c: char) -> bool {
        self.pinyin.reads(c)
    }

    /// `ZHG2P()(text)[0]`.
    pub fn phonemize(&self, text: &str) -> Result<String, Error> {
        if kokoro_text::py::strip(text).is_empty() {
            return Ok(String::new());
        }
        let text = map_punctuation(&cn2an::transform(text));
        self.legacy_call(&text)
    }

    /// `legacy_call`: runs of U+4E00-U+9FFF read as words, the rest kept, and U+032F (the
    /// non-syllabic mark of misaki's diphthongs) removed.
    fn legacy_call(&self, text: &str) -> Result<String, Error> {
        let mut out = String::with_capacity(text.len() * 3);
        let mut words = Vec::new();
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            let zh = is_zh(c);
            let len = rest.find(|c: char| is_zh(c) != zh).unwrap_or(rest.len());
            let (segment, tail) = rest.split_at(len);
            if zh {
                words.clear();
                self.jieba.cut(segment, &mut words);
                for (i, &(s, e)) in words.iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    self.pinyin.word2ipa(&segment[s..e], &mut out).map_err(|character| Error { character })?;
                }
            } else {
                out.push_str(segment);
            }
            rest = tail;
        }
        if out.contains('\u{32F}') {
            out = out.replace('\u{32F}', "");
        }
        Ok(out)
    }

    /// jieba's `lcut` of a run of U+4E00-U+9FFF, for tests.
    pub fn cut<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let mut words = Vec::new();
        self.jieba.cut(text, &mut words);
        words.into_iter().map(|(s, e)| &text[s..e]).collect()
    }
}

/// `ZHG2P.map_punctuation`.
pub fn map_punctuation(text: &str) -> String {
    const MAP: [(char, &str); 18] = [
        ('、', ", "),
        ('，', ", "),
        ('。', ". "),
        ('．', ". "),
        ('！', "! "),
        ('：', ": "),
        ('；', "; "),
        ('？', "? "),
        ('«', " “"),
        ('»', "” "),
        ('《', " “"),
        ('》', "” "),
        ('「', " “"),
        ('」', "” "),
        ('【', " “"),
        ('】', "” "),
        ('（', " ("),
        ('）', ") "),
    ];
    // The replacements' outputs hold none of the characters replaced, so one pass gives what the
    // chain of `str.replace` calls does.
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match MAP.iter().find(|&&(k, _)| k == c) {
            Some((_, r)) => out.push_str(r),
            None => out.push(c),
        }
    }
    kokoro_text::py::strip(&out).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    static G2P: LazyLock<ZhG2p> = LazyLock::new(ZhG2p::new);

    #[test]
    fn polyphones() {
        assert_eq!(G2P.phonemize("银行行长").unwrap(), "i↗nxa↗ŋxa↗ŋꭧa↓ŋ");
        assert_eq!(G2P.phonemize("他在银行工作，行长很长").unwrap(), "tʰa→ ʦai↘ i↗nxa↗ŋ kʊ→ŋʦwo↘, xa↗ŋꭧa↓ŋ xə↓nꭧa↓ŋ");
    }

    #[test]
    fn passes_through_and_fails_like_misaki() {
        assert_eq!(G2P.phonemize("abc 你好").unwrap(), "abc ni↓xau↓");
        assert_eq!(G2P.phonemize("嗯").unwrap(), "n↗");
        assert_eq!(G2P.phonemize("〇㐀").unwrap(), "〇㐀");
        assert_eq!(G2P.phonemize("  ").unwrap(), "");
        assert_eq!(G2P.phonemize("兙"), Err(Error { character: '兙' }));
        assert!(G2P.reads('兀') && !G2P.reads('兙') && G2P.reads('㐀') && !G2P.reads('ー'));
    }
}
