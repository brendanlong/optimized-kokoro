//! Footnote and citation marks, and decorative marks, that the text keeps from its web page and
//! that aren't meant to be read (`docs/divergences.md`, "Text: footnote marks"). Each is dropped
//! only where its shape says what it is:
//!
//! - `↩` (with a number after it, "↩2"): a footnote's link back to the text.
//! - One or two digits glued to the punctuation after a lowercase word and followed by a space or
//!   the end ("we're going to find.6 None", "accessible,13 machine"): a footnote marker. Not after
//!   an abbreviation that takes a number ("No.5", "vol.3", "fig.2").
//! - A bracketed number or list of numbers after a word and a space ("a paper from Google [2].",
//!   "[3, 4]", "[5-7]"): a citation, as the glued kind ("ended.[3]") already was.
//! - `¶` that isn't before a number (a heading's anchor link), runs of `§` or `♦` used as section
//!   breaks ("§ § §"), and `♦` or `◆` starting a sentence (a bullet).
//!
//! Some of these shapes are also content: digits after a comma or semicolon ("the total was
//! large,15 of them broken") and a bracket before punctuation ("Press [1]. Then press [2].",
//! "arr [3]."). Those are dropped only in a text with at least two clear footnote marks: `↩`,
//! digits glued to a sentence's end ("find.6 None"), or a bracket between two sentences ("around.
//! [4] Most").

/// Abbreviations a number follows: "No.5" isn't a footnote.
const NUMBERED: &[&str] = &[
    "no", "nos", "fig", "figs", "vol", "vols", "eq", "eqs", "ch", "chap", "sec", "sect", "pp", "op", "art", "para",
    "ex", "ref", "refs", "nr", "pt", "pg", "cf", "ver", "rev", "ed", "eds", "item", "step", "part", "page", "line",
    "table", "chapter", "section", "version", "appendix", "rule", "note", "ss", "approx", "ca", "al",
];

fn is_space(c: char) -> bool {
    c.is_whitespace()
}

/// The footnote pass.
pub fn footnotes(text: &str) -> String {
    if !text.contains(|c: char| c.is_ascii_digit() || matches!(c, '↩' | '¶' | '§' | '♦' | '◆')) {
        return text.to_owned();
    }
    let t: Vec<char> = text.chars().collect();
    let mut marks = Vec::new();
    let mut i = 0;
    while i < t.len() {
        match debris(&t, i) {
            Some((end, mark)) => {
                marks.push((i, end, mark));
                i = end;
            }
            None => i += 1,
        }
    }
    // Shapes that may be content ("large,15 of them", "Press [1].") go only in a text whose other
    // marks show it has footnotes.
    let footnoted = marks.iter().filter(|m| m.2 == Mark::Clear).count() >= 2;
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, end, mark) in marks {
        if mark == Mark::Ambiguous && !footnoted {
            continue;
        }
        out.extend(&t[last..start]);
        // A mark standing between spaces goes with the space before it.
        if out.ends_with(' ') && t.get(end).is_none_or(|&c| is_space(c)) {
            out.pop();
        }
        last = end;
    }
    out.extend(&t[last..]);
    out
}

/// What a mark that may be dropped is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    /// A footnote mark by its shape alone: `↩`, "find.6 None", "around. [4] Most".
    Clear,
    /// A footnote mark only in a text with others: "accessible,13 machine", "Google [2].", which
    /// may also be a number in a list or a key to press ("Press [1].").
    Ambiguous,
    /// A decorative mark: `¶` after a heading, "§ § §", a `♦` bullet.
    Decoration,
}

fn digits_end(t: &[char], i: usize) -> usize {
    let mut j = i;
    while t.get(j).is_some_and(char::is_ascii_digit) {
        j += 1;
    }
    j
}

/// Whether a sentence starts at `i`: the text's start, or after `.`, `!`, `?` or `:` and spaces.
fn sentence_start(t: &[char], i: usize) -> bool {
    let mut k = i;
    while k > 0 && is_space(t[k - 1]) {
        k -= 1;
    }
    k == 0 || matches!(t[k - 1], '.' | '!' | '?' | ':')
}

/// The end of the debris starting at `i`, if some does.
fn debris(t: &[char], i: usize) -> Option<(usize, Mark)> {
    let c = t[i];
    match c {
        '↩' => {
            let mut end = i + 1;
            while t.get(end).is_some_and(|&c| matches!(c, '\u{FE0E}' | '\u{FE0F}')) {
                end += 1;
            }
            Some((digits_end(t, end), Mark::Clear))
        }
        '¶' => {
            let next = if t.get(i + 1) == Some(&' ') { i + 2 } else { i + 1 };
            (!t.get(next).is_some_and(char::is_ascii_digit)).then_some((i + 1, Mark::Decoration))
        }
        '§' | '♦' | '◆' => {
            // A run of the same mark with spaces between ("§ § §", "♦♦♦").
            let mut end = i + 1;
            let mut count = 1;
            loop {
                let mut k = end;
                while t.get(k) == Some(&' ') {
                    k += 1;
                }
                if t.get(k) == Some(&c) {
                    count += 1;
                    end = k + 1;
                } else {
                    break;
                }
            }
            let bullet = c != '§' && sentence_start(t, i);
            (count >= 2 || bullet).then_some((end, Mark::Decoration))
        }
        '0'..='9' if i > 0 && !t[i - 1].is_ascii_digit() => footnote_mark(t, i),
        ' ' if t.get(i + 1) == Some(&'[') => citation(t, i),
        _ => None,
    }
}

/// One or two digits at `i` that are a footnote marker: right after `.`, `,`, `;`, `!` or `?`
/// (and maybe a closing quote or bracket) that follows a lowercase word of two letters or more,
/// and followed by a space or the end. Returns the digits' end, and whether the mark is clear: after
/// a sentence's end it is ("find.6 None"), after `,` or `;` it may be a list's number ("large,15 of
/// them").
fn footnote_mark(t: &[char], i: usize) -> Option<(usize, Mark)> {
    let end = digits_end(t, i);
    if end - i > 2 || t.get(end).is_some_and(|&c| !is_space(c)) {
        return None;
    }
    let mut k = i;
    while k > 0 && matches!(t[k - 1], ')' | ']' | '"' | '”' | '\'' | '’') && i - k < 2 {
        k -= 1;
    }
    if k == 0 || !matches!(t[k - 1], '.' | ',' | ';' | '!' | '?') {
        return None;
    }
    // After a sentence's end the next word starts a sentence ("find.6 None"), so "came back.10
    // minutes later" (a missing space) and "beta.2 is out" keep their numbers.
    let mut next = end;
    while t.get(next).is_some_and(|&c| is_space(c)) {
        next += 1;
    }
    let new_sentence = t.get(next).is_none_or(|c| c.is_uppercase() || matches!(c, '"' | '“' | '(' | '['));
    if matches!(t[k - 1], '.' | '!' | '?') && !new_sentence {
        return None;
    }
    let mut start = k - 1;
    while start > 0 && matches!(t[start - 1], ')' | ']' | '"' | '”' | '\'' | '’') && k - 1 - start < 2 {
        start -= 1;
    }
    let word_end = start;
    while start > 0 && t[start - 1].is_alphabetic() {
        start -= 1;
    }
    let word: String = t[start..word_end].iter().collect::<String>();
    let glued_before = start > 0 && (t[start - 1].is_alphanumeric() || matches!(t[start - 1], '.' | '_'));
    let lowercase_end = word.chars().last().is_some_and(char::is_lowercase);
    (word.chars().count() >= 2 && lowercase_end && !glued_before && !NUMBERED.contains(&word.to_lowercase().as_str()))
        .then_some((end, if matches!(t[k - 1], '.' | '!' | '?') { Mark::Clear } else { Mark::Ambiguous }))
}

/// A bracketed citation after the space at `i`: "[2]", "[3, 4]", "[5-7]". Returns its end. It
/// follows a word or the end of a clause or quotation, and is followed by a space, the end, or
/// punctuation that isn't a bracket. A list's numbers have one or two digits: "[512, 24]" and
/// "[0, 255]" are a shape and a range in code or prose about it, and "x = [1, 2]" or "f(a, [1])"
/// aren't after a word. Clear between two sentences, ambiguous before punctuation ("Press [1].").
fn citation(t: &[char], i: usize) -> Option<(usize, Mark)> {
    let before = i.checked_sub(1).map(|k| t[k])?;
    if !(before.is_alphabetic() || matches!(before, '.' | ')' | '"' | '”' | '\'' | '’')) {
        return None;
    }
    let mut k = i + 2;
    let mut longest = 0;
    let mut count = 0;
    loop {
        let end = digits_end(t, k);
        if end == k || end - k > 3 {
            return None;
        }
        if t[k] == '0' {
            return None;
        }
        longest = longest.max(end - k);
        count += 1;
        k = end;
        match t.get(k) {
            Some(']') => break,
            Some(',') if t.get(k + 1) == Some(&' ') => k += 2,
            Some(',' | '-' | '–') => k += 1,
            _ => return None,
        }
    }
    // Before punctuation or the end; or between two sentences ("around. [4] Most").
    let between_sentences = matches!(before, '.' | '!' | '?')
        && t.get(k + 1).is_some_and(|&c| is_space(c))
        && t.get(k + 2).is_some_and(|c| c.is_uppercase());
    let after_ok = t.get(k + 1).is_none_or(|&c| ".,;:!?".contains(c)) || between_sentences;
    let mark = if between_sentences { Mark::Clear } else { Mark::Ambiguous };
    (after_ok && (count == 1 || longest <= 2)).then_some((k + 1, mark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footnote_debris_is_dropped() {
        for (text, expected) in [
            ("the earlier term. ↩ Next ↩︎ and ↩2 end ↩", "the earlier term. Next and end"),
            ("we're going to find.6 None of it", "we're going to find. None of it"),
            ("duc de Saint-Simon.3", "duc de Saint-Simon."),
            ("he said “so.”4 Then", "he said “so.” Then"),
            // One clear mark alone goes, but not the ambiguous ones with it.
            ("large,15 of them. It ends.4 Then", "large,15 of them. It ends. Then"),
            // With two clear marks, the ambiguous shapes go too.
            (
                "accessible,13 for all. It ends.4 Then ↩ a paper from Google [2]. And so [3, 4], then [5-7]. Also [12–14]",
                "accessible, for all. It ends. Then a paper from Google. And so, then. Also",
            ),
            // Mid-sentence, before a word, a bracket isn't taken for a citation ("press [1] to").
            ("press [1] to go and in [0, 1].", "press [1] to go and in [0, 1]."),
            ("floating around. [4] Most of them. [5] and", "floating around. Most of them. [5] and"),
            ("Dune 1.1.0?¶ and ¶ 12 and ¶3", "Dune 1.1.0? and ¶ 12 and ¶3"),
            ("end. § § § Next ♦♦♦ and. ♦ If you", "end. Next and. If you"),
        ] {
            assert_eq!(footnotes(text), expected, "{text}");
        }
    }

    #[test]
    fn numbers_that_belong_are_kept() {
        for text in [
            "version 3.6 and v1.2 and No.5 and vol.3 and fig.2 and p.5",
            "Python 2.7,3 and items a.5 and e.g.5 and GPT.4 and find.6.5 and find.123 x",
            "x = [1, 2] and a[3], see [a1] and [1.5]",
            "f(a, [1, 2, 3]) and np.array([[-1], [0], [1]]), a shape [512, 24]. and [0, 255] for display",
            "§ 230 and the § sign and ♦ in a sentence",
            "[1] Smith, J.",
            // Shapes that may be content, in a text without two clear footnote marks.
            "The total was large,15 of them broken. Press [1]. Then press [2].",
            "Indices are listed in brackets, e.g. arr [3]. The apples,12 oranges,7 and pears,3.",
            "machine learning accessible,13 for all, from Google [2].",
        ] {
            assert_eq!(footnotes(text), text);
        }
    }
}
