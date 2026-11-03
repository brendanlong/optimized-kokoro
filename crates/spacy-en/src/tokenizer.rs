//! spaCy's `Tokenizer.__call__` (spacy/tokenizer.pyx), with en_core_web_sm's rules.

use crate::unicode::CharSet;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Deserialize)]
pub(crate) struct TokenizerData {
    prefix: PatternData,
    suffix: PatternData,
    infix: PatternData,
    url_match: PatternData,
    special_cases: HashMap<String, Vec<(String, Option<String>)>>,
    special_phrases: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PatternData {
    pattern: String,
    filter: String,
}

/// A Python regex, plus the same regex without lookarounds, which matches wherever it does (and
/// more) and runs on the fast non-backtracking engine.
struct Pattern {
    full: fancy_regex::Regex,
    filter: regex::Regex,
}

impl Pattern {
    fn new(data: &PatternData) -> Self {
        Pattern {
            // Python's `re` has no backtracking limit; fancy-regex errors out past its default one.
            full: fancy_regex::RegexBuilder::new(&data.pattern)
                .backtrack_limit(usize::MAX)
                .build()
                .expect("exported pattern"),
            filter: regex::Regex::new(&data.filter).expect("exported filter pattern"),
        }
    }

    /// `re.search` from `pos`: the leftmost match.
    fn find_from(&self, s: &str, pos: usize) -> Option<(usize, usize)> {
        let candidate = self.filter.find_at(s, pos)?.start();
        let m = self.full.find_from_pos(s, candidate).unwrap_or_else(|e| panic!("regex on {s:?}: {e}"));
        m.map(|m| (m.start(), m.end()))
    }

    /// `re.finditer`, for a pattern that can't match the empty string.
    fn find_iter<'s>(&'s self, s: &'s str) -> impl Iterator<Item = (usize, usize)> + 's {
        let mut pos = 0;
        std::iter::from_fn(move || {
            let (start, end) = self.find_from(s, pos)?;
            assert!(end > start, "empty match in {s:?}");
            pos = end;
            Some((start, end))
        })
    }
}

struct SpecialToken {
    len: usize,
    norm: Option<Box<str>>,
}

/// A token as byte offsets into the text, with the NORM a special case gave it, if any.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RawToken<'n> {
    pub start: usize,
    pub end: usize,
    pub norm: Option<&'n str>,
    pub space: bool,
}

pub(crate) struct Tokenizer {
    prefix: Pattern,
    suffix: Pattern,
    infix: Pattern,
    url_match: Pattern,
    specials: HashMap<Box<str>, Vec<SpecialToken>>,
    /// Special cases re-applied over token sequences, keyed by their first token's text.
    phrases: HashMap<Box<str>, Vec<Vec<Box<str>>>>,
    space: CharSet,
}

impl Tokenizer {
    pub fn new(data: TokenizerData, space: CharSet) -> Self {
        let specials = data
            .special_cases
            .into_iter()
            .map(|(chunk, tokens)| {
                let tokens = tokens
                    .into_iter()
                    .map(|(orth, norm)| SpecialToken { len: orth.len(), norm: norm.map(String::into_boxed_str) })
                    .collect();
                (chunk.into_boxed_str(), tokens)
            })
            .collect();
        let mut phrases: HashMap<Box<str>, Vec<Vec<Box<str>>>> = HashMap::new();
        for phrase in data.special_phrases {
            let mut words = phrase.into_iter().map(String::into_boxed_str);
            let first = words.next().expect("empty special-case phrase");
            phrases.entry(first).or_default().push(words.collect());
        }
        Tokenizer {
            prefix: Pattern::new(&data.prefix),
            suffix: Pattern::new(&data.suffix),
            infix: Pattern::new(&data.infix),
            url_match: Pattern::new(&data.url_match),
            specials,
            phrases,
            space,
        }
    }

    pub fn tokenize<'n>(&'n self, text: &str) -> Vec<RawToken<'n>> {
        let mut tokens = Vec::with_capacity(text.len() / 4 + 1);
        let Some(first) = text.chars().next() else {
            return tokens;
        };
        let mut start = 0;
        let mut in_ws = self.space.contains(first);
        for (i, c) in text.char_indices() {
            if self.space.contains(c) != in_ws {
                if start < i {
                    self.tokenize_span(text, start, i, &mut tokens);
                }
                if c == ' ' {
                    tokens.last_mut().expect("a span precedes the space").space = true;
                    start = i + 1;
                } else {
                    start = i;
                }
                in_ws = !in_ws;
            }
        }
        if start < text.len() {
            self.tokenize_span(text, start, text.len(), &mut tokens);
            tokens.last_mut().unwrap().space = text.ends_with(' ') && !in_ws;
        }
        self.apply_special_phrases(text, &mut tokens);
        tokens
    }

    fn push_special<'n>(&'n self, chunk: &str, mut at: usize, tokens: &mut Vec<RawToken<'n>>) -> bool {
        let Some(special) = self.specials.get(chunk) else {
            return false;
        };
        for t in special {
            tokens.push(RawToken { start: at, end: at + t.len, norm: t.norm.as_deref(), space: false });
            at += t.len;
        }
        true
    }

    /// `_tokenize`: `_split_affixes` then `_attach_tokens`. spaCy also caches results per span, which
    /// doesn't change them: a cached span is only looked up again as a string with no affixes left,
    /// for which `_split_affixes` is a no-op.
    fn tokenize_span<'n>(&'n self, text: &str, start: usize, end: usize, tokens: &mut Vec<RawToken<'n>>) {
        if self.push_special(&text[start..end], start, tokens) {
            return;
        }
        let lexeme = |start, end| RawToken { start, end, norm: None, space: false };
        let (mut a, mut b) = (start, end);
        let mut prefixes = Vec::new();
        let mut suffixes = Vec::new();
        let mut last_size = 0;
        while a < b && b - a != last_size {
            if self.specials.contains_key(&text[a..b]) {
                break;
            }
            last_size = b - a;
            let pre_len = self.find_prefix(&text[a..b]);
            if pre_len != 0 && a + pre_len < b && self.specials.contains_key(&text[a + pre_len..b]) {
                prefixes.push(lexeme(a, a + pre_len));
                a += pre_len;
                break;
            }
            let suf_len = self.find_suffix(&text[a + pre_len..b]);
            if suf_len != 0 && a < b - suf_len && self.specials.contains_key(&text[a..b - suf_len]) {
                suffixes.push(lexeme(b - suf_len, b));
                b -= suf_len;
                break;
            }
            if pre_len != 0 && suf_len != 0 && pre_len + suf_len <= b - a {
                prefixes.push(lexeme(a, a + pre_len));
                suffixes.push(lexeme(b - suf_len, b));
                a += pre_len;
                b -= suf_len;
            } else if pre_len != 0 {
                prefixes.push(lexeme(a, a + pre_len));
                a += pre_len;
            } else if suf_len != 0 {
                suffixes.push(lexeme(b - suf_len, b));
                b -= suf_len;
            }
        }
        tokens.extend(prefixes);
        if a < b && !self.push_special(&text[a..b], a, tokens) {
            let string = &text[a..b];
            if self.url_match.find_from(string, 0).is_some() {
                tokens.push(lexeme(a, b));
            } else {
                let mut start = 0;
                for (m_start, m_end) in self.infix.find_iter(string) {
                    if m_start == 0 {
                        continue;
                    }
                    if m_start != start {
                        tokens.push(lexeme(a + start, a + m_start));
                    }
                    tokens.push(lexeme(a + m_start, a + m_end));
                    start = m_end;
                }
                if start < string.len() {
                    tokens.push(lexeme(a + start, b));
                }
            }
        }
        tokens.extend(suffixes.into_iter().rev());
    }

    fn find_prefix(&self, s: &str) -> usize {
        self.prefix.find_from(s, 0).map_or(0, |(start, end)| end - start)
    }

    fn find_suffix(&self, s: &str) -> usize {
        self.suffix.find_from(s, 0).map_or(0, |(start, end)| {
            debug_assert_eq!(end, s.len());
            end - start
        })
    }

    /// `_apply_special_cases`: the PhraseMatcher pass over special cases that contain affixes.
    fn apply_special_phrases<'n>(&'n self, text: &str, tokens: &mut Vec<RawToken<'n>>) {
        let word = |t: &RawToken| &text[t.start..t.end];
        let mut matches = Vec::new();
        for (i, t) in tokens.iter().enumerate() {
            let Some(rests) = self.phrases.get(word(t)) else {
                continue;
            };
            for rest in rests {
                let end = i + 1 + rest.len();
                if end <= tokens.len() && rest.iter().zip(&tokens[i + 1..end]).all(|(w, t)| **w == *word(t)) {
                    matches.push((i, end));
                }
            }
        }
        if matches.is_empty() {
            return;
        }
        // Longest first, then leftmost; a span is dropped if its first or last token was seen.
        matches.sort_by(|x, y| (y.1 - y.0).cmp(&(x.1 - x.0)).then(x.0.cmp(&y.0)));
        let mut seen = BTreeSet::new();
        let mut kept = Vec::new();
        for &(start, end) in &matches {
            if !seen.contains(&start) && !seen.contains(&(end - 1)) {
                kept.push((start, end));
            }
            seen.extend(start..end);
        }
        kept.sort_by_key(|&(start, _)| start);
        let mut out = Vec::with_capacity(tokens.len());
        let mut i = 0;
        let mut kept = kept.into_iter().peekable();
        while i < tokens.len() {
            match kept.peek() {
                Some(&(start, end)) if start == i => {
                    kept.next();
                    let (from, to) = (tokens[start].start, tokens[end - 1].end);
                    if self.push_special(&text[from..to], from, &mut out) {
                        out.last_mut().unwrap().space = tokens[end - 1].space;
                    } else {
                        out.extend_from_slice(&tokens[start..end]);
                    }
                    i = end;
                }
                _ => {
                    out.push(tokens[i]);
                    i += 1;
                }
            }
        }
        *tokens = out;
    }
}
