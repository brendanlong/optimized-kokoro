//! Line joining, sentence splitting and chunking (`text_processor.py`'s `smart_split`).
//!
//! G2P lives in other crates, so chunking takes the phonemizer as a function. Chunk limits count
//! phoneme characters, as the reference does.

use crate::normalize::{Normalizer, Options};
use crate::py::{self, parse_float};
use crate::pyre::{Pattern, compile};
use crate::tags::{Limits, PAUSE_TAG, strip_emoji};
use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::LazyLock;
use unicode_segmentation::UnicodeSegmentation;

/// `settings.absolute_max_tokens`: phonemes per chunk. The model itself takes up to 510.
pub const DEFAULT_MAX_TOKENS: usize = 450;

static CUSTOM_PHONEMES: LazyLock<Pattern> = LazyLock::new(|| compile(r"(\[[^\[\]]*?\]\(\/[^\/\(\)]*?\/\))"));
static ANNOTATION_MARK: LazyLock<Pattern> = LazyLock::new(|| compile(r".(\d+)"));
static PARAGRAPH: LazyLock<Pattern> = LazyLock::new(|| compile(r"(?:\r?\n[^\S\r\n]*){2,}"));
static LINE: LazyLock<Pattern> = LazyLock::new(|| compile(r"\r?\n[^\S\r\n]*"));
static CLAUSE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"([,;:，、；：])").unwrap());

/// `join_lines`: blank lines end a sentence (adding a period if it has no final punctuation),
/// single newlines are hard wraps.
pub fn join_lines(text: &str) -> String {
    let paragraphs: Vec<String> = PARAGRAPH.split(text)
        .into_iter()
        .flatten()
        .map(|p| py::strip(&LINE.sub_str(p, " ")).to_owned())
        .filter(|p| !p.is_empty())
        .collect();
    let last = paragraphs.len().saturating_sub(1);
    let mut out = String::with_capacity(text.len() + paragraphs.len());
    for (index, paragraph) in paragraphs.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(paragraph);
        let end = paragraph.trim_end_matches(['"', '\'', ')', ']', '”', '’']).chars().next_back();
        if index < last && end.is_some_and(|c| !".!?:…".contains(c)) {
            out.push('.');
        }
    }
    out
}

/// What `smart_split` normalizes: the normalizer runs between `[word](/phonemes/)` annotations,
/// never inside them.
pub fn normalize(text: &str, lang_code: &str, options: &Options) -> String {
    match Normalizer::for_lang(lang_code).filter(|_| options.normalize) {
        Some(normalizer) => {
            let parts = CUSTOM_PHONEMES.split(text);
            let joined: String = parts
                .into_iter()
                .enumerate()
                .map(|(index, part)| {
                    let part = part.expect("one group, always present");
                    if index % 2 == 0 { normalizer.normalize(part, options) } else { part.to_owned() }
                })
                .collect();
            py::strip(&joined).to_owned()
        }
        None => text.to_owned(),
    }
}

/// `sentences`: UAX #29 sentences (unicode-segmentation 1.13.3, as the reference's
/// `unicode_segmentation_rs` binding bundles), never split inside an annotation.
pub fn sentences(text: &str) -> Vec<String> {
    let mut annotations = Vec::new();
    let masked = CUSTOM_PHONEMES.sub(text, |c| {
        let annotation = &c[0];
        // Keeps the annotation's first letter, so a capitalised one can still start a sentence.
        let first = annotation[1..].chars().next().filter(|&c| py::is_alnum(c)).unwrap_or('x');
        let index = annotations.len();
        annotations.push(annotation.to_owned());
        format!("{first}\u{e000}{index}\u{e001}")
    });
    masked
        .unicode_sentences()
        .map(|sentence| {
            ANNOTATION_MARK.sub(sentence, |c| {
                index_of(&c[1]).and_then(|i| annotations.get(i)).cloned().unwrap_or_else(|| c[0].to_owned())
            })
        })
        .collect()
}

fn index_of(digits: &str) -> Option<usize> {
    digits.chars().try_fold(0usize, |n, c| n.checked_mul(10)?.checked_add(py::digit_value(c)? as usize))
}

/// What a G2P returns: at least its phoneme string, whose length in characters is what chunk
/// limits count.
pub trait Phonemes {
    fn phonemes(&self) -> &str;
}

impl Phonemes for String {
    fn phonemes(&self) -> &str {
        self
    }
}

fn phoneme_len<P: Phonemes>(p: &P) -> usize {
    p.phonemes().chars().count()
}

/// A phonemized piece of text: `Phonemes(text, ...)` in the reference.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece<P> {
    pub text: String,
    pub phonemes: P,
}

/// One item of `smart_split`'s output.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk<P> {
    Speech(Piece<P>),
    /// Seconds of silence from a `[pause:Xs]` tag.
    Pause(f64),
}

fn phonemize<P, E, F: FnMut(&str) -> Result<P, E>>(g2p: &mut F, text: &str) -> Result<Piece<P>, E> {
    Ok(Piece { text: text.to_owned(), phonemes: g2p(text)? })
}

/// `sentence_chunks`: the sentence whole, or cut at clause punctuation and repacked when its
/// phonemes are longer than `max_tokens`.
pub fn sentence_chunks<P: Phonemes, F: FnMut(&str) -> P>(sentence: &str, max_tokens: usize, g2p: &mut F) -> Vec<Piece<P>> {
    let Ok(pieces) = try_sentence_chunks(sentence, max_tokens, &mut |text: &str| Ok::<_, Infallible>(g2p(text)));
    pieces
}

/// [`sentence_chunks`] with a G2P that can fail; the first error ends the sentence.
pub fn try_sentence_chunks<P: Phonemes, E, F: FnMut(&str) -> Result<P, E>>(
    sentence: &str,
    max_tokens: usize,
    g2p: &mut F,
) -> Result<Vec<Piece<P>>, E> {
    let whole = phonemize(g2p, sentence)?;
    if phoneme_len(&whole.phonemes) <= max_tokens {
        return Ok(vec![whole]);
    }
    let mut out = Vec::new();
    let mut group: Vec<Piece<P>> = Vec::new();
    let mut size = 0;
    for clause in clauses(sentence) {
        let piece = phonemize(g2p, &clause)?;
        let len = phoneme_len(&piece.phonemes);
        if !group.is_empty() && size + 1 + len > max_tokens {
            out.push(join(std::mem::take(&mut group), g2p)?);
            size = 0;
        }
        size += len + usize::from(!group.is_empty());
        group.push(piece);
    }
    if !group.is_empty() {
        out.push(join(group, g2p)?);
    }
    Ok(out)
}

/// `split_clauses`' texts: each clause stripped, with the punctuation that ended it.
fn clauses(sentence: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = 0;
    let mut push = |text: &str, punctuation: &str| {
        let clause = py::strip(text);
        if !clause.is_empty() {
            out.push(format!("{clause}{punctuation}"));
        }
    };
    for m in CLAUSE.find_iter(sentence) {
        push(&sentence[last..m.start()], m.as_str());
        last = m.end();
    }
    push(&sentence[last..], "");
    out
}

/// `_join`: one piece as it is, several phonemized again as one text, since G2P reads a word
/// differently with its neighbours.
fn join<P, E, F: FnMut(&str) -> Result<P, E>>(mut group: Vec<Piece<P>>, g2p: &mut F) -> Result<Piece<P>, E> {
    if group.len() == 1 {
        return Ok(group.pop().unwrap());
    }
    let text = group.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" ");
    phonemize(g2p, &text)
}

/// `smart_split`: pause tags, emoji removal, line joining, normalization, then one chunk per
/// sentence (or per packed run of clauses), phonemized once. Lazy: G2P runs as chunks are taken.
pub fn smart_split<'a, P: Phonemes, F: FnMut(&str) -> P>(
    text: &str,
    lang_code: &'a str,
    options: &'a Options,
    limits: &'a Limits,
    max_tokens: usize,
    mut g2p: F,
) -> impl Iterator<Item = Chunk<P>> + use<'a, P, F> {
    try_smart_split(text, lang_code, options, limits, max_tokens, move |t: &str| Ok::<_, Infallible>(g2p(t)))
        .map(|chunk| match chunk {
            Ok(chunk) => chunk,
        })
}

/// [`smart_split`] with a G2P that can fail. An error takes the place of the chunks of the
/// sentence whose G2P failed; later sentences still follow.
pub struct SmartSplit<'a, P, F> {
    parts: std::vec::IntoIter<Option<String>>,
    lang_code: &'a str,
    options: &'a Options,
    limits: &'a Limits,
    max_tokens: usize,
    g2p: F,
    sentences: VecDeque<String>,
    ready: VecDeque<Chunk<P>>,
    pending_pause: Option<String>,
}

pub fn try_smart_split<'a, P: Phonemes, E, F: FnMut(&str) -> Result<P, E>>(
    text: &str,
    lang_code: &'a str,
    options: &'a Options,
    limits: &'a Limits,
    max_tokens: usize,
    g2p: F,
) -> SmartSplit<'a, P, F> {
    let parts: Vec<Option<String>> = PAUSE_TAG.split(text).into_iter().map(|p| p.map(str::to_owned)).collect();
    SmartSplit {
        parts: parts.into_iter(),
        lang_code,
        options,
        limits,
        max_tokens,
        g2p,
        sentences: VecDeque::new(),
        ready: VecDeque::new(),
        pending_pause: None,
    }
}

impl<P, F> SmartSplit<'_, P, F> {
    fn start_part(&mut self, raw: &str) {
        if py::strip(raw).is_empty() {
            return;
        }
        let raw = if self.options.remove_emoji { strip_emoji(raw) } else { raw.to_owned() };
        let processed = normalize(&join_lines(&raw), self.lang_code, self.options);
        self.sentences.extend(sentences(&processed));
    }
}

impl<P: Phonemes, E, F: FnMut(&str) -> Result<P, E>> Iterator for SmartSplit<'_, P, F> {
    type Item = Result<Chunk<P>, E>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(chunk) = self.ready.pop_front() {
                return Some(Ok(chunk));
            }
            if let Some(sentence) = self.sentences.pop_front() {
                let sentence = py::strip(&sentence);
                if !sentence.is_empty() {
                    let pieces = match try_sentence_chunks(sentence, self.max_tokens, &mut self.g2p) {
                        Ok(pieces) => pieces,
                        Err(e) => return Some(Err(e)),
                    };
                    self.ready.extend(
                        pieces.into_iter().filter(|p| !p.phonemes.phonemes().is_empty()).map(Chunk::Speech),
                    );
                }
                continue;
            }
            if let Some(duration) = self.pending_pause.take() {
                let duration = parse_float(&duration).min(self.limits.max_pause_duration_s);
                if duration > 0.0 {
                    return Some(Ok(Chunk::Pause(duration)));
                }
                continue;
            }
            let raw = self.parts.next()?.expect("text between pause tags");
            self.pending_pause = self.parts.next().map(|d| d.expect("the pause tag's one group"));
            self.start_part(&raw);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_wrapped_lines_and_ends_paragraphs() {
        assert_eq!(join_lines("one\ntwo\n\n  three\r\n\r\nfour!\n\nfive"), "one two. three. four! five");
        assert_eq!(join_lines("\"quoted\"\n\n'"), "\"quoted\". '");
    }

    #[test]
    fn annotations_stay_whole() {
        let s = sentences("Say [Hi!](/hˈI/) now. Then stop.");
        assert_eq!(s, ["Say [Hi!](/hˈI/) now. ", "Then stop."]);
    }

    #[test]
    fn huge_placeholder_indices_are_text() {
        // Python's int() refuses more than 4300 digits and fails the request (docs/divergences.md).
        let text = format!("a\u{e000}{}\u{e001} b. [x](/y/) c.", "9".repeat(5000));
        assert_eq!(sentences(&text).concat(), text);
    }

    #[test]
    fn long_sentences_split_at_clauses() {
        let mut calls = Vec::new();
        let g2p = |t: &str| {
            calls.push(t.to_owned());
            "x".repeat(t.len())
        };
        let chunks: Vec<_> =
            smart_split("aaaa, bbbb, cccc. [pause:1.5s] Next.", "z", &Options::default(), &Limits::default(), 12, g2p)
                .collect();
        let shape: Vec<_> = chunks
            .iter()
            .map(|c| match c {
                Chunk::Speech(p) => p.text.clone(),
                Chunk::Pause(s) => format!("<{s}>"),
            })
            .collect();
        assert_eq!(shape, ["aaaa, bbbb,", "cccc.", "<1.5>", "Next."]);
        assert_eq!(calls, ["aaaa, bbbb, cccc.", "aaaa,", "bbbb,", "cccc.", "aaaa, bbbb,", "Next."]);
    }

    #[test]
    fn a_g2p_error_stands_in_for_its_sentence() {
        let g2p = |t: &str| if t.contains('!') { Err(t.to_owned()) } else { Ok("x".repeat(t.len())) };
        let chunks: Vec<_> =
            try_smart_split("aaaa, bb!, cccc. Next.", "z", &Options::default(), &Limits::default(), 12, g2p)
                .map(|c| c.map(|c| match c {
                    Chunk::Speech(p) => p.text,
                    Chunk::Pause(s) => format!("<{s}>"),
                }))
                .collect();
        assert_eq!(chunks, [Err("aaaa, bb!, cccc.".to_owned()), Ok("Next.".to_owned())]);
    }
}
