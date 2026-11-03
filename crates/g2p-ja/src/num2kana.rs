//! misaki's `num2kana.Convert(digits, 'hiragana')`: a number of up to nine digits in kana.
//!
//! Longer numbers, which `Convert` answers with an English error message (that cutlet then
//! crashes on), are read digit by digit; see [`number`].

const DIGITS: [&str; 10] = ["ゼロ", "いち", "に", "さん", "よん", "ご", "ろく", "なな", "はち", "きゅう"];
const TEN: &str = "じゅう";
const HUNDRED: &str = "ひゃく";
const THOUSAND: &str = "せん";
const MAN: &str = "まん";
const OKU: &str = "おく";

fn one(d: u8) -> &'static str {
    DIGITS[(d - b'0') as usize]
}

fn len_two(s: &[u8], out: &mut String) {
    if s[0] == b'0' {
        out.push_str(one(s[1]));
    } else if s == b"10" {
        out.push_str(TEN);
    } else if s[0] == b'1' {
        out.push_str(TEN);
        out.push_str(one(s[1]));
    } else if s[1] == b'0' {
        out.push_str(one(s[0]));
        out.push_str(TEN);
    } else {
        out.push_str(one(s[0]));
        out.push_str(TEN);
        out.push_str(one(s[1]));
    }
}

fn len_three(s: &[u8], out: &mut String) {
    match s[0] {
        b'1' => out.push_str(HUNDRED),
        b'3' => out.push_str("さんびゃく"),
        b'6' => out.push_str("ろっぴゃく"),
        b'8' => out.push_str("はっぴゃく"),
        d => {
            out.push_str(one(d));
            out.push_str(HUNDRED);
        }
    }
    if &s[1..] == b"00" && s.len() == 3 {
    } else if s[1] == b'0' {
        out.push_str(one(s[2]));
    } else {
        len_two(&s[1..], out);
    }
}

fn len_four(mut s: &[u8], stand_alone: bool, out: &mut String) {
    if s == b"0000" {
        return;
    }
    while s[0] == b'0' {
        s = &s[1..];
    }
    match s.len() {
        1 => out.push_str(one(s[0])),
        2 => len_two(s, out),
        3 => len_three(s, out),
        _ => {
            match s[0] {
                b'1' if stand_alone => out.push_str(THOUSAND),
                b'1' => out.push_str("いっせん"),
                b'3' => out.push_str("さんぜん"),
                b'8' => out.push_str("はっせん"),
                d => {
                    out.push_str(one(d));
                    out.push_str(THOUSAND);
                }
            }
            if &s[1..] == b"000" && s.len() == 4 {
            } else if s[1] == b'0' {
                len_two(&s[2..], out);
            } else {
                len_three(&s[1..], out);
            }
        }
    }
}

fn len_x(s: &[u8], out: &mut String) {
    let head = &s[..s.len() - 4];
    match head.len() {
        1 => {
            out.push_str(one(head[0]));
            out.push_str(MAN);
        }
        2 => {
            len_two(head, out);
            out.push_str(MAN);
        }
        3 => {
            len_three(head, out);
            out.push_str(MAN);
        }
        4 => {
            len_four(head, false, out);
            out.push_str(MAN);
        }
        _ => {
            out.push_str(one(s[0]));
            out.push_str(OKU);
            len_four(&s[1..5], false, out);
            if &s[1..5] != b"0000" {
                out.push_str(MAN);
            }
        }
    }
    len_four(&s[s.len() - 4..], false, out);
}

/// `Convert(digits)` for one to nine ASCII digits.
fn convert(mut s: &[u8], out: &mut String) {
    while s[0] == b'0' && s.len() > 1 {
        s = &s[1..];
    }
    match s.len() {
        1 => out.push_str(one(s[0])),
        2 => len_two(s, out),
        3 => len_three(s, out),
        4 => len_four(s, true, out),
        _ => len_x(s, out),
    }
}

/// `Convert(digits, 'hiragana')` for one to nine ASCII digits, as the reference reads them.
pub fn convert_kana(digits: &[u8]) -> String {
    let mut out = String::new();
    if !digits.is_empty() {
        convert(digits, &mut out);
    }
    out
}

/// How the port reads a number the reference can't (more than nine digits, where `Convert`
/// answers with an English error message cutlet crashes on, or digits other than ASCII's):
/// up to nine digits as `Convert` reads them; up to sixteen without a leading zero in units of
/// 万, 億 and 兆 (`10000000000` is ひゃくおく); otherwise digit by digit (an ISBN, a phone number).
/// Each item is one word's kana.
pub fn number_words(digits: &[u8]) -> Vec<String> {
    if digits.is_empty() {
        return Vec::new();
    }
    if digits.len() <= 9 {
        return vec![convert_kana(digits)];
    }
    if digits.len() <= 16 && digits[0] != b'0' {
        return vec![units(digits)];
    }
    digits.iter().map(|&d| one(d).to_owned()).collect()
}

/// Ten to sixteen digits, the first not 0, in groups of four: 兆, 億, 万 and the rest, each group
/// read as `Convert` reads the four digits before 万 (`len_four(…, False)`), with the sound
/// changes before ちょう (いっちょう, はっちょう, じゅっちょう).
fn units(digits: &[u8]) -> String {
    let n = digits.len();
    let mut out = String::new();
    let groups = [(n.saturating_sub(16), n.saturating_sub(12), "ちょう"), (n.saturating_sub(12), n - 8, "おく"), (n - 8, n - 4, "まん")];
    for (start, end, unit) in groups {
        let group = &digits[start..end];
        if group.is_empty() || group.iter().all(|&d| d == b'0') {
            continue;
        }
        let mut kana = String::new();
        len_four(&[&b"0000"[..4 - group.len()], group].concat(), false, &mut kana);
        if unit == "ちょう" {
            for (plain, geminate) in [("いち", "いっ"), ("はち", "はっ"), ("じゅう", "じゅっ")] {
                if let Some(stem) = kana.strip_suffix(plain) {
                    kana = format!("{stem}{geminate}");
                    break;
                }
            }
        }
        out.push_str(&kana);
        out.push_str(unit);
    }
    len_four(&digits[n - 4..], false, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::{convert_kana, number_words};

    #[test]
    fn numbers() {
        for (n, kana) in [
            ("0", "ゼロ"),
            ("007", "なな"),
            ("10", "じゅう"),
            ("11", "じゅういち"),
            ("20", "にじゅう"),
            ("300", "さんびゃく"),
            ("1000", "せん"),
            ("2001", "にせんいち"),
            ("10000", "いちまん"),
            ("11000", "いちまんいっせん"),
            ("100000000", "いちおく"),
            ("123456789", "いちおくにせんさんびゃくよんじゅうごまんろくせんななひゃくはちじゅうきゅう"),
        ] {
            assert_eq!(convert_kana(n.as_bytes()), kana, "{n}");
        }
        for (n, words) in [
            ("10000000000", "ひゃくおく"),
            ("1000000000000", "いっちょう"),
            ("8000000000000", "はっちょう"),
            ("10000000000000", "じゅっちょう"),
            ("1234567890123456", "いっせんにひゃくさんじゅうよんちょうごせんろっぴゃくななじゅうはちおくきゅうせんじゅうにまんさんぜんよんひゃくごじゅうろく"),
            ("0012345678", "ゼロ ゼロ いち に さん よん ご ろく なな はち"),
            ("12345678901234567", "いち に さん よん ご ろく なな はち きゅう ゼロ いち に さん よん ご ろく なな"),
            ("٣", "さん"),
        ] {
            let ascii: Vec<u8> = n.chars().map(|c| b'0' + crate::cutlet::digit_value(c).unwrap()).collect();
            assert_eq!(number_words(&ascii).join(" "), words, "{n}");
        }
    }
}
