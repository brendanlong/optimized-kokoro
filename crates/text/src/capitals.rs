//! Text written in capitals: where the normalizer writes its words in capitals too, and the
//! English G2P reads it cased as English usually is (`kokoro_g2p_en::G2p::sentence_case`).

use crate::py;
use std::ops::Range;

pub fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '’' || unicode_normalization::char::is_combining_mark(c)
}

/// `text` in words and what is between them, in order. A hyphen or dot between letters joins them,
/// so a compound or a spelled-out acronym is one word ("WELL-KNOWN", "S-H-A-D-E").
pub fn words(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let word = is_word_char(first);
        let mut end = first.len_utf8();
        let mut chars = rest[end..].chars().peekable();
        while let Some(c) = chars.next() {
            let joins = word && "-.".contains(c) && chars.peek().is_some_and(|&n| n.is_alphabetic());
            if is_word_char(c) != word && !joins {
                break;
            }
            end += c.len_utf8();
        }
        let (piece, after) = rest.split_at(end);
        rest = after;
        Some((piece, word))
    })
}

/// The sentences of `text`: each ends at a line break, or at whitespace after `.`, `!`, `?` or `…`
/// and any closing quotes or brackets.
fn sentences(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    let mut ended = false;
    let mut chars = text.char_indices();
    std::iter::from_fn(move || {
        for (i, c) in chars.by_ref() {
            if c == '\n' || ended && py::is_space(c) {
                let end = i + c.len_utf8();
                ended = false;
                return Some(std::mem::replace(&mut start, end)..end);
            }
            if ".!?…".contains(c) {
                ended = true;
            } else if !"\"'”’)]»".contains(c) {
                ended = false;
            }
        }
        (start < text.len()).then(|| std::mem::replace(&mut start, text.len())..text.len())
    })
}

/// The sentences of `text` written in capitals: in a run of sentences without a lowercase letter
/// that has two words of two capitals or more. One alone, like "OK" or "NASA.", is an acronym as
/// much as a sentence.
pub fn sentences_in_capitals(text: &str) -> Vec<Range<usize>> {
    let mut previous = false;
    if !text.chars().any(|c| std::mem::replace(&mut previous, c.is_uppercase()) && previous) {
        return Vec::new();
    }
    let mut runs: Vec<(Vec<Range<usize>>, usize)> = vec![(Vec::new(), 0)];
    for range in sentences(text) {
        let sentence = &text[range.clone()];
        if sentence.chars().any(char::is_lowercase) {
            runs.push((Vec::new(), 0));
            continue;
        }
        let (run, capitals) = runs.last_mut().expect("never empty");
        *capitals += words(sentence)
            .filter(|&(w, word)| word && w.chars().filter(|c| c.is_uppercase()).nth(1).is_some())
            .count();
        run.push(range);
    }
    runs.into_iter().filter(|(_, capitals)| *capitals >= 2).flat_map(|(run, _)| run).collect()
}

/// [`sentences_in_capitals`] joined into runs, without the whitespace around them.
pub fn runs(text: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for range in sentences_in_capitals(text) {
        match out.last_mut() {
            Some(last) if last.end == range.start => last.end = range.end,
            _ => out.push(range),
        }
    }
    for range in &mut out {
        let run = &text[range.clone()];
        range.end = range.start + run.trim_end_matches(py::is_space).len();
        range.start += run.len() - run.trim_start_matches(py::is_space).len();
    }
    out.retain(|r| !r.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_of_sentences_in_capitals() {
        let runs = |t: &str| runs(t).into_iter().map(|r| t[r].to_owned()).collect::<Vec<_>>();
        assert_eq!(
            runs("It's fine. NO IT ISN'T! DR. WHO? Fine.\nCHAPTER ONE"),
            ["NO IT ISN'T! DR. WHO?", "CHAPTER ONE"]
        );
        for t in ["The FBI said so.", "OK", "NASA!", "A I", "i saw it"] {
            assert!(runs(t).is_empty(), "{t}");
        }
    }
}
