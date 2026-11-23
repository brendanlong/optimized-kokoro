//! phonemizer-fork 3.3.2's espeak backend, as misaki configures it: `preserve_punctuation=True`,
//! `with_stress=True`, `tie='^'`, the default separator (words end in a space) and `strip=False`.

use kokoro_espeak_ng::{Error, Espeak, EspeakData, Result};
use std::borrow::Cow;
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// `phonemizer.punctuation._DEFAULT_MARKS`.
const MARKS: &str = ";:,.!?¡¿—…\"«»“”(){}[]";
const ESPEAK_TIE: char = '\u{361}';

pub struct Backend {
    espeak: Espeak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Begin,
    End,
    Inner,
    Alone,
}

struct Mark<'a> {
    text: &'a str,
    position: Position,
}

impl Backend {
    /// `EspeakBackend(language)`: selects the first voice (in `espeak_ListVoices` order) whose
    /// first language is `language`, as phonemizer's `EspeakWrapper.set_voice` does.
    pub fn new(language: &str, data: Arc<EspeakData>) -> Result<Self> {
        let identifier = data
            .list_voices()
            .into_iter()
            .find(|v| v.languages.first().is_some_and(|l| l.1 == language))
            .map(|v| v.identifier.clone())
            .ok_or_else(|| Error::VoiceNotFound(language.to_string()))?;
        Ok(Self { espeak: Espeak::new(data, &identifier)? })
    }

    /// `EspeakWrapper.text_to_phonemes`: the non-empty clauses espeak reads in `text`, joined by
    /// spaces.
    pub fn espeak(&mut self, text: &str) -> String {
        let clauses = self.espeak.text_to_phonemes(&accents_espeak_knows(text));
        clauses.into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

/// The last Latin letter espeak knows with accents (`remove_accent` and `LookupAccentedLetter`
/// stop at U+024F).
const LAST_KNOWN_LATIN: char = '\u{24f}';

/// `text` with each accented Latin letter espeak doesn't know written as its letter without
/// accents. espeak spells out a word with a Latin letter beyond U+024F ("Nguyễn" as "N G U Y
/// letter 1EC5 N") and reads an accent written as a combining mark as punctuation, so a letter
/// with accents (precomposed or as combining marks) is kept only when it is one character espeak
/// knows: "é" and "e" + U+0301 are "é", "ễ" and "ạ" are "e" and "a".
fn accents_espeak_knows(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| c > LAST_KNOWN_LATIN) {
        return Cow::Borrowed(text);
    }
    let known = |letter: &str| {
        let mut composed = letter.nfc();
        match (composed.next(), composed.next()) {
            (Some(c), None) if c <= LAST_KNOWN_LATIN => c,
            _ => letter.chars().nfd().next().expect("a letter"),
        }
    };
    let mut out = String::with_capacity(text.len());
    let mut letter = String::new();
    for c in text.chars() {
        if !letter.is_empty() && is_combining_mark(c) {
            letter.push(c);
            continue;
        }
        if !letter.is_empty() {
            out.push(known(&letter));
            letter.clear();
        }
        let base = std::iter::once(c).nfd().next().expect("a character");
        if base.is_alphabetic() && base <= LAST_KNOWN_LATIN {
            letter.push(c);
        } else {
            out.push(c);
        }
    }
    if !letter.is_empty() {
        out.push(known(&letter));
    }
    Cow::Owned(out)
}

/// `EspeakBackend.phonemize([text])`, as one string, with `espeak` reading each chunk (normally
/// [`Backend::espeak`]): `None` for empty text. phonemizer returns a list, of which misaki keeps
/// the first element; for one line of text it never has more than one (`docs/divergences.md`), so
/// this joins them instead.
pub fn phonemize(text: &str, mut espeak: impl FnMut(&str) -> String) -> Option<String> {
    let (chunks, marks) = preserve(text);
    if chunks.is_empty() && marks.is_empty() {
        return None;
    }
    let phonemized = chunks.iter().map(|chunk| postprocess_line(&espeak(chunk))).collect();
    Some(restore(phonemized, &marks))
}

/// `EspeakBackend._postprocess_line`, with `language_switch='remove-flags'`: espeak's
/// `(en)…(fr)` language-switch flags are dropped. The text's own parentheses are marks, kept
/// out of what espeak reads and put back afterwards, so they're never mistaken for flags.
fn postprocess_line(line: &str) -> String {
    let line = py_strip(line).replace('\n', " ").replace("  ", " ");
    let line = remove_flags(&squeeze_underscores(&line).replace("_ ", " "));
    if line.is_empty() {
        return line;
    }
    let mut out = String::with_capacity(line.len() + 8);
    for word in line.split(' ') {
        out.push_str(&py_strip(word).replace(ESPEAK_TIE, "^"));
        out.push(' ');
    }
    out
}

/// Python's `str.isspace`, which also counts the ASCII information separators.
pub(crate) fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub(crate) fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

fn squeeze_underscores(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !(c == '_' && out.ends_with('_')) {
            out.push(c);
        }
    }
    out
}

/// `re.sub(r'\(.+?\)', '', line)`.
fn remove_flags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        let Some(first) = after.chars().next() else { break };
        match after[first.len_utf8()..].find(')') {
            Some(close) => {
                out.push_str(&rest[..open]);
                rest = &after[first.len_utf8() + close + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// The spans `(\s*[MARKS]+\s*)+` matches: each maximal run of whitespace and marks that holds a mark.
fn mark_spans(line: &str) -> Vec<(usize, usize)> {
    let is_mark = |c: char| MARKS.contains(c);
    let mut spans = Vec::new();
    let mut run: Option<(usize, bool)> = None;
    for (i, c) in line.char_indices() {
        if is_mark(c) || py_isspace(c) {
            let (start, has_mark) = run.unwrap_or((i, false));
            run = Some((start, has_mark || is_mark(c)));
        } else if let Some((start, true)) = run.take() {
            spans.push((start, i));
        }
    }
    if let Some((start, true)) = run {
        spans.push((start, line.len()));
    }
    spans
}

/// `Punctuation.preserve([line])`: the text between marks, and the marks.
fn preserve(line: &str) -> (Vec<&str>, Vec<Mark<'_>>) {
    let spans = mark_spans(line);
    if spans.is_empty() {
        return ([line].into_iter().filter(|l| !l.is_empty()).collect(), Vec::new());
    }
    if spans.len() == 1 && spans[0] == (0, line.len()) {
        return (Vec::new(), vec![Mark { text: line, position: Position::Alone }]);
    }
    let last = spans.len() - 1;
    let marks: Vec<Mark> = spans
        .iter()
        .enumerate()
        .map(|(n, &(start, end))| {
            let text = &line[start..end];
            let position = if n == 0 && line.starts_with(text) {
                Position::Begin
            } else if n == last && line.ends_with(text) {
                Position::End
            } else {
                Position::Inner
            };
            Mark { text, position }
        })
        .collect();
    let mut chunks = Vec::with_capacity(marks.len() + 1);
    let mut rest = line;
    for mark in &marks {
        match rest.find(mark.text) {
            Some(at) => {
                chunks.push(&rest[..at]);
                rest = &rest[at + mark.text.len()..];
            }
            None => {
                chunks.push(rest);
                rest = "";
            }
        }
    }
    chunks.push(rest);
    chunks.retain(|c| !c.is_empty());
    (chunks, marks)
}

/// `Punctuation.restore` with the default separator (word `' '`) and `strip=False`, for one input
/// line, with its output lines joined.
fn restore(mut text: Vec<String>, marks: &[Mark]) -> String {
    const SEP: &str = " ";
    let mut out = Vec::new();
    let mut marks = marks;
    let mut placed_end = false;
    text.reverse();
    while !text.is_empty() || !marks.is_empty() {
        if marks.is_empty() {
            while let Some(mut line) = text.pop() {
                if !line.ends_with(SEP) {
                    line.push_str(SEP);
                }
                out.push(line);
            }
        } else if text.is_empty() {
            out.push(marks.iter().map(|m| m.text).collect::<String>());
            marks = &[];
        } else if !placed_end {
            let (current, rest) = marks.split_first().unwrap();
            marks = rest;
            let mark = current.text;
            let first = text.last_mut().unwrap();
            if first.ends_with(SEP) {
                first.truncate(first.len() - SEP.len());
            }
            match current.position {
                Position::Begin => first.insert_str(0, mark),
                Position::End => {
                    let mut line = text.pop().unwrap();
                    line.push_str(mark);
                    if !mark.ends_with(SEP) {
                        line.push_str(SEP);
                    }
                    out.push(line);
                    placed_end = true;
                }
                Position::Alone => {
                    let mut line = mark.to_string();
                    if !mark.ends_with(SEP) {
                        line.push_str(SEP);
                    }
                    out.push(line);
                    placed_end = true;
                }
                Position::Inner => {
                    if text.len() == 1 {
                        text[0].push_str(mark);
                    } else {
                        // Python's `text[1] = text[0] + mark + text[1]`, growing the joined line in
                        // place so that a run of inner marks takes linear time.
                        let mut first = text.pop().unwrap();
                        let next = text.pop().unwrap();
                        first.push_str(mark);
                        first.push_str(&next);
                        text.push(first);
                    }
                }
            }
        } else {
            out.push(text.pop().unwrap());
        }
    }
    out.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_letters_keep_only_accents_espeak_knows() {
        assert_eq!(accents_espeak_knows("Nguyễn Thị Phạm Đồng"), "Nguyen Thi Pham Đong");
        assert_eq!(accents_espeak_knows("cafe\u{301} ṣe\u{323}"), "café se");
        for other in ["café Dvořák", "क़िला", "Привет", "한국어", "a\u{2003}b"] {
            assert_eq!(accents_espeak_knows(other), other);
        }
    }

    #[test]
    fn preserve_splits_at_marks() {
        let (chunks, marks) = preserve("hello, my world!");
        assert_eq!(chunks, ["hello", "my world"]);
        assert_eq!(
            marks.iter().map(|m| (m.text, m.position)).collect::<Vec<_>>(),
            [(", ", Position::Inner), ("!", Position::End)]
        );
    }

    #[test]
    fn restore_inverts_preserve() {
        let (_, marks) = preserve("“hello, my world!”");
        let restored = restore(vec!["h ".into(), "m w ".into()], &marks);
        assert_eq!(restored, "“h, m w!” ");
    }

    #[test]
    fn only_marks() {
        let (chunks, marks) = preserve(" … ");
        assert!(chunks.is_empty());
        assert_eq!(restore(Vec::new(), &marks), " … ");
    }

    #[test]
    fn flags_are_removed_lazily() {
        assert_eq!(remove_flags("(en)fʊtbɔːl(fr) a"), "fʊtbɔːl a");
        assert_eq!(remove_flags("a () b (x"), "a () b (x");
    }
}
