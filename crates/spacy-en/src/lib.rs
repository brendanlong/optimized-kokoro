//! spaCy's English tokenizer and part-of-speech tagger, as misaki runs them:
//! `spacy.load("en_core_web_sm", enable=["tok2vec", "tagger"])` with spaCy 3.8.5 and
//! en_core_web_sm 3.8.0. Each token's `text`, `tag_` and `whitespace_` match spaCy's.
//!
//! The model data in `data/` is exported from the installed package by `export.py`.
// The one exception is `tagger::matmul_bt`, which calls matrixmultiply's sgemm.
#![deny(unsafe_code)]

mod hash;
mod patterns;
mod tagger;
mod tokenizer;
mod unicode;

use serde::Deserialize;
use std::collections::HashMap;
use tagger::{FEATURES, ModelMeta, Tagger};
#[doc(hidden)]
pub use tokenizer::PatternMatches;
use tokenizer::{RawToken, Tokenizer, TokenizerData};
use unicode::Unicode;

/// en_core_web_sm's tag set, in the tagger's output order.
pub const TAGS: [&str; 50] = [
    "$", "''", ",", "-LRB-", "-RRB-", ".", ":", "ADD", "AFX", "CC", "CD", "DT", "EX", "FW", "HYPH", "IN", "JJ", "JJR",
    "JJS", "LS", "MD", "NFP", "NN", "NNP", "NNPS", "NNS", "PDT", "POS", "PRP", "PRP$", "RB", "RBR", "RBS", "RP", "SYM",
    "TO", "UH", "VB", "VBD", "VBG", "VBN", "VBP", "VBZ", "WDT", "WP", "WP$", "WRB", "XX", "_SP", "``",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'t> {
    pub text: &'t str,
    /// Penn Treebank tag (`tag_`); empty from [`Nlp::tokenize`].
    pub tag: &'static str,
    /// `whitespace_`: `" "` or `""`.
    pub whitespace: &'static str,
}

#[derive(Deserialize)]
struct Data {
    #[serde(flatten)]
    tokenizer: TokenizerData,
    lexeme_norm: Vec<(u64, Box<str>)>,
    base_norms: HashMap<Box<str>, Box<str>>,
    symbols: HashMap<Box<str>, u64>,
    tags: Vec<String>,
    model: ModelMeta,
    unicode: Unicode,
}

pub struct Nlp {
    tokenizer: Tokenizer,
    tagger: Tagger,
    /// The model's NORM exceptions, by string id; checked before `base_norms`.
    lexeme_norm: HashMap<u64, Box<str>>,
    /// spaCy's `BASE_NORMS`.
    base_norms: HashMap<Box<str>, Box<str>>,
    /// spaCy's reserved symbols, whose string ids are small integers instead of hashes.
    symbols: HashMap<Box<str>, u64>,
    unicode: Unicode,
}

impl Default for Nlp {
    fn default() -> Self {
        Self::new()
    }
}

impl Nlp {
    /// Loads the bundled en_core_web_sm data.
    pub fn new() -> Self {
        Self::from_data(include_bytes!("../data/tokenizer.json"), include_bytes!("../data/tagger.safetensors"))
    }

    /// Loads `tokenizer.json` and `tagger.safetensors` as written by `export.py`.
    pub fn from_data(tokenizer_json: &[u8], weights: &[u8]) -> Self {
        let data: Data = serde_json::from_slice(tokenizer_json).expect("tokenizer.json");
        assert_eq!(data.tags, TAGS, "tag set");
        let tagger = Tagger::load(weights, &data.model, TAGS.len());
        Nlp {
            tokenizer: Tokenizer::new(data.tokenizer, data.unicode.space.clone()),
            tagger,
            lexeme_norm: data.lexeme_norm.into_iter().collect(),
            base_norms: data.base_norms,
            symbols: data.symbols,
            unicode: data.unicode,
        }
    }

    /// Tokenizes without tagging.
    pub fn tokenize<'t>(&self, text: &'t str) -> Vec<Token<'t>> {
        self.tokenizer.tokenize(text).iter().map(|t| token(text, t, "")).collect()
    }

    /// Tokenizes and tags `text`, as `nlp(text)` does.
    pub fn process<'t>(&self, text: &'t str) -> Vec<Token<'t>> {
        let (raw, scores) = self.scores(text);
        raw.iter()
            .zip(scores.chunks_exact(TAGS.len()))
            .map(|(t, s)| token(text, t, TAGS[tagger::argmax(s)]))
            .collect()
    }

    /// Tokens with the tagger's unnormalized scores, `TAGS.len()` per token.
    #[doc(hidden)]
    pub fn tag_scores<'t>(&self, text: &'t str) -> Vec<(Token<'t>, Vec<f32>)> {
        let (raw, scores) = self.scores(text);
        raw.iter().zip(scores.chunks_exact(TAGS.len())).map(|(t, s)| (token(text, t, ""), s.to_vec())).collect()
    }

    /// What the tokenizer's prefix, suffix, infix and URL patterns find on `s`, for testing them
    /// against Python's.
    #[doc(hidden)]
    pub fn pattern_matches(&self, s: &str) -> PatternMatches {
        self.tokenizer.pattern_matches(s)
    }

    fn scores<'n>(&'n self, text: &str) -> (Vec<RawToken<'n>>, Vec<f32>) {
        let raw = self.tokenizer.tokenize(text);
        let features: Vec<_> = raw.iter().map(|t| self.features(text, t)).collect();
        let scores = self.tagger.scores(&features);
        (raw, scores)
    }

    /// The `doc.to_array(FEATURES)` row for one token.
    fn features(&self, text: &str, t: &RawToken) -> [u64; FEATURES.len()] {
        let word = &text[t.start..t.end];
        let norm = match t.norm {
            Some(norm) => self.key(norm),
            None => match self.lexeme_norm.get(&self.key(word)).or_else(|| self.base_norms.get(word)) {
                Some(norm) => self.key(norm),
                None => self.key(&self.unicode.lower(word)),
            },
        };
        let first = word.chars().next().map_or(0, char::len_utf8);
        let suffix_start = word.char_indices().rev().nth(2).map_or(0, |(i, _)| i);
        [
            norm,
            self.key(&word[..first]),
            self.key(&word[suffix_start..]),
            self.key(&self.unicode.word_shape(word)),
            u64::from(t.space),
            u64::from(self.unicode.is_space(word)),
        ]
    }

    /// `StringStore.add`: the string's id.
    fn key(&self, s: &str) -> u64 {
        if s.is_empty() {
            return 0;
        }
        self.symbols.get(s).copied().unwrap_or_else(|| hash::murmur64a(s.as_bytes(), 1))
    }
}

fn token<'t>(text: &'t str, t: &RawToken, tag: &'static str) -> Token<'t> {
    Token { text: &text[t.start..t.end], tag, whitespace: if t.space { " " } else { "" } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shareable_across_threads() {
        fn check<T: Send + Sync>() {}
        check::<Nlp>();
    }

    #[test]
    fn tags_a_sentence() {
        let nlp = Nlp::new();
        let tokens = nlp.process("I can't read it.");
        let words: Vec<_> = tokens.iter().map(|t| (t.text, t.tag, t.whitespace)).collect();
        assert_eq!(
            words,
            [("I", "PRP", " "), ("ca", "MD", ""), ("n't", "RB", " "), ("read", "VB", " "), ("it", "PRP", ""), (".", ".", "")]
        );
    }
}
