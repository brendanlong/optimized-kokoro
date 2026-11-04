//! Python semantics the reference server relies on: `str` predicates, `float()` and `repr(float)`.

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

/// The shortest digits that read back as `x`, as Python picks them: of the shortest strings, the
/// one nearest `x`, ties to an even last digit. Rust's shortest form breaks those ties the other
/// way, and its fixed-precision form rounds the exact value ties-to-even, so this takes the length
/// from the first and the digits from the second.
fn shortest_sci(x: f64) -> String {
    let shortest = format!("{x:e}");
    let mantissa = shortest.split_once('e').unwrap().0;
    let digits = mantissa.bytes().filter(u8::is_ascii_digit).count();
    let nearest = format!("{:.*e}", digits - 1, x);
    if nearest.parse::<f64>() == Ok(x) { nearest } else { shortest }
}

/// `repr(float)`: the shortest round-trip digits, in exponent form below 1e-4 or from 1e16.
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    let sci = shortest_sci(x);
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let (sign, mantissa) = mantissa.strip_prefix('-').map_or(("", mantissa), |m| ("-", m));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if (-4..16).contains(&exp) {
        let n = digits.len() as i32;
        let body = if exp < 0 {
            format!("0.{}{}", "0".repeat((-exp - 1) as usize), digits)
        } else if exp + 1 >= n {
            format!("{}{}.0", digits, "0".repeat((exp + 1 - n) as usize))
        } else {
            let (a, b) = digits.split_at((exp + 1) as usize);
            format!("{a}.{b}")
        };
        format!("{sign}{body}")
    } else {
        let mantissa = if digits.len() == 1 { digits } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        format!("{sign}{mantissa}e{}{:02}", if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

/// Errors Python raises converting a float to an int (`round`, `int`, `math.floor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotFinite;

/// `str(int(x))` for an integral float. Integers past `u128` are reported as `None` digits; every
/// caller turns those into words and inflect rejects anything over 36 digits anyway.
pub fn int_str(x: f64) -> Result<Option<String>, NotFinite> {
    if !x.is_finite() {
        return Err(NotFinite);
    }
    let magnitude = x.abs();
    if magnitude >= 2f64.powi(128) {
        return Ok(None);
    }
    let digits = (magnitude as u128).to_string();
    Ok(Some(if x < 0.0 && digits != "0" { format!("-{digits}") } else { digits }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (x, s) in [
            (1.0, "1.0"),
            (0.5, "0.5"),
            (1e-5, "1e-05"),
            (0.0001, "0.0001"),
            (1e16, "1e+16"),
            (1234567890123456.0, "1234567890123456.0"),
            (1.5e300, "1.5e+300"),
            (-3.25, "-3.25"),
            (123.456, "123.456"),
            (624802084152476.0 + 0.25, "624802084152476.2"),
            (-0.0, "-0.0"),
        ] {
            assert_eq!(float_repr(x), s);
        }
    }

    /// `reference/fixtures/float_repr.txt`, or `KOKORO_FLOAT_REPR` for a bigger run of the same script.
    #[test]
    fn float_repr_fixture() {
        let path = std::env::var("KOKORO_FLOAT_REPR")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/fixtures/float_repr.txt").into());
        let text = std::fs::read_to_string(path).unwrap();
        let mismatches: Vec<_> = text
            .lines()
            .filter_map(|line| {
                let (bits, expected) = line.split_once(' ').unwrap();
                let actual = float_repr(f64::from_bits(u64::from_str_radix(bits, 16).unwrap()));
                (actual != expected).then(|| format!("{expected} != {actual}"))
            })
            .collect();
        assert!(mismatches.is_empty(), "{} differ: {:?}", mismatches.len(), &mismatches[..mismatches.len().min(10)]);
    }

    #[test]
    fn digits() {
        assert_eq!(digit_value('٣'), Some(3));
        assert_eq!(digit_value('a'), None);
    }
}
