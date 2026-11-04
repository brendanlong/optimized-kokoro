//! Python semantics the reference server relies on: `str` predicates and `float()`.

use crate::tables::{DIGIT, SPACE, WORD};

fn in_table(table: &[(char, char)], c: char) -> bool {
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

/// `str.isspace()`, Python `re`'s `\s`.
pub fn is_space(c: char) -> bool {
    in_table(SPACE, c)
}

/// Python `re`'s `\w`.
pub fn is_word(c: char) -> bool {
    in_table(WORD, c)
}

/// `str.isalnum()` for one character.
pub fn is_alnum(c: char) -> bool {
    c != '_' && is_word(c)
}

/// The value of a `str.isdecimal()` character, as `int()` and `float()` read it.
pub fn digit_value(c: char) -> Option<u32> {
    let i = DIGIT.partition_point(|&(_, hi)| hi < c);
    DIGIT.get(i).filter(|&&(lo, _)| lo <= c).map(|&(lo, _)| (c as u32 - lo as u32) % 10)
}

/// A character's case folding as Python's IGNORECASE compares it: the simple lowercase, with
/// i/ı, s/ſ and µ/μ equated.
pub(crate) fn fold(c: char) -> char {
    match c.to_lowercase().next().unwrap_or(c) {
        'ı' => 'i',
        'ſ' => 's',
        'µ' => 'μ',
        c => c,
    }
}

/// `str.strip()` with no arguments.
pub fn strip(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Python's `float()` on a string of decimal digits with at most one `.`, after `,` grouping is removed.
pub fn parse_float(s: &str) -> f64 {
    let ascii: String = s.chars().map(|c| digit_value(c).map_or(c, |d| char::from(b'0' + d as u8))).collect();
    ascii.parse().unwrap_or_else(|_| panic!("not a decimal number: {s:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits() {
        assert_eq!(digit_value('٣'), Some(3));
        assert_eq!(digit_value('a'), None);
    }
}
