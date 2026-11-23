//! phonemizer-fork 3.3.2's espeak backend, as misaki configures it: `preserve_punctuation=True`,
//! `with_stress=True`, `tie='^'`, the default separator (words end in a space) and `strip=False`.

use kokoro_espeak_ng::{Error, Espeak, EspeakData, Result};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// `phonemizer.punctuation._DEFAULT_MARKS`.
const MARKS: &str = ";:,.!?¡¿—…\"«»“”(){}[]";
const ESPEAK_TIE: char = '\u{361}';

pub struct Backend {
    espeak: Espeak,
    /// [`Espeak::names_letter`] for the accented letters met so far.
    names: HashMap<char, bool>,
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
        Ok(Self { espeak: Espeak::new(data, &identifier)?, names: HashMap::new() })
    }

    /// `EspeakWrapper.text_to_phonemes`: the non-empty clauses espeak reads in `text`, joined by
    /// spaces.
    pub fn espeak(&mut self, text: &str) -> String {
        let text = self.letters_espeak_names(text);
        let clauses = self.espeak.text_to_phonemes(&text);
        clauses.into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" ")
    }

    /// Whether espeak reads `text` as written, without composing its marks or taking accents off
    /// letters it has no name for.
    pub fn reads_as_written(&mut self, text: &str) -> bool {
        matches!(self.letters_espeak_names(text), Cow::Borrowed(_))
    }

    /// [`Espeak::names_letter`], remembered.
    fn names_letter(&mut self, c: char) -> bool {
        *self.names.entry(c).or_insert_with(|| self.espeak.names_letter(c))
    }

    /// `text` with letters and their combining marks composed ([`compose_marks`]), and each
    /// accented letter the voice has no name for written without its accents. espeak spells out a
    /// word with a letter it has no name for, that letter as its code point ("Nguyễn" as "N G U Y
    /// letter 1EC5 N"), so "ễ" and "ạ" are read as "e" and "a", and "ѝ" in Russian as "и".
    fn letters_espeak_names<'t>(&mut self, text: &'t str) -> Cow<'t, str> {
        let text = compose_marks(text);
        if text.is_ascii() {
            return text;
        }
        let mut changed = false;
        let out: String = text
            .chars()
            .map(|c| match accented_letter_base(c) {
                Some(base) if !self.names_letter(c) => {
                    changed = true;
                    base
                }
                _ => c,
            })
            .collect();
        if changed { out.into() } else { text }
    }
}

/// `text` with each letter and the combining marks after it written as the fewest characters
/// they compose to (NFC), in every script: "e" + U+0302 + U+0303 is "ễ", "И" + U+0306 is "Й".
/// Characters are only ever joined, never split: NFC would write U+0958 "क़" (a composition
/// exclusion) as "क" + U+093C. Symbols keep their marks ("=" + U+0338 isn't "≠").
pub fn compose_marks(text: &str) -> Cow<'_, str> {
    if !text.chars().any(is_combining_mark) {
        return text.into();
    }
    let mut chars = text.char_indices().peekable();
    let mut out = String::new();
    let mut copied = 0;
    while let Some((start, c)) = chars.next() {
        let mut end = start + c.len_utf8();
        while let Some(&(i, m)) = chars.peek().filter(|&&(_, m)| is_combining_mark(m)) {
            end = i + m.len_utf8();
            chars.next();
        }
        let cluster = &text[start..end];
        if c.is_alphabetic() && cluster.nfc().count() < cluster.chars().count() {
            out.push_str(&text[copied..start]);
            out.extend(cluster.nfc());
            copied = end;
        }
    }
    if copied == 0 {
        return text.into();
    }
    out.push_str(&text[copied..]);
    out.into()
}

/// The letter `c` is written with accents on, if it is one: its canonical decomposition is a
/// letter and combining marks ("ễ" is "e", "क़" is "क"; "한", whose decomposition is jamo, is
/// none).
fn accented_letter_base(c: char) -> Option<char> {
    if c.is_ascii() || !c.is_alphabetic() {
        return None;
    }
    let mut d = std::iter::once(c).nfd();
    let base = d.next()?;
    let mut marks = d.peekable();
    (marks.peek().is_some() && marks.all(is_combining_mark)).then_some(base)
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
    fn letters_are_composed_with_their_marks() {
        assert_eq!(
            compose_marks("Nguye\u{302}\u{303}n cafe\u{301} \u{418}\u{306}\u{43e}\u{440}\u{43a} \u{3b1}\u{301}."),
            "Nguyễn café Йорк ά."
        );
        for other in [
            "Nguyễn",
            "\u{915}\u{93c}",
            "\u{958}",
            "\u{958}\u{94d}",
            "\u{212a}",
            "\u{5d1}\u{5bc}",
            "\u{301}a",
            "=\u{338}",
        ] {
            assert_eq!(compose_marks(other), other);
        }
    }

    #[test]
    fn accented_letters_espeak_has_no_name_for_lose_their_accents() {
        let data = crate::default_data().unwrap();
        let mut english = Backend::new("en-us", data.clone()).unwrap();
        assert_eq!(english.letters_espeak_names("Nguyễn Thị Phạm Đồng"), "Nguyen Thi Pham Đong");
        assert_eq!(english.letters_espeak_names("cafe\u{301} ṣe\u{323}"), "café se");
        for other in ["café Dvořák", "Привет", "한국어", "a\u{2003}b", "∉ ≮ \u{385}", "=\u{338}"] {
            assert_eq!(english.letters_espeak_names(other), other);
        }
        let mut russian = Backend::new("ru", data.clone()).unwrap();
        assert_eq!(russian.letters_espeak_names("\u{438}\u{300} \u{45d} Йорк"), "и и Йорк");
        assert_eq!(russian.letters_espeak_names("ư Ư"), "u U");
        let mut hebrew = Backend::new("he", data.clone()).unwrap();
        assert_eq!(hebrew.letters_espeak_names("\u{fb31}\u{5d9}\u{5ea}"), "\u{5d1}\u{5d9}\u{5ea}");
        let mut hindi = Backend::new("hi", data).unwrap();
        for other in ["क़िला", "\u{915}\u{93c}िला"] {
            assert_eq!(hindi.letters_espeak_names(other), other);
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
