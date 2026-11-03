//! cn2an 0.5.23's `transform(text, "an2cn")`: Arabic numerals to Chinese ones, in five passes
//! (dates, fractions, percentages, degrees Celsius, then any other number).
//!
//! Python's `\d` is any decimal digit (Unicode `Nd`). `an2cn` first maps full-width characters
//! to ASCII (`proces`'s `full_angle_to_half_angle`; its traditional-to-simplified table has no
//! digits) and then accepts only `0-9`, `.` and `-`, so a number in any other digits fails, and
//! the failed pass leaves its match as it was. So does an integer part of more than 16 digits.
//!
//! Each pattern is matched by hand. None of them can match in two ways from one position, so a
//! scan that skips a whole run of digits after a failed match finds the same matches as Python's
//! `re.sub`, in linear time.

use kokoro_text::py::digit_value;

const DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
const UNITS: [&str; 16] = ["", "十", "百", "千", "万", "十", "百", "千", "亿", "十", "百", "千", "万", "十", "百", "千"];

fn is_digit(c: char) -> bool {
    digit_value(c).is_some()
}

/// `transform(text, "an2cn")`.
pub fn transform(text: &str) -> String {
    let text = sub(text, date, date_skip);
    let text = sub(&text, fraction, digits);
    let text = sub(&text, percent, digits);
    let text = sub(&text, celsius, digits);
    sub(&text, number, digits)
}

/// A pattern: the end of its match at `pos` and its replacement, or `None`.
type Pattern = fn(&str, usize) -> Option<(usize, String)>;

/// `re.sub(pattern, f, text)`. On a failed match at a digit, the scan moves on by `skip(text, pos)`
/// bytes, past the positions where the pattern would fail the same way.
fn sub(text: &str, pattern: Pattern, skip: fn(&str, usize) -> usize) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut pos = 0;
    while pos < text.len() {
        if let Some((end, replacement)) = pattern(text, pos).filter(|&(end, _)| end > pos) {
            out.push_str(&replacement);
            pos = end;
            continue;
        }
        let c = text[pos..].chars().next().expect("a character");
        let skip = if is_digit(c) { skip(text, pos).max(c.len_utf8()) } else { c.len_utf8() };
        out.push_str(&text[pos..pos + skip]);
        pos += skip;
    }
    out
}

/// The byte length of the run of digits at `pos`.
fn digits(text: &str, pos: usize) -> usize {
    text[pos..].find(|c: char| !is_digit(c)).unwrap_or(text.len() - pos)
}

fn count(text: &str, pos: usize) -> usize {
    text[pos..pos + digits(text, pos)].chars().count()
}

fn at(text: &str, pos: usize, c: char) -> bool {
    text[pos..].starts_with(c)
}

/// Where a date can start in the run of digits at `pos`, if the match at `pos` failed: with at
/// most 4 digits left before 年, or 2 before 月 or 日; otherwise past the run.
fn date_skip(text: &str, pos: usize) -> usize {
    let len = digits(text, pos);
    let n = count(text, pos);
    let keep = match text[pos + len..].chars().next() {
        Some('年') if n >= 2 => 4,
        Some('月' | '日') => 2,
        _ => return len,
    };
    let skip = n.saturating_sub(keep);
    text[pos..].char_indices().nth(skip).map_or(len, |(i, _)| i)
}

/// `(\d{2,4}年)?(\d{1,2}月)?(\d{1,2}日)?`, replaced by the year's digits read one by one and the
/// month and day as numbers.
fn date(text: &str, pos: usize) -> Option<(usize, String)> {
    let mut end = pos;
    let mut parts: Vec<(usize, usize, bool)> = Vec::new();
    for (unit, lo, hi) in [('年', 2, 4), ('月', 1, 2), ('日', 1, 2)] {
        let len = digits(text, end);
        let n = count(text, end);
        if (lo..=hi).contains(&n) && at(text, end + len, unit) {
            parts.push((end, end + len, unit == '年'));
            end += len + unit.len_utf8();
        }
    }
    if end == pos {
        return None;
    }
    // The year is converted first, and kept if the month or day then fails: `__sub_util` returns
    // its `inputs`, which by then hold the converted year.
    let mut year_read: Option<(String, usize)> = None;
    let mut out = String::new();
    let mut last = pos;
    for (s, e, year) in parts {
        out.push_str(&text[last..s]);
        let digits = &text[s..e];
        let read = if year { an2cn_direct(digits) } else { an2cn(digits) };
        match read {
            Some(read) => out.push_str(&read),
            None if year => return Some((end, text[pos..end].to_owned())),
            None => {
                let (prefix, from) = match year_read {
                    Some((read, e)) => (read, e),
                    None => (String::new(), pos),
                };
                return Some((end, prefix + &text[from..end]));
            }
        }
        last = e;
        if year {
            year_read = Some((out.clone(), e));
        }
    }
    out.push_str(&text[last..end]);
    Some((end, out))
}

/// `\d+/\d+` as `{denominator}分之{numerator}`.
fn fraction(text: &str, pos: usize) -> Option<(usize, String)> {
    let a = digits(text, pos);
    if a == 0 || !at(text, pos + a, '/') {
        return None;
    }
    let b = digits(text, pos + a + 1);
    if b == 0 {
        return None;
    }
    let end = pos + a + 1 + b;
    match (an2cn(&text[pos..pos + a]), an2cn(&text[pos + a + 1..end])) {
        (Some(numerator), Some(denominator)) => Some((end, format!("{denominator}分之{numerator}"))),
        _ => Some((end, text[pos..end].to_owned())),
    }
}

/// `-?(\d+\.)?\d+` at `pos`: its end.
fn decimal(text: &str, pos: usize) -> Option<usize> {
    let start = if at(text, pos, '-') { pos + 1 } else { pos };
    let a = digits(text, start);
    if a == 0 {
        return None;
    }
    let mut end = start + a;
    if at(text, end, '.') {
        let b = digits(text, end + 1);
        if b > 0 {
            end += 1 + b;
        }
    }
    Some(end)
}

/// `-?(\d+\.)?\d+%` as `百分之…`.
fn percent(text: &str, pos: usize) -> Option<(usize, String)> {
    let start = if at(text, pos, '-') { pos + 1 } else { pos };
    let a = digits(text, start);
    if a == 0 {
        return None;
    }
    let integer = start + a;
    let mut end = None;
    if at(text, integer, '.') {
        let b = digits(text, integer + 1);
        if b > 0 && at(text, integer + 1 + b, '%') {
            end = Some(integer + 1 + b);
        }
    }
    let end = match end {
        Some(end) => end,
        None if at(text, integer, '%') => integer,
        None => return None,
    };
    let read = an2cn(&text[pos..end]).map(|n| format!("百分之{n}"));
    Some((end + 1, read.unwrap_or_else(|| text[pos..end + 1].to_owned())))
}

/// `\d+℃` as `…摄氏度`.
fn celsius(text: &str, pos: usize) -> Option<(usize, String)> {
    let a = digits(text, pos);
    if a == 0 || !at(text, pos + a, '℃') {
        return None;
    }
    let end = pos + a + '℃'.len_utf8();
    let read = an2cn(&text[pos..pos + a]).map(|n| format!("{n}摄氏度"));
    Some((end, read.unwrap_or_else(|| text[pos..end].to_owned())))
}

/// `-?(\d+\.)?\d+` as a number.
fn number(text: &str, pos: usize) -> Option<(usize, String)> {
    let end = decimal(text, pos)?;
    Some((end, an2cn(&text[pos..end]).unwrap_or_else(|| text[pos..end].to_owned())))
}

/// `preprocess` and `__check_inputs_is_valid`: full-width forms made ASCII, then only digits,
/// `.` and `-` allowed.
fn ascii(text: &str) -> Option<String> {
    let s: String = text
        .chars()
        .map(|c| match c as u32 {
            0x3000 => ' ',
            0xFF01..=0xFF5E => char::from_u32(c as u32 - 0xFEE0).expect("ASCII"),
            _ => c,
        })
        .collect();
    s.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-').then_some(s)
}

/// `an2cn(text, "direct")`: each digit read on its own.
fn an2cn_direct(text: &str) -> Option<String> {
    let s = ascii(text)?;
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => ("负", rest),
        None => ("", s.as_str()),
    };
    let mut out = sign.to_owned();
    for c in s.chars() {
        if c == '.' {
            out.push('点');
        } else {
            out.push_str(DIGITS[c.to_digit(10)? as usize]);
        }
    }
    Some(out)
}

/// `an2cn(text, "low")`; `None` where it raises.
fn an2cn(text: &str) -> Option<String> {
    let s = ascii(text)?;
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) => ("负", rest),
        None => ("", s.as_str()),
    };
    let mut parts = s.split('.');
    let integer = parts.next()?;
    let decimal = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let mut out = format!("{sign}{}", integer_convert(integer)?);
    if let Some(decimal) = decimal {
        if !decimal.is_empty() {
            out.push('点');
        }
        for c in decimal.chars().take(16) {
            out.push_str(DIGITS[c.to_digit(10)? as usize]);
        }
    }
    Some(out)
}

/// `__integer_convert(integer, "low")`.
fn integer_convert(integer: &str) -> Option<String> {
    // `str(int(integer))`: int() fails on "" (and on "-", which `ascii` let through), and on more
    // than 4,300 digits (Python's default `sys.get_int_max_str_digits()`), leading zeros included.
    if integer.is_empty() || integer.len() > 4300 || !integer.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = integer.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let len = digits.len();
    if len > UNITS.len() {
        return None;
    }
    let mut out = String::new();
    for (i, d) in digits.bytes().enumerate() {
        let d = usize::from(d - b'0');
        let unit = UNITS[len - i - 1];
        if d != 0 {
            out.push_str(DIGITS[d]);
            out.push_str(unit);
        } else {
            if (len - i - 1) % 4 == 0 {
                out.push_str(DIGITS[0]);
                out.push_str(unit);
            }
            if i > 0 && !out.ends_with('零') {
                out.push_str(DIGITS[0]);
            }
        }
    }
    let out = out.replace("零零", "零").replace("零万", "万").replace("零亿", "亿").replace("亿万", "亿");
    let mut out = out.trim_matches('零').to_owned();
    if out.starts_with("一十") {
        out.remove(0);
    }
    if out.is_empty() {
        out.push('零');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::transform;

    #[test]
    fn numbers_dates_and_the_rest() {
        assert_eq!(transform("1256年"), "一二五六年");
        assert_eq!(transform("12345年"), "一二三四五年");
        assert_eq!(transform("2024年3月5日"), "二零二四年三月五日");
        assert_eq!(transform("3/4"), "四分之三");
        assert_eq!(transform("-12.5%"), "百分之负十二点五");
        assert_eq!(transform("1.2.3%"), "一.百分之二点三");
        assert_eq!(transform("30℃"), "三十摄氏度");
        assert_eq!(transform("10010"), "一万零一十");
        assert_eq!(transform("100000000"), "一亿");
        assert_eq!(transform("12345678901234567"), "12345678901234567");
        assert_eq!(transform("３５"), "三十五");
        assert_eq!(transform("٣٣"), "٣٣");
        assert_eq!(transform("007"), "七");
        assert_eq!(transform("0.50"), "零点五零");
        assert_eq!(transform("123月"), "一二十三月");
        assert_eq!(transform("1-2"), "一负二");
        assert_eq!(transform("5."), "五.");
        assert_eq!(transform("12345678901234567.5"), "12345678901234567.5");
        assert_eq!(transform("0.12345678901234567890"), "零点一二三四五六七八九零一二三四五六");
        assert_eq!(transform("2024年٣月"), "二零二四年٣月");
        assert_eq!(transform("100000001"), "一亿零一");
        assert_eq!(transform("1000010000"), "十亿零一万");
        let zeros = format!("{}5", "0".repeat(4300));
        assert_eq!(transform(&zeros), zeros);
        assert_eq!(transform(&format!("{}5", "0".repeat(4299))), "五");
    }
}
