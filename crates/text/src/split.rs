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
    let paragraphs: Vec<String> = PARAGRAPH
        .split(text)
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
    /// Where the chunk was cut from its sentence: `before`, the sentence started in an earlier
    /// chunk; `after`, it goes on in the next, and this chunk ends at clause punctuation. Both
    /// false for a whole sentence.
    pub split: Split,
}

/// Which sides of a chunk [`sentence_chunks`] cut at clause punctuation (see [`Piece::split`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Split {
    pub before: bool,
    pub after: bool,
}

/// One item of `smart_split`'s output.
#[derive(Debug, Clone, PartialEq)]
pub enum Chunk<P> {
    Speech(Piece<P>),
    /// Seconds of silence from a `[pause:Xs]` tag.
    Pause(f64),
}

fn phonemize<P, E, F: FnMut(&str) -> Result<P, E>>(g2p: &mut F, text: &str) -> Result<Piece<P>, E> {
    Ok(Piece { text: text.to_owned(), phonemes: g2p(text)?, split: Split::default() })
}

/// The fewest phonemes a chunk cut from a sentence at clauses may have. Kokoro drawls a short
/// input rendered alone: over 501 clauses of 90 Gutenberg sentences rendered alone and in their
/// sentence (`worker/bench/clause_short.py`), the median clause alone took 1.91 times as long at 1-5
/// ids, 1.24 at 11-15, 1.08 at 21-24, and 1.00-1.03 from 25 on, about as long as in its sentence.
/// Brendan heard a short clause alone ring. A group of clauses shorter than this gets a neighbour's
/// clause (a divergence, `docs/divergences.md`).
pub const MIN_CLAUSE_CHUNK: usize = 25;

/// `sentence_chunks`: the sentence whole, or cut at clause punctuation and repacked when its
/// phonemes are longer than `max_tokens`. Each piece says where it was cut ([`Piece::split`]).
///
/// Unlike the reference, no chunk cut from a sentence has fewer than [`MIN_CLAUSE_CHUNK`] phonemes
/// (see [`group`]). The reference reads each group as soon as the clause after it doesn't fit; this
/// reads every clause first, then the groups, so the G2P calls come in another order.
pub fn sentence_chunks<P: Phonemes, F: FnMut(&str) -> P>(
    sentence: &str,
    max_tokens: usize,
    g2p: &mut F,
) -> Vec<Piece<P>> {
    let Ok(pieces) = try_sentence_chunks(sentence, max_tokens, &mut |text: &str| Ok::<_, Infallible>(g2p(text)));
    pieces
}

/// [`sentence_chunks`] with a G2P that can fail; the first error ends the sentence.
pub fn try_sentence_chunks<P: Phonemes, E, F: FnMut(&str) -> Result<P, E>>(
    sentence: &str,
    max_tokens: usize,
    g2p: &mut F,
) -> Result<Vec<Piece<P>>, E> {
    try_sentence_chunks_grouped(sentence, max_tokens, max_tokens, g2p)
}

/// [`try_sentence_chunks`] packing the clauses of a sentence longer than `max_tokens` into groups
/// of at most `groups` phonemes instead of `max_tokens` (the server packs chunks to a smaller size
/// than the most a chunk may have, `SmartSplit::with_clause_groups`). A sentence of at most
/// `max_tokens` phonemes is still one piece.
pub fn try_sentence_chunks_grouped<P: Phonemes, E, F: FnMut(&str) -> Result<P, E>>(
    sentence: &str,
    max_tokens: usize,
    groups: usize,
    g2p: &mut F,
) -> Result<Vec<Piece<P>>, E> {
    let whole = phonemize(g2p, sentence)?;
    if phoneme_len(&whole.phonemes) <= max_tokens {
        return Ok(vec![whole]);
    }
    let mut pieces = Vec::new();
    for clause in clauses(sentence) {
        pieces.push(phonemize(g2p, &clause)?);
    }
    let lens: Vec<usize> = pieces.iter().map(|p| phoneme_len(&p.phonemes)).collect();
    let mut pieces = pieces.into_iter();
    let mut out = Vec::new();
    for range in group(&lens, groups.min(max_tokens)) {
        out.push(join(pieces.by_ref().take(range.len()).collect(), g2p)?);
    }
    mark_splits(&mut out);
    Ok(out)
}

/// Packs clauses of `lens` phonemes into consecutive groups of at most `max` (counting a space
/// between clauses), as the reference does: each clause that fits joins the group before it.
/// Then, where a group has fewer than [`MIN_CLAUSE_CHUNK`] phonemes (a divergence), it takes the
/// next group's first clause if they fit together, else the previous group's last clause if they
/// fit and what is left of that group is long enough (or nothing), else it joins a neighbour over
/// the limit, which the server then stitches from pieces rendered with words around them.
fn group(lens: &[usize], max: usize) -> Vec<std::ops::Range<usize>> {
    let size = |r: std::ops::Range<usize>| lens[r.clone()].iter().sum::<usize>() + r.len().saturating_sub(1);
    let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
    for i in 0..lens.len() {
        match groups.last_mut() {
            Some(last) if size(last.start..i + 1) <= max => last.end = i + 1,
            _ => groups.push(i..i + 1),
        }
    }
    let mut i = 0;
    while groups.len() > 1 && i < groups.len() {
        let g = groups[i].clone();
        if size(g.clone()) >= MIN_CLAUSE_CHUNK {
            i += 1;
        } else if i + 1 < groups.len() && size(g.start..g.end + 1) <= max {
            groups[i].end += 1;
            groups[i + 1].start += 1;
            if groups[i + 1].is_empty() {
                groups.remove(i + 1);
            }
        } else if i > 0 && size(g.start - 1..g.end) <= max && {
            let rest = groups[i - 1].start..g.start - 1;
            rest.is_empty() || size(rest) >= MIN_CLAUSE_CHUNK
        } {
            groups[i - 1].end -= 1;
            groups[i].start -= 1;
            if groups[i - 1].is_empty() {
                groups.remove(i - 1);
                i -= 1;
            }
        } else if i + 1 < groups.len() {
            groups[i].end = groups[i + 1].end;
            groups.remove(i + 1);
        } else {
            groups[i - 1].end = g.end;
            groups.remove(i);
        }
    }
    groups
}

/// Marks which of a sentence's chunks were cut from it at clause punctuation: all but the last
/// end at it, and all but the first follow one.
fn mark_splits<P>(pieces: &mut [Piece<P>]) {
    let last = pieces.len().saturating_sub(1);
    for (i, piece) in pieces.iter_mut().enumerate() {
        piece.split = Split { before: i > 0, after: i < last };
    }
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
    try_smart_split(text, lang_code, options, limits, max_tokens, move |t: &str| Ok::<_, Infallible>(g2p(t))).map(
        |chunk| match chunk {
            Ok(chunk) => chunk,
        },
    )
}

/// [`smart_split`] with a G2P that can fail. An error takes the place of the chunks of the
/// sentence whose G2P failed; later sentences still follow.
pub struct SmartSplit<'a, P, F> {
    parts: std::vec::IntoIter<Option<String>>,
    lang_code: &'a str,
    options: &'a Options,
    limits: &'a Limits,
    max_tokens: usize,
    /// The size clause groups are packed to ([`SmartSplit::with_clause_groups`]).
    clause_groups: usize,
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
        clause_groups: max_tokens,
        g2p,
        sentences: VecDeque::new(),
        ready: VecDeque::new(),
        pending_pause: None,
    }
}

impl<P, F> SmartSplit<'_, P, F> {
    /// Packs the clauses of a sentence cut at clauses (one longer than `max_tokens`) into groups
    /// of at most `groups` phonemes (by default `max_tokens`, as the reference does). The server
    /// packs its chunks to a soft size, a sentence's clause groups included, and never cuts a
    /// sentence that fits `max_tokens` (`docs/divergences.md`).
    pub fn with_clause_groups(mut self, groups: usize) -> Self {
        self.clause_groups = groups;
        self
    }

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
                    let mut pieces =
                        match try_sentence_chunks_grouped(sentence, self.max_tokens, self.clause_groups, &mut self.g2p)
                        {
                            Ok(pieces) => pieces,
                            Err(e) => return Some(Err(e)),
                        };
                    // A piece without phonemes has no audio, so its neighbours' audio meets.
                    pieces.retain(|p| !p.phonemes.phonemes().is_empty());
                    mark_splits(&mut pieces);
                    self.ready.extend(pieces.into_iter().map(Chunk::Speech));
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

    /// A clause of `n` copies of `c`, the fake G2P's phonemes being the text's characters.
    fn words(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    /// The chunks of `text` with the fake G2P and limit `limit`: (text, split before, split after).
    fn cut(text: &str, limit: usize) -> Vec<(String, bool, bool)> {
        smart_split(text, "z", &Options::default(), &Limits::default(), limit, |t: &str| "x".repeat(t.len()))
            .map(|c| match c {
                Chunk::Speech(p) => (p.text, p.split.before, p.split.after),
                Chunk::Pause(_) => unreachable!(),
            })
            .collect()
    }

    #[test]
    fn long_sentences_split_at_clauses() {
        let (a, b, c) = (words('a', 30), words('b', 30), words('c', 30));
        let text = format!("{a}, {b}, {c}. [pause:1.5s] Next.");
        let mut calls = Vec::new();
        let g2p = |t: &str| {
            calls.push(t.to_owned());
            "x".repeat(t.len())
        };
        let chunks: Vec<_> = smart_split(&text, "z", &Options::default(), &Limits::default(), 70, g2p).collect();
        let shape: Vec<_> = chunks
            .iter()
            .map(|c| match c {
                Chunk::Speech(p) => (p.text.clone(), p.split.before, p.split.after),
                Chunk::Pause(s) => (format!("<{s}>"), false, false),
            })
            .collect();
        let s = |t: &str, before, after| (t.to_owned(), before, after);
        let ab = format!("{a}, {b},");
        assert_eq!(
            shape,
            [s(&ab, false, true), s(&format!("{c}."), true, false), s("<1.5>", false, false), s("Next.", false, false)]
        );
        // The reference's calls: the sentence, each clause, then each group of several.
        let sentence = format!("{a}, {b}, {c}.");
        assert_eq!(calls, [&sentence, &format!("{a},"), &format!("{b},"), &format!("{c}."), &ab, "Next."]);
    }

    /// The limit is the caller's: the server lowers it when its worker takes fewer ids.
    #[test]
    fn clause_splits_follow_the_limit() {
        let (a, b, c, d) = (words('a', 30), words('b', 30), words('c', 30), words('d', 30));
        let text = format!("{a}, {b}; {c}: {d}.");
        let s = |t: String, before, after| (t, before, after);
        assert_eq!(cut(&text, 450), [s(text.clone(), false, false)]);
        assert_eq!(cut(&text, 70), [s(format!("{a}, {b};"), false, true), s(format!("{c}: {d}."), true, false)]);
        assert_eq!(
            cut(&text, 0),
            [
                s(format!("{a},"), false, true),
                s(format!("{b};"), true, true),
                s(format!("{c}:"), true, true),
                s(format!("{d}."), true, false)
            ]
        );
    }

    /// Clause groups can be packed smaller than the limit; a sentence within the limit stays whole.
    #[test]
    fn clause_groups_can_be_smaller_than_the_limit() {
        let (a, b, c, d) = (words('a', 30), words('b', 30), words('c', 30), words('d', 30));
        let text = format!("{a}, {b}; {c}: {d}.");
        let cut = |limit, groups| -> Vec<String> {
            try_smart_split(&text, "z", &Options::default(), &Limits::default(), limit, |t: &str| {
                Ok::<_, Infallible>("x".repeat(t.len()))
            })
            .with_clause_groups(groups)
            .map(|c| match c.unwrap() {
                Chunk::Speech(p) => p.text,
                Chunk::Pause(_) => unreachable!(),
            })
            .collect()
        };
        assert_eq!(cut(450, 40), std::slice::from_ref(&text));
        assert_eq!(cut(100, 100), [format!("{a}, {b}; {c}:"), format!("{d}.")]);
        assert_eq!(cut(100, 40), [format!("{a},"), format!("{b};"), format!("{c}:"), format!("{d}.")]);
        // Never larger than the limit.
        assert_eq!(cut(70, 450), cut(70, 70));
    }

    /// A group of clauses of fewer than `MIN_CLAUSE_CHUNK` phonemes, which the reference would make a
    /// chunk, takes the next clause if they fit, else the previous one, else joins a neighbour over
    /// the limit (to be stitched). Otherwise the reference's groups stay.
    #[test]
    fn short_clauses_never_stand_alone() {
        let texts = |chunks: Vec<(String, bool, bool)>| chunks.into_iter().map(|c| c.0).collect::<Vec<_>>();
        let (x48, x60, x89, x97) = (words('x', 48), words('x', 59), words('x', 89), words('x', 97));
        let (y59, y97) = (words('y', 59), words('y', 97));
        // "he," is 3 phonemes. Where the reference doesn't leave it alone, nothing changes.
        assert_eq!(texts(cut(&format!("{x89}, he, {y59}."), 100)), [format!("{x89}, he,"), format!("{y59}.")]);
        assert_eq!(texts(cut(&format!("{x60}, he, {y59}."), 70)), [format!("{x60}, he,"), format!("{y59}.")]);
        // The reference cuts "x…," | "he," | "y….": neither neighbour fits, so it joins the next, over
        // the limit.
        assert_eq!(texts(cut(&format!("{x97}, he, {y97}."), 100)), [format!("{x97},"), format!("he, {y97}.")]);
        // It takes the previous chunk's last clause when that chunk keeps enough.
        assert_eq!(
            texts(cut(&format!("{x48}, {x48}, he, {y97}."), 100)),
            [format!("{x48},"), format!("{x48}, he,"), format!("{y97}.")]
        );
        // Last: with the previous over the limit, so here the sentence stays whole.
        assert_eq!(texts(cut(&format!("{x97}, he."), 100)), [format!("{x97}, he.")]);
        // First: it takes the next clause when they fit.
        assert_eq!(texts(cut(&format!("he, {x60}, {y97}."), 100)), [format!("he, {x60},"), format!("{y97}.")]);
        // Short clauses in a row are one group: 4 × 6 + 3 spaces is 27, long enough.
        let shorts = "aaaaa, bbbbb, ccccc, ddddd,";
        assert_eq!(texts(cut(&format!("{shorts} {x97}."), 100)), [shorts.to_owned(), format!("{x97}.")]);
        // A clause without phonemes is short too, so it never makes a chunk without audio.
        let silent = |t: &str| if t == "bbbb," { String::new() } else { "x".repeat(t.len()) };
        let chunks: Vec<_> =
            smart_split(&format!("{x97}, bbbb, {y97}."), "z", &Options::default(), &Limits::default(), 98, silent)
                .map(|c| match c {
                    Chunk::Speech(p) => p.text,
                    Chunk::Pause(_) => unreachable!(),
                })
                .collect();
        assert_eq!(chunks, [format!("{x97},"), format!("bbbb, {y97}.")]);
        // With a limit below the minimum, every chunk is over it.
        assert!(cut("aa, bb, cc, dd, ee, ff, gg, hh, ii, jj, kk, ll.", 10).iter().all(|c| c.0.len() >= 25));
    }

    /// The grouping by itself: greedy as the reference, then no group under the minimum.
    #[test]
    fn groups_are_greedy_then_never_short() {
        assert_eq!(group(&[40, 40, 40], 100), [0..2, 2..3]);
        assert_eq!(group(&[90, 3, 59], 100), [0..2, 2..3]);
        assert_eq!(group(&[97, 3, 97], 100), [0..1, 1..3]);
        assert_eq!(group(&[48, 48, 3, 97], 100), [0..1, 1..3, 3..4]);
        assert_eq!(group(&[97, 3], 100), vec![0..2]);
        assert_eq!(group(&[3; 40], 0).iter().map(|r| r.len()).collect::<Vec<_>>(), [7, 7, 7, 7, 12]);
        assert!(group(&[], 100).is_empty());
    }

    #[test]
    fn a_g2p_error_stands_in_for_its_sentence() {
        let g2p = |t: &str| if t.contains('!') { Err(t.to_owned()) } else { Ok("x".repeat(t.len())) };
        let chunks: Vec<_> =
            try_smart_split("aaaa, bb!, cccc. Next.", "z", &Options::default(), &Limits::default(), 12, g2p)
                .map(|c| {
                    c.map(|c| match c {
                        Chunk::Speech(p) => p.text,
                        Chunk::Pause(s) => format!("<{s}>"),
                    })
                })
                .collect();
        assert_eq!(chunks, [Err("aaaa, bb!, cccc.".to_owned()), Ok("Next.".to_owned())]);
    }
}
