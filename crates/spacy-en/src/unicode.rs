//! Python's `str` predicates and `str.lower()`, from tables exported from the reference Python, so
//! they follow its Unicode version rather than Rust's.

use serde::Deserialize;
use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
#[serde(from = "Vec<(u32, u32)>")]
pub struct CharSet {
    ascii: u128,
    ranges: Vec<(u32, u32)>,
}

impl From<Vec<(u32, u32)>> for CharSet {
    fn from(ranges: Vec<(u32, u32)>) -> Self {
        let mut ascii = 0u128;
        for &(lo, hi) in &ranges {
            for c in lo..=hi.min(127) {
                ascii |= 1 << c;
            }
        }
        CharSet { ascii, ranges }
    }
}

impl CharSet {
    pub fn contains(&self, c: char) -> bool {
        let c = c as u32;
        if c < 128 {
            return self.ascii & (1 << c) != 0;
        }
        self.ranges
            .binary_search_by(|&(lo, hi)| {
                if hi < c {
                    std::cmp::Ordering::Less
                } else if lo > c {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }
}

#[derive(Debug, Deserialize)]
pub struct Unicode {
    pub space: CharSet,
    pub alpha: CharSet,
    pub upper: CharSet,
    pub digit: CharSet,
    cased: CharSet,
    case_ignorable: CharSet,
    #[serde(deserialize_with = "lower_table")]
    lower: HashMap<char, Box<str>>,
}

fn lower_table<'de, D: serde::Deserializer<'de>>(d: D) -> Result<HashMap<char, Box<str>>, D::Error> {
    let pairs: Vec<(u32, String)> = Deserialize::deserialize(d)?;
    pairs
        .into_iter()
        .map(|(cp, s)| {
            char::from_u32(cp)
                .map(|c| (c, s.into_boxed_str()))
                .ok_or_else(|| serde::de::Error::custom(format!("bad code point {cp:#x}")))
        })
        .collect()
}

const CAPITAL_SIGMA: char = '\u{3a3}';

impl Unicode {
    /// `str.isspace()`.
    pub fn is_space(&self, s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| self.space.contains(c))
    }

    /// `str.lower()`, including the final-sigma rule.
    pub fn lower<'a>(&self, s: &'a str) -> Cow<'a, str> {
        let Some(first) = s.char_indices().find(|&(_, c)| self.lower.contains_key(&c)) else {
            return Cow::Borrowed(s);
        };
        let mut out = String::with_capacity(s.len() + 4);
        out.push_str(&s[..first.0]);
        for (i, c) in s[first.0..].char_indices().map(|(i, c)| (i + first.0, c)) {
            if c == CAPITAL_SIGMA {
                out.push(if self.final_sigma(s, i) { '\u{3c2}' } else { '\u{3c3}' });
            } else if let Some(l) = self.lower.get(&c) {
                out.push_str(l);
            } else {
                out.push(c);
            }
        }
        Cow::Owned(out)
    }

    /// CPython's `handle_capital_sigma`.
    fn final_sigma(&self, s: &str, i: usize) -> bool {
        let before = s[..i].chars().rev().find(|&c| !self.case_ignorable.contains(c));
        if !before.is_some_and(|c| self.cased.contains(c)) {
            return false;
        }
        let after = s[i + CAPITAL_SIGMA.len_utf8()..].chars().find(|&c| !self.case_ignorable.contains(c));
        !after.is_some_and(|c| self.cased.contains(c))
    }

    /// `spacy.lang.lex_attrs.word_shape`.
    pub fn word_shape(&self, text: &str) -> Cow<'static, str> {
        if text.chars().count() >= 100 {
            return Cow::Borrowed("LONG");
        }
        let mut shape = String::with_capacity(text.len());
        let mut last = None;
        let mut seq = 0;
        for c in text.chars() {
            let shape_char = if self.alpha.contains(c) {
                if self.upper.contains(c) { 'X' } else { 'x' }
            } else if self.digit.contains(c) {
                'd'
            } else {
                c
            };
            if last == Some(shape_char) {
                seq += 1;
            } else {
                seq = 0;
                last = Some(shape_char);
            }
            if seq < 4 {
                shape.push(shape_char);
            }
        }
        Cow::Owned(shape)
    }
}
