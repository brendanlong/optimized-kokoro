//! pypinyin 0.55.0's `lazy_pinyin(word, style=Style.TONE3, neutral_tone_with_five=True)` for the
//! words jieba gives misaki, with each syllable already turned into misaki's IPA
//! (`ZHG2P.py2ipa`): `reference/gen_zh_data.py` computed that for every reading in the data, with
//! misaki's own code.
//!
//! A word is cut with pypinyin's mmseg (forward maximum matching on the phrase dictionary, strict
//! mode), and each piece is read from `phrases_dict` if it is a phrase, else character by character
//! from `pinyin_dict`, taking the first reading of each character (`heteronym=False`).

use rustc_hash::{FxHashMap, FxHashSet};

pub struct Pinyin {
    /// Each character's IPA (its first reading).
    chars: FxHashMap<char, &'static str>,
    /// Each phrase's IPA, its syllables joined.
    phrases: FxHashMap<&'static str, String>,
    /// mmseg's `PrefixSet`: every prefix of every phrase, phrases included.
    prefixes: FxHashSet<&'static str>,
}

impl Pinyin {
    pub fn new() -> Pinyin {
        let mut syllables = FxHashMap::default();
        for line in include_str!("../data/syllables.txt").lines() {
            let (py, ipa) = line.split_once(' ').expect("pinyin ipa");
            assert!(!ipa.starts_with('!'), "misaki reads every syllable in the data");
            syllables.insert(py, ipa);
        }
        let ipa = |py: &str| *syllables.get(py).unwrap_or_else(|| panic!("no IPA for {py}"));
        let chars = include_str!("../data/chars.txt")
            .lines()
            .map(|line| {
                let (c, py) = line.split_once(' ').expect("char pinyin");
                (c.chars().next().expect("a character"), ipa(py))
            })
            .collect();
        let mut phrases = FxHashMap::default();
        let mut prefixes = FxHashSet::default();
        for line in include_str!("../data/phrases.txt").lines() {
            let (phrase, pys) = line.split_once(' ').expect("phrase pinyin");
            phrases.insert(phrase, pys.split(' ').map(ipa).collect());
            for (i, c) in phrase.char_indices() {
                prefixes.insert(&phrase[..i + c.len_utf8()]);
            }
        }
        Pinyin { chars, phrases, prefixes }
    }

    /// Whether pypinyin reads `c` (`lazy_pinyin(c)[0] != c`).
    pub fn reads(&self, c: char) -> bool {
        self.chars.contains_key(&c)
    }

    /// misaki's `word2ipa(word)`, appended to `out`: `Err(c)` for the first character without a
    /// reading, where misaki's `pinyin_to_ipa` raises.
    pub fn word2ipa(&self, word: &str, out: &mut String) -> Result<(), char> {
        let mut pieces = Vec::new();
        self.mmseg(word, &mut pieces);
        for (s, e) in pieces {
            let piece = &word[s..e];
            if let Some(ipa) = self.phrases.get(piece) {
                out.push_str(ipa);
                continue;
            }
            for c in piece.chars() {
                out.push_str(self.chars.get(&c).ok_or(c)?);
            }
        }
        Ok(())
    }

    /// `mmseg.seg.cut(text)` (`Seg(p_set, no_non_phrases=True)`), as byte ranges.
    ///
    /// One quirk is kept: when everything left is a prefix of some phrase but holds no phrase, the
    /// rest comes out one character at a time without looking for phrases inside it.
    fn mmseg(&self, text: &str, out: &mut Vec<(usize, usize)>) {
        let mut pos = 0;
        'outer: while pos < text.len() {
            let remain = &text[pos..];
            let mut last_valid = None;
            for (i, c) in remain.char_indices() {
                let end = i + c.len_utf8();
                let word = &remain[..end];
                if self.prefixes.contains(word) {
                    if self.phrases.contains_key(word) {
                        last_valid = Some(end);
                    }
                } else {
                    let len = last_valid.unwrap_or_else(|| remain.chars().next().map_or(0, char::len_utf8));
                    out.push((pos, pos + len));
                    pos += len;
                    continue 'outer;
                }
            }
            match last_valid {
                Some(len) => {
                    out.push((pos, pos + len));
                    pos += len;
                }
                None => {
                    // `remain` would be a phrase, and so `last_valid`, if it were in the dictionary.
                    out.extend(remain.char_indices().map(|(i, c)| (pos + i, pos + i + c.len_utf8())));
                    break;
                }
            }
        }
    }
}
