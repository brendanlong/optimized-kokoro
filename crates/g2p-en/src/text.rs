//! Python `str` semantics misaki depends on, with Unicode tables generated from the reference Python.

use crate::unicode_tables::{
    PY_ALPHA, PY_ASSIGNED, PY_DIGIT, PY_LOWER_CHANGES, PY_UPPER_CHANGES, RE_DIGIT, RE_L, RE_LL, RE_LU,
};
use std::borrow::Cow;
use unicode_normalization::UnicodeNormalization;

fn in_table(c: char, table: &[(u32, u32)]) -> bool {
    let c = c as u32;
    table
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

/// `c.isalpha()`: general category L*.
pub fn is_alpha(c: char) -> bool {
    if c.is_ascii() { c.is_ascii_alphabetic() } else { in_table(c, PY_ALPHA) }
}

/// `s.isalpha()`: non-empty and every character alphabetic.
pub fn str_is_alpha(s: &str) -> bool {
    !s.is_empty() && s.chars().all(is_alpha)
}

/// `c.isspace()`, which unlike Unicode White_Space includes U+001C..U+001F.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t'..='\r'
            | '\x1c'..='\x20'
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

pub fn lstrip(s: &str) -> &str {
    s.trim_start_matches(is_space)
}

pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// The value of a character for which `c.isdigit()` holds (every such character has an integer value).
pub fn digit_value(c: char) -> Option<u8> {
    if c.is_ascii_digit() {
        return Some(c as u8 - b'0');
    }
    let cp = c as u32;
    let i = PY_DIGIT.partition_point(|&(_, hi, _)| hi < cp);
    PY_DIGIT.get(i).filter(|&&(lo, _, _)| lo <= cp).map(|&(lo, _, v)| v + (cp - lo) as u8)
}

/// `\p{L}` in the `regex` module.
pub fn re_letter(c: char) -> bool {
    if c.is_ascii() { c.is_ascii_alphabetic() } else { in_table(c, RE_L) }
}

/// `\p{Lu}` in the `regex` module.
pub fn re_upper(c: char) -> bool {
    if c.is_ascii() { c.is_ascii_uppercase() } else { in_table(c, RE_LU) }
}

/// `\p{Ll}` in the `regex` module.
pub fn re_lower(c: char) -> bool {
    if c.is_ascii() { c.is_ascii_lowercase() } else { in_table(c, RE_LL) }
}

/// `\d` in the `regex` module.
pub fn re_digit(c: char) -> bool {
    if c.is_ascii() { c.is_ascii_digit() } else { in_table(c, RE_DIGIT) }
}

/// `re.match(r'^[0-9]+$', s)`; `$` also matches before a final newline.
pub fn is_ascii_digits(s: &str) -> bool {
    let s = s.strip_suffix('\n').unwrap_or(s);
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Applies `f` to the runs of `s` between characters `keep` selects, which are copied unchanged.
/// Rust's case and normalization tables follow a newer Unicode version than the reference Python, which
/// leaves characters it doesn't know (and a few whose case pairs are newer) alone.
fn around(s: &str, keep: impl Fn(char) -> bool, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if keep(c) {
            out.push_str(&f(&s[start..i]));
            out.push(c);
            start = i + c.len_utf8();
        }
    }
    out.push_str(&f(&s[start..]));
    out
}

fn python_keeps_lower(c: char) -> bool {
    !in_table(c, PY_LOWER_CHANGES) && !char_maps_to_itself(c.to_lowercase(), c)
}

fn python_keeps_upper(c: char) -> bool {
    !in_table(c, PY_UPPER_CHANGES) && !char_maps_to_itself(c.to_uppercase(), c)
}

fn char_maps_to_itself(mut mapped: impl Iterator<Item = char>, c: char) -> bool {
    mapped.next() == Some(c) && mapped.next().is_none()
}

/// `s.lower()`
pub fn lower(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        if s.bytes().any(|b| b.is_ascii_uppercase()) { Cow::Owned(s.to_ascii_lowercase()) } else { Cow::Borrowed(s) }
    } else if s.chars().any(python_keeps_lower) {
        Cow::Owned(around(s, python_keeps_lower, str::to_lowercase))
    } else {
        Cow::Owned(s.to_lowercase())
    }
}

/// `s.upper()`
pub fn upper(s: &str) -> Cow<'_, str> {
    if s.is_ascii() {
        if s.bytes().any(|b| b.is_ascii_lowercase()) { Cow::Owned(s.to_ascii_uppercase()) } else { Cow::Borrowed(s) }
    } else if s.chars().any(python_keeps_upper) {
        Cow::Owned(around(s, python_keeps_upper, str::to_uppercase))
    } else {
        Cow::Owned(s.to_uppercase())
    }
}

/// `s == s.lower()`
pub fn is_lower(s: &str) -> bool {
    if s.is_ascii() { !s.bytes().any(|b| b.is_ascii_uppercase()) } else { lower(s) == s }
}

/// `s == s.upper()`
pub fn is_upper(s: &str) -> bool {
    if s.is_ascii() { !s.bytes().any(|b| b.is_ascii_lowercase()) } else { upper(s) == s }
}

/// `c == c.lower()`
pub fn char_is_lower(c: char) -> bool {
    !in_table(c, PY_LOWER_CHANGES) || char_maps_to_itself(c.to_lowercase(), c)
}

/// `unicodedata.normalize('NFKC', s)`
pub fn nfkc(s: &str) -> Cow<'_, str> {
    let unassigned = |c: char| !in_table(c, PY_ASSIGNED);
    if s.is_ascii() {
        Cow::Borrowed(s)
    } else if s.chars().any(unassigned) {
        Cow::Owned(around(s, unassigned, |run| run.nfkc().collect()))
    } else {
        Cow::Owned(s.nfkc().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert!(is_alpha('é') && is_alpha('ß') && is_alpha('中') && !is_alpha('1') && !is_alpha('\''));
        assert!(!str_is_alpha(""));
        assert!(is_space('\x1c') && is_space('\u{3000}') && !is_space('\u{200b}'));
        assert_eq!(digit_value('٣'), Some(3));
        assert_eq!(digit_value('²'), Some(2));
        assert_eq!(digit_value('a'), None);
        assert!(re_upper('É') && re_lower('é') && !re_upper('ǅ') && re_letter('ǅ'));
        assert!(is_ascii_digits("12\n") && !is_ascii_digits("\n") && !is_ascii_digits("1a"));
        assert!(is_upper("ÉCOLE") && is_lower("1-2") && is_upper("1-2"));
        assert_eq!(upper("ɤé"), "ɤÉ");
        assert_eq!(nfkc("\u{1ccd6}ﬁ"), "\u{1ccd6}fi");
    }

    /// Every code point against Python, when `PY_CHARS_JSONL` points at tools/gen_chars.py's output.
    #[test]
    fn every_char_matches_python() {
        let Ok(path) = std::env::var("PY_CHARS_JSONL") else { return };
        let data = std::fs::read_to_string(path).unwrap();
        let mut differ = Vec::new();
        for line in data.lines() {
            let (s, l, u, alpha, space, digit, n): (String, String, String, bool, bool, Option<u8>, String) =
                serde_json::from_str(line).unwrap();
            let c = s.chars().next().unwrap();
            if lower(&s) != l
                || upper(&s) != u
                || char_is_lower(c) != (l == s)
                || is_alpha(c) != alpha
                || is_space(c) != space
                || digit_value(c) != digit
                || nfkc(&s) != n
            {
                differ.push(format!("U+{:04X}", c as u32));
            }
        }
        assert!(data.lines().count() > 1_000_000);
        assert!(differ.is_empty(), "{} differ: {:?}", differ.len(), &differ[..differ.len().min(30)]);
    }
}
