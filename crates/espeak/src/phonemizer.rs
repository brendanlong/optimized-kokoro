//! phonemizer-fork 3.3.2's espeak backend, as misaki configures it: `preserve_punctuation=True`,
//! `with_stress=True`, `tie='^'`, the default separator (words end in a space) and `strip=False`.

use kokoro_espeak_ng::{Error, Espeak, EspeakData, PHONEMES_IPA_TIE, Result};
use std::sync::{Arc, Mutex};

/// `phonemizer.punctuation._DEFAULT_MARKS`.
const MARKS: &str = ";:,.!?¡¿—…\"«»“”(){}[]";
const ESPEAK_TIE: char = '\u{361}';

pub struct Backend {
    espeak: Mutex<Espeak>,
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
        let mut espeak = Espeak::new(data);
        espeak.set_voice_by_name(&identifier)?;
        Ok(Self { espeak: Mutex::new(espeak) })
    }

    /// `EspeakWrapper.text_to_phonemes`: the non-empty clauses espeak reads in `text`, joined by
    /// spaces.
    pub fn espeak(&self, text: &str) -> Result<String> {
        let mut espeak = self.espeak.lock().unwrap_or_else(|e| e.into_inner());
        let clauses = espeak.text_to_phonemes(text, PHONEMES_IPA_TIE);
        Ok(clauses.into_iter().filter(|c| !c.is_empty()).collect::<Vec<_>>().join(" "))
    }

    /// `EspeakBackend.phonemize([text])`.
    pub fn phonemize(&self, text: &str) -> Result<Vec<String>> {
        self.phonemize_with(text, |chunk| self.espeak(chunk))
    }

    /// [`phonemize`](Self::phonemize) with `espeak` standing in for [`espeak`](Self::espeak).
    pub fn phonemize_with(&self, text: &str, mut espeak: impl FnMut(&str) -> Result<String>) -> Result<Vec<String>> {
        let (chunks, marks) = preserve(text);
        let phonemized =
            chunks.iter().map(|chunk| Ok(self.postprocess_line(&espeak(chunk)?))).collect::<Result<Vec<_>>>()?;
        Ok(restore(phonemized, &marks))
    }

    /// `EspeakBackend._postprocess_line`, with `language_switch='remove-flags'`: espeak's
    /// `(en)…(fr)` language-switch flags are dropped. The text's own parentheses are marks, kept
    /// out of what espeak reads and put back afterwards, so they're never mistaken for flags.
    fn postprocess_line(&self, line: &str) -> String {
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

/// `Punctuation.restore` with the default separator (word `' '`) and `strip=False`, for one input line.
fn restore(mut text: Vec<String>, marks: &[Mark]) -> Vec<String> {
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
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(restored, ["“h, m w!” "]);
    }

    #[test]
    fn only_marks() {
        let (chunks, marks) = preserve(" … ");
        assert!(chunks.is_empty());
        assert_eq!(restore(Vec::new(), &marks), [" … "]);
    }

    #[test]
    fn flags_are_removed_lazily() {
        assert_eq!(remove_flags("(en)fʊtbɔːl(fr) a"), "fʊtbɔːl a");
        assert_eq!(remove_flags("a () b (x"), "a () b (x");
    }
}
