//! The one number reader: every number in English text becomes words here, before spaCy and
//! misaki see it, so misaki's own number code (and its num2words port) is gone
//! (`docs/divergences.md`, "Numbers").
//!
//! It reads numbers the way the reference's normalizer did where that was right (cardinals with
//! "and", four-digit numbers from 1501 as years, decimals digit by digit after "point", ranges
//! with "to", versions with "point", money, `k`/`m`/`b` multipliers) and also reads what the
//! reference left to misaki (ordinals, decades, numbers inside words) and what neither read well
//! (units, degrees, fractions, ISO dates, codes, ratios), without the reference's quirks.

use crate::py::{digit_value, is_alnum};

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const SCALES: [&str; 12] = [
    "",
    "thousand",
    "million",
    "billion",
    "trillion",
    "quadrillion",
    "quintillion",
    "sextillion",
    "septillion",
    "octillion",
    "nonillion",
    "decillion",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn under_100(n: usize) -> String {
    match n {
        0..20 => ONES[n].to_owned(),
        _ if n.is_multiple_of(10) => TENS[n / 10].to_owned(),
        _ => format!("{}-{}", TENS[n / 10], ONES[n % 10]),
    }
}

fn under_1000(n: usize) -> String {
    match (n / 100, n % 100) {
        (0, rest) => under_100(rest),
        (h, 0) => format!("{} hundred", ONES[h]),
        (h, rest) => format!("{} hundred and {}", ONES[h], under_100(rest)),
    }
}

/// Each digit's name: "zero four two". Digits of any script count by their value.
pub fn digits(digits: &str) -> String {
    let names: Vec<&str> = digits.chars().filter_map(digit_value).map(|d| ONES[d as usize]).collect();
    names.join(" ")
}

/// A cardinal from ASCII digits: "one thousand two hundred and thirty-four". "and" comes before
/// tens and units, as both readers in the reference had it, but no commas: a comma is a pause.
/// Past the decillions (36 digits), the digits one by one.
pub fn cardinal(digits_str: &str) -> String {
    let ascii: String = digits_str.chars().filter_map(digit_value).map(|d| char::from(b'0' + d as u8)).collect();
    let trimmed = ascii.trim_start_matches('0');

    if trimmed.is_empty() {
        return "zero".to_owned();
    }
    if trimmed.len() > 3 * SCALES.len() {
        return digits(trimmed);
    }
    let bytes = trimmed.as_bytes();
    let mut groups: Vec<usize> = Vec::new();
    let mut end = bytes.len();
    while end > 0 {
        let start = end.saturating_sub(3);
        groups.push(bytes[start..end].iter().fold(0, |n, &b| n * 10 + usize::from(b - b'0')));
        end = start;
    }
    let mut words: Vec<String> = Vec::new();
    for (scale, &group) in groups.iter().enumerate().rev() {
        if group == 0 {
            continue;
        }
        if scale == 0 && group < 100 && !words.is_empty() {
            words.push("and".to_owned());
        }
        words.push(under_1000(group));
        if scale > 0 {
            words.push(SCALES[scale].to_owned());
        }
    }
    words.join(" ")
}

/// The ordinal of a cardinal reading: its last word made ordinal ("twenty-one" → "twenty-first").
pub fn ordinal_of(reading: &str) -> String {
    let split = reading.rfind([' ', '-']).map_or(0, |i| i + 1);
    let (head, last) = reading.split_at(split);
    let last = match last {
        "one" => "first".to_owned(),
        "two" => "second".to_owned(),
        "three" => "third".to_owned(),
        "five" => "fifth".to_owned(),
        "eight" => "eighth".to_owned(),
        "nine" => "ninth".to_owned(),
        "twelve" => "twelfth".to_owned(),
        w if w.ends_with('y') => format!("{}ieth", &w[..w.len() - 1]),
        w => format!("{w}th"),
    };
    format!("{head}{last}")
}

/// A reading made plural, as for decades: "nineteen nineties", "eighteen hundreds", "hundreds".
fn plural_of(reading: &str) -> String {
    let reading = match reading.strip_prefix("one ") {
        Some(scale) if SCALES.contains(&scale) || scale == "hundred" => scale,
        _ => reading,
    };
    match reading.strip_suffix('y') {
        Some(stem) => format!("{stem}ies"),
        None if reading.ends_with('x') => format!("{reading}es"),
        None => format!("{reading}s"),
    }
}

/// Four digits read in pairs, as years are: "nineteen oh five", "nineteen hundred", "twenty ten".
fn pairs(n: usize) -> String {
    match (n / 100, n % 100) {
        (high, 0) => format!("{} hundred", under_100(high)),
        (high, low @ 1..10) => format!("{} oh {}", under_100(high), ONES[low]),
        (high, low) => format!("{} {}", under_100(high), under_100(low)),
    }
}

/// The reference's year rule: four digits from 1501 to 9999 are a year unless they end in 000 to
/// 009 ("two thousand and five").
fn as_year(digits: &str) -> Option<usize> {
    let n: usize = (digits.len() == 4).then(|| digits.parse().ok()).flatten()?;
    (n > 1500 && n % 1000 > 9).then_some(n)
}

/// How misaki read a number inside a word ("BA123", "A4"): with a leading zero or more than three
/// digits, digit by digit; three digits as "one twenty-three" or "one oh five"; else a cardinal.
fn code(digits_str: &str) -> String {
    let b = digits_str.as_bytes();
    if b[0] == b'0' || b.len() > 3 {
        return digits(digits_str);
    }
    if b.len() == 3 && !digits_str.ends_with("00") {
        let first = ONES[usize::from(b[0] - b'0')];
        return if b[1] == b'0' {
            format!("{first} oh {}", ONES[usize::from(b[2] - b'0')])
        } else {
            format!("{first} {}", cardinal(&digits_str[1..]))
        };
    }
    cardinal(digits_str)
}

/// A fraction `numerator/denominator` in words, for small denominators: "three quarters".
fn fraction(numerator: usize, denominator: usize) -> String {
    let unit = match denominator {
        2 => "half".to_owned(),
        4 => "quarter".to_owned(),
        d => ordinal_of(ONES[d]),
    };
    let unit = match (numerator, denominator) {
        (1, _) => unit,
        (_, 2) => "halves".to_owned(),
        _ => format!("{unit}s"),
    };
    format!("{} {unit}", ONES[numerator])
}

/// After "and": "one and a half", "two and three quarters".
fn and_fraction(numerator: usize, denominator: usize) -> String {
    let words = fraction(numerator, denominator);
    match words.strip_prefix("one ") {
        Some(unit) => format!("and a {unit}"),
        None => format!("and {words}"),
    }
}

/// The vulgar fraction characters.
pub(crate) fn vulgar_fraction(c: char) -> Option<(usize, usize)> {
    Some(match c {
        '½' => (1, 2),
        '⅓' => (1, 3),
        '⅔' => (2, 3),
        '¼' => (1, 4),
        '¾' => (3, 4),
        '⅕' => (1, 5),
        '⅖' => (2, 5),
        '⅗' => (3, 5),
        '⅘' => (4, 5),
        '⅙' => (1, 6),
        '⅚' => (5, 6),
        '⅐' => (1, 7),
        '⅛' => (1, 8),
        '⅜' => (3, 8),
        '⅝' => (5, 8),
        '⅞' => (7, 8),
        '⅑' => (1, 9),
        '⅒' => (1, 10),
        '↉' => (0, 3),
        _ => return None,
    })
}

/// A fraction character on its own: "one half".
pub(crate) fn vulgar_fraction_words(c: char) -> Option<String> {
    vulgar_fraction(c).map(|(n, d)| fraction(n, d))
}

/// Superscript digits.
pub(crate) fn superscript_digit(c: char) -> Option<u8> {
    Some(match c {
        '⁰' => 0,
        '¹' => 1,
        '²' => 2,
        '³' => 3,
        '⁴'..='⁹' => (c as u32 - '⁴' as u32 + 4) as u8,
        _ => return None,
    })
}

/// Subscript digits.
pub(crate) fn subscript_digit(c: char) -> Option<u8> {
    ('₀'..='₉').contains(&c).then(|| (c as u32 - '₀' as u32) as u8)
}

/// " squared", " cubed" or " to the power of N" for a run of superscript digits.
pub(crate) fn power(exponent: &str) -> String {
    match exponent {
        "2" => " squared".to_owned(),
        "3" => " cubed".to_owned(),
        e => format!(" to the power of {}", cardinal(e)),
    }
}

/// Circled and otherwise enclosed numbers (①, ⑴, ⒈, ❶, ➀, ➊).
pub(crate) fn enclosed_number(c: char) -> Option<usize> {
    let c = c as usize;
    Some(match c {
        0x2460..=0x2473 => c - 0x2460 + 1,
        0x2474..=0x2487 => c - 0x2474 + 1,
        0x2488..=0x249B => c - 0x2488 + 1,
        0x24EA => 0,
        0x24EB..=0x24F4 => c - 0x24EB + 11,
        0x24F5..=0x24FE => c - 0x24F5 + 1,
        0x24FF => 0,
        0x2776..=0x277F => c - 0x2776 + 1,
        0x2780..=0x2789 => c - 0x2780 + 1,
        0x278A..=0x2793 => c - 0x278A + 1,
        _ => return None,
    })
}

/// A currency written before its number: names for one and for several, and the minor unit's.
struct Currency {
    one: &'static str,
    many: &'static str,
    minor: Option<(&'static str, &'static str)>,
}

fn currency(c: char) -> Option<Currency> {
    let (one, many, minor) = match c {
        '$' => ("dollar", "dollars", Some(("cent", "cents"))),
        '£' => ("pound", "pounds", Some(("penny", "pence"))),
        '€' => ("euro", "euros", Some(("cent", "cents"))),
        '¥' => ("yen", "yen", None),
        '₹' => ("rupee", "rupees", None),
        '₩' => ("won", "won", None),
        '₽' => ("ruble", "rubles", None),
        '₿' => ("bitcoin", "bitcoins", None),
        '₱' => ("peso", "pesos", None),
        '₪' => ("shekel", "shekels", None),
        '₺' => ("lira", "lira", None),
        _ => return None,
    };
    Some(Currency { one, many, minor })
}

/// Units read after a number, written right after it or after one space. Only unambiguous ones:
/// no single letters, nothing that is also a common word.
const UNITS: &[(&str, &str, &str)] = &[
    ("km/h", "kilometer per hour", "kilometers per hour"),
    ("km²", "square kilometer", "square kilometers"),
    ("km2", "square kilometer", "square kilometers"),
    ("km", "kilometer", "kilometers"),
    ("cm²", "square centimeter", "square centimeters"),
    ("cm³", "cubic centimeter", "cubic centimeters"),
    ("cm", "centimeter", "centimeters"),
    ("mm", "millimeter", "millimeters"),
    ("µm", "micrometer", "micrometers"),
    ("μm", "micrometer", "micrometers"),
    ("nm", "nanometer", "nanometers"),
    ("m²", "square meter", "square meters"),
    ("m2", "square meter", "square meters"),
    ("m³", "cubic meter", "cubic meters"),
    ("m3", "cubic meter", "cubic meters"),
    ("m/s", "meter per second", "meters per second"),
    ("ft²", "square foot", "square feet"),
    ("ft", "foot", "feet"),
    ("yd", "yard", "yards"),
    ("mi", "mile", "miles"),
    ("mph", "mile per hour", "miles per hour"),
    ("kph", "kilometer per hour", "kilometers per hour"),
    ("kg", "kilogram", "kilograms"),
    ("mg", "milligram", "milligrams"),
    ("µg", "microgram", "micrograms"),
    ("μg", "microgram", "micrograms"),
    ("lbs", "pounds", "pounds"),
    ("lb", "pound", "pounds"),
    ("oz", "ounce", "ounces"),
    ("mL", "milliliter", "milliliters"),
    ("ml", "milliliter", "milliliters"),
    ("kWh", "kilowatt hour", "kilowatt hours"),
    ("kW", "kilowatt", "kilowatts"),
    ("MW", "megawatt", "megawatts"),
    ("GW", "gigawatt", "gigawatts"),
    ("mAh", "milliamp hour", "milliamp hours"),
    ("kHz", "kilohertz", "kilohertz"),
    ("MHz", "megahertz", "megahertz"),
    ("GHz", "gigahertz", "gigahertz"),
    ("Hz", "hertz", "hertz"),
    ("kbps", "kilobit per second", "kilobits per second"),
    ("Kbps", "kilobit per second", "kilobits per second"),
    ("Mbps", "megabit per second", "megabits per second"),
    ("Gbps", "gigabit per second", "gigabits per second"),
    ("KB", "kilobyte", "kilobytes"),
    ("kB", "kilobyte", "kilobytes"),
    ("MB", "megabyte", "megabytes"),
    ("GB", "gigabyte", "gigabytes"),
    ("TB", "terabyte", "terabytes"),
    ("PB", "petabyte", "petabytes"),
    ("ms", "millisecond", "milliseconds"),
    ("µs", "microsecond", "microseconds"),
    ("μs", "microsecond", "microseconds"),
    ("ns", "nanosecond", "nanoseconds"),
    ("min", "minute", "minutes"),
    ("hrs", "hours", "hours"),
    ("hr", "hour", "hours"),
    ("secs", "seconds", "seconds"),
    ("sec", "second", "seconds"),
    ("kΩ", "kilohm", "kilohms"),
    ("kΩ", "kilohm", "kilohms"),
    ("MΩ", "megohm", "megohms"),
    ("MΩ", "megohm", "megohms"),
    ("Ω", "ohm", "ohms"),
    ("Ω", "ohm", "ohms"),
];

/// Multipliers written as words after a number (case-insensitive, as the reference matched them).
const MULTIPLIER_WORDS: [&str; 6] = ["hundred", "thousand", "million", "billion", "trillion", "quadrillion"];

/// A number as written: integer digits (ASCII, commas removed), whether it had thousands
/// commas, and its decimals.
#[derive(Debug, Clone)]
struct Body {
    int: String,
    commas: bool,
    frac: Option<String>,
}

impl Body {
    fn is_one(&self) -> bool {
        self.int == "1" && self.frac.is_none()
    }

    fn integer(&self) -> Option<usize> {
        if self.frac.is_some() || self.int.len() > 15 {
            return None;
        }
        self.int.parse().ok()
    }

    /// The plain reading: digit by digit with a leading zero (three digits or more), a year, or
    /// a cardinal; decimals digit by digit after "point".
    fn words(&self) -> String {
        let whole = if self.int.len() >= 3 && self.int.starts_with('0') && !self.commas && self.frac.is_none() {
            digits(&self.int)
        } else if let Some(year) = (!self.commas && self.frac.is_none()).then(|| as_year(&self.int)).flatten() {
            pairs(year)
        } else {
            cardinal(&self.int)
        };
        match &self.frac {
            None => whole,
            Some(frac) => format!("{whole} point {}", digits(frac)),
        }
    }
}

struct Scanner<'a> {
    t: &'a [char],
}

impl Scanner<'_> {
    fn get(&self, i: usize) -> Option<char> {
        self.t.get(i).copied()
    }

    fn is(&self, i: usize, f: impl Fn(char) -> bool) -> bool {
        self.get(i).is_some_and(f)
    }

    fn digit(&self, i: usize) -> bool {
        self.is(i, |c| digit_value(c).is_some())
    }

    /// The end of the run of digits from `i`.
    fn digits_end(&self, i: usize) -> usize {
        let mut j = i;
        while self.digit(j) {
            j += 1;
        }
        j
    }

    fn ascii(&self, range: std::ops::Range<usize>) -> String {
        self.t[range].iter().map(|&c| char::from(b'0' + digit_value(c).expect("a digit") as u8)).collect()
    }

    /// Nothing that makes a word with what's before `i`: not a letter or digit, nor `.` or `,`
    /// right after one (the reference's rule for where a number may start).
    fn free_before(&self, i: usize) -> bool {
        if i == 0 {
            return true;
        }
        let before = self.t[i - 1];
        !glues(before) && !(matches!(before, '.' | ',') && i >= 2 && glues(self.t[i - 2]))
    }

    /// No letter or digit at `i`.
    fn free_at(&self, i: usize) -> bool {
        !self.is(i, is_alnum)
    }

    /// `text` (ASCII, case-sensitive) at `i`.
    fn at(&self, i: usize, text: &str) -> bool {
        text.chars().enumerate().all(|(k, c)| self.get(i + k) == Some(c))
    }

    fn at_ignore_case(&self, i: usize, text: &str) -> bool {
        text.chars().enumerate().all(|(k, c)| self.get(i + k).is_some_and(|x| x.to_ascii_lowercase() == c))
    }

    /// A number at `i` (digits, or `.` and digits): with thousands commas (one to three digits,
    /// then groups of exactly three) and decimals.
    fn body(&self, i: usize) -> Option<(Body, usize)> {
        let mut j = self.digits_end(i);
        let mut int = self.ascii(i..j);
        let mut commas = false;
        if (1..=3).contains(&(j - i)) {
            while self.get(j) == Some(',') && self.digits_end(j + 1) == j + 4 {
                int.push_str(&self.ascii(j + 1..j + 4));
                commas = true;
                j += 4;
            }
        }
        let mut frac = None;
        if self.get(j) == Some('.') && self.digit(j + 1) {
            let end = self.digits_end(j + 1);
            frac = Some(self.ascii(j + 1..end));
            j = end;
        }
        if int.is_empty() && frac.is_none() {
            return None;
        }
        if int.is_empty() {
            int.push('0');
        }
        Some((Body { int, commas, frac }, j))
    }

    /// Dotted parts after the number at `i` with at least two dots ("2.0.1"), the reference's
    /// version reading: each part a cardinal, joined by "point".
    fn version(&self, i: usize) -> Option<(String, usize)> {
        let mut parts = Vec::new();
        let mut j = i;
        loop {
            let end = self.digits_end(j);
            if end == j {
                return None;
            }
            parts.push(cardinal(&self.ascii(j..end)));
            j = end;
            if self.get(j) == Some('.') && self.digit(j + 1) {
                j += 1;
            } else {
                break;
            }
        }
        (parts.len() >= 3).then(|| (parts.join(" point "), j))
    }

    /// A unit after a number ending at `i`: right after it or after one space, and not glued to
    /// a letter or digit after it.
    fn unit(&self, i: usize, one: bool) -> Option<(String, usize)> {
        for start in [i, i + 1] {
            if start == i + 1 && self.get(i) != Some(' ') {
                break;
            }
            for &(key, singular, plural) in UNITS {
                let n = key.chars().count();
                if key.chars().enumerate().all(|(k, c)| self.get(start + k) == Some(c)) && self.free_at(start + n) {
                    return Some((if one { singular } else { plural }.to_owned(), start + n));
                }
            }
        }
        None
    }

    /// Degrees after a number ending at `i`: `°` (after at most one space), then C, F or K.
    fn degrees(&self, i: usize, one: bool) -> Option<(String, usize)> {
        let start =
            if self.get(i) == Some(' ') && matches!(self.get(i + 1), Some('°' | '℃' | '℉')) { i + 1 } else { i };
        let degrees = if one { "degree" } else { "degrees" };
        match self.get(start)? {
            '℃' => Some((format!("{degrees} Celsius"), start + 1)),
            '℉' => Some((format!("{degrees} Fahrenheit"), start + 1)),
            '°' => {
                let scale = match self.get(start + 1) {
                    Some('C') => Some("Celsius"),
                    Some('F') => Some("Fahrenheit"),
                    Some('K') => Some("kelvin"),
                    _ => None,
                }
                .filter(|_| self.free_at(start + 2));
                Some(match scale {
                    Some(scale) => (format!("{degrees} {scale}"), start + 2),
                    None => (degrees.to_owned(), start + 1),
                })
            }
            _ => None,
        }
    }

    /// A fraction `N/D` at `i` that reads as one ("3/4", not "9/11" or a date): one digit over
    /// a larger denominator up to 10, not glued to more numbers or slashes.
    fn slash_fraction(&self, i: usize) -> Option<(usize, usize, usize)> {
        let n_end = self.digits_end(i);
        if n_end == i || self.get(n_end) != Some('/') && self.get(n_end) != Some('⁄') {
            return None;
        }
        let d_end = self.digits_end(n_end + 1);
        let n: usize = self.ascii(i..n_end).parse().ok()?;
        let d: usize = self.ascii(n_end + 1..d_end).parse().ok()?;
        let after_ok = self.free_at(d_end)
            && !matches!(self.get(d_end), Some('/' | '⁄'))
            && !(matches!(self.get(d_end), Some('.' | ',')) && self.digit(d_end + 1));
        let before_ok = i == 0 || !matches!(self.t[i - 1], '/' | '⁄');
        (d_end > n_end + 1 && n >= 1 && n < d && (2..=10).contains(&d) && after_ok && before_ok)
            .then_some((n, d, d_end))
    }

    /// A chain of numbers joined by `-` from `i` (each part an integer): the parts' ranges.
    fn dash_chain(&self, i: usize) -> Vec<(usize, usize)> {
        let mut parts = vec![(i, self.digits_end(i))];
        loop {
            let end = parts[parts.len() - 1].1;
            if self.get(end) == Some('-') && self.digit(end + 1) {
                let next = self.digits_end(end + 1);
                parts.push((end + 1, next));
            } else {
                return parts;
            }
        }
    }

    /// An ISO date, `YYYY-MM-DD`: "May fifth, twenty twenty".
    fn iso_date(&self, parts: &[(usize, usize)]) -> Option<String> {
        let [y, m, d] = parts else { return None };
        if y.1 - y.0 != 4 || m.1 - m.0 != 2 || d.1 - d.0 != 2 {
            return None;
        }
        let year: usize = self.ascii(y.0..y.1).parse().ok()?;
        let month: usize = self.ascii(m.0..m.1).parse().ok()?;
        let day: usize = self.ascii(d.0..d.1).parse().ok()?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let year = if year % 1000 > 9 && year >= 1000 { pairs(year) } else { cardinal(&year.to_string()) };
        Some(format!("{} {}, {year}", MONTHS[month - 1], ordinal_of(&under_100(day))))
    }

    /// The number expression starting at `i`, if one does: its end and its words.
    /// `plain` says a number here isn't glued to what's before (an operator was read there).
    fn expression(&self, i: usize, plain: bool) -> Option<(usize, String, bool)> {
        let c = self.t[i];
        let free = plain || self.free_before(i);
        match c {
            '~' if free && self.number_at(i + 1) => {
                let (end, words, after) = self.expression(i + 1, true)?;
                Some((end, format!("about {words}"), after))
            }
            '#' if self.digit(i + 1) => {
                let (end, words, after) = self.expression(i + 1, true)?;
                Some((end, format!("number {words}"), after))
            }
            '±' if self.number_at(i + 1) || self.get(i + 1) == Some(' ') && self.number_at(i + 2) => {
                let start = if self.get(i + 1) == Some(' ') { i + 2 } else { i + 1 };
                let (end, words, after) = self.expression(start, true)?;
                Some((end, format!("plus or minus {words}"), after))
            }
            '-' | '−' if free => {
                let next = self.get(i + 1)?;
                if currency(next).is_some() && self.number_at(i + 2) {
                    let (end, words, after) = self.money(i + 1)?;
                    Some((end, format!("minus {words}"), after))
                } else if self.number_at(i + 1) {
                    let (end, words, after) = self.number(i + 1, true)?;
                    Some((end, format!("minus {words}"), after))
                } else {
                    None
                }
            }
            _ if currency(c).is_some() && self.number_at(i + 1) => self.money(i),
            _ if self.digit(i) || c == '.' && self.digit(i + 1) && free => self.number(i, free),
            _ => None,
        }
    }

    /// Digits, or `.` and a digit, at `i`.
    fn number_at(&self, i: usize) -> bool {
        self.digit(i) || self.get(i) == Some('.') && self.digit(i + 1)
    }

    /// A word multiplier after a number ending at `i` (" million", " hundred thousand"), or a
    /// glued `k`, `m` or `b` (and `t` for money): its words and end.
    fn multiplier(&self, i: usize, money: bool) -> Option<(String, usize)> {
        let mut words = Vec::new();
        let mut j = i;
        while self.get(j) == Some(' ') {
            let Some(word) = MULTIPLIER_WORDS.iter().find(|w| self.at_ignore_case(j + 1, w)) else { break };
            if !self.free_at(j + 1 + word.len()) {
                break;
            }
            words.push(self.t[j + 1..j + 1 + word.len()].iter().collect::<String>());
            j += 1 + word.len();
        }
        if !words.is_empty() {
            return Some((words.join(" "), j));
        }
        let word = match self.get(i)?.to_ascii_lowercase() {
            'k' => "thousand",
            'm' => "million",
            'b' => "billion",
            't' if money => "trillion",
            _ => return None,
        };
        self.free_at(i + 1).then(|| (word.to_owned(), i + 1))
    }

    /// Money: a currency symbol and a number, and whether a range's "to" ends it ("$5-10").
    fn money(&self, i: usize) -> Option<(usize, String, bool)> {
        let currency = currency(self.t[i])?;
        let (body, end) = self.body(i + 1)?;
        if self.get(end) == Some('.') && self.digit(end + 1) {
            // "$1.2.3" isn't money: the symbol and the version are read apart.
            return None;
        }
        if let Some((multiplier, m_end)) = self.multiplier(end, true) {
            return Some((m_end, format!("{} {multiplier} {}", body.words_plain(), currency.many), false));
        }
        let whole = cardinal(&body.int);
        let name = if body.int.trim_start_matches('0') == "1" { currency.one } else { currency.many };
        let cents = body.frac.as_deref().filter(|f| f.bytes().any(|b| b != b'0'));
        let mut words = match (cents, currency.minor) {
            (None, _) => format!("{whole} {name}"),
            (Some(frac), Some((minor_one, minor_many))) if frac.len() <= 2 => {
                let cents = format!("{frac:0<2}");
                let minor = if cents == "01" { minor_one } else { minor_many };
                let cents = cardinal(&cents);
                if body.int.bytes().all(|b| b == b'0') {
                    format!("{cents} {minor}")
                } else {
                    format!("{whole} {name} and {cents} {minor}")
                }
            }
            (Some(_), _) => format!("{} {}", body.words_plain(), currency.many),
        };
        if self.get(end) == Some('-') && self.number_at(end + 1) {
            words.push_str(" to");
            return Some((end + 1, words, true));
        }
        Some((end, words, false))
    }

    /// A number at `i`: plain when `free` (nothing glued before it), else as part of a word.
    /// Returns its end, its words, and whether it ends in an operator ("to", "by", "times"),
    /// after which the next number stands on its own.
    fn number(&self, i: usize, free: bool) -> Option<(usize, String, bool)> {
        if !free {
            return self.in_word(i);
        }
        if self.digit(i)
            && let Some((words, end)) = self.version(i)
        {
            return Some((end, words, false));
        }
        let (body, end) = self.body(i)?;
        let one = body.is_one();
        let integer = body.frac.is_none();
        if let Some((multiplier, end)) = self.multiplier(end, false) {
            return Some((end, format!("{} {multiplier}", body.words_plain()), false));
        }
        if integer {
            for suffix in ["st", "nd", "rd", "th"] {
                if self.at_ignore_case(end, suffix) && self.free_at(end + 2) {
                    return Some((end + 2, ordinal_of(&cardinal(&body.int)), false));
                }
            }
            // Decades and plurals: "1990s", "1990's", "80s", "100s".
            let plural = if self.is(end, |c| c == 's' || c == 'S') && self.free_at(end + 1) {
                Some(end + 1)
            } else if self.at(end, "'s") && self.free_at(end + 2) {
                Some(end + 2)
            } else {
                None
            };
            if let Some(after) = plural {
                let reading = match body.integer() {
                    Some(n) if body.int.len() == 4 && !body.commas && n % 1000 > 9 => pairs(n),
                    _ => cardinal(&body.int),
                };
                return Some((after, plural_of(&reading), false));
            }
            if self.get(end) == Some('¢') {
                return Some((end + 1, format!("{} {}", body.words(), if one { "cent" } else { "cents" }), false));
            }
        }
        if let Some((words, end)) = self.degrees(end, one) {
            return Some((end, format!("{} {words}", body.words()), false));
        }
        if let Some((words, end)) = self.unit(end, one) {
            return Some((end, format!("{} {words}", body.words()), false));
        }
        if integer {
            // "1½", "1 ½", "1 3/4"
            let gap = usize::from(self.get(end) == Some(' '));
            if let Some((n, d)) = self.get(end + gap).and_then(vulgar_fraction) {
                return Some((end + gap + 1, format!("{} {}", body.words(), and_fraction(n, d)), false));
            }
            if gap == 1
                && let Some((n, d, f_end)) = self.slash_fraction(end + 1)
            {
                return Some((f_end, format!("{} {}", body.words(), and_fraction(n, d)), false));
            }
            if !body.commas
                && let Some((n, d, f_end)) = self.slash_fraction(i)
            {
                return Some((f_end, fraction(n, d), false));
            }
            // Feet and inches: 6'2" or 6′2″.
            if matches!(self.get(end), Some('\'' | '′')) && self.digit(end + 1) {
                let in_end = self.digits_end(end + 1);
                if matches!(self.get(in_end), Some('"' | '″')) {
                    let inches = self.ascii(end + 1..in_end);
                    let feet = if one { "foot" } else { "feet" };
                    let inch = if inches.trim_start_matches('0') == "1" { "inch" } else { "inches" };
                    return Some((in_end + 1, format!("{} {feet} {} {inch}", body.words(), cardinal(&inches)), false));
                }
            }
            // Numbers joined by dashes: an ISO date, a code ("978-3-16-148410-0"), or a phone
            // number ("555-1234"); a range is below.
            if !body.commas && self.get(end) == Some('-') && self.digit(end + 1) {
                let chain = self.dash_chain(i);
                if chain.len() >= 3 {
                    let last = chain[chain.len() - 1].1;
                    let words = self.iso_date(&chain).unwrap_or_else(|| {
                        chain.iter().map(|&(a, b)| digits(&self.ascii(a..b))).collect::<Vec<_>>().join(", ")
                    });
                    return Some((last, words, false));
                }
                let (second, second_end) = chain[1];
                if second_end - second == 4 && end - i == 3 && !self.is(second_end, |c| c == '.') {
                    let words = format!("{}, {}", digits(&body.int), digits(&self.ascii(second..second_end)));
                    return Some((second_end, words, false));
                }
            }
            // Ratios ("3:1", "16:9"); times were read before this pass.
            if self.get(end) == Some(':') && self.digit(end + 1) {
                return Some((end + 1, format!("{} to", body.words()), true));
            }
        }
        // A range: "10-20", "1.5-2".
        if self.get(end) == Some('-') && self.number_at(end + 1) {
            return Some((end + 1, format!("{} to", body.words()), true));
        }
        // "3x faster", "1920x1080"
        if self.get(end) == Some('x') {
            if self.free_at(end + 1) {
                return Some((end + 1, format!("{} times", body.words()), false));
            }
            if self.digit(end + 1) {
                return Some((end + 1, format!("{} by", body.words()), true));
            }
        }
        // "2 * 3"
        let mut j = end;
        while self.get(j) == Some(' ') {
            j += 1;
        }
        if self.get(j) == Some('*') {
            let mut k = j + 1;
            while self.get(k) == Some(' ') {
                k += 1;
            }
            if self.digit(k) {
                return Some((k, format!("{} times", body.words()), true));
            }
        }
        // Superscript powers: "10²", "10⁶".
        let mut sup_end = end;
        let mut exponent = String::new();
        while let Some(d) = self.get(sup_end).and_then(superscript_digit) {
            exponent.push(char::from(b'0' + d));
            sup_end += 1;
        }
        if !exponent.is_empty() {
            return Some((sup_end, format!("{}{}", body.words(), power(&exponent)), false));
        }
        // Letters right after: "1080p" reads its four digits in pairs, as misaki did.
        if self.is(end, char::is_alphabetic)
            && !body.commas
            && let Some(n) = body.integer().filter(|n| body.int.len() == 4 && n % 1000 > 9)
        {
            return Some((end, pairs(n), false));
        }
        Some((end, body.words(), false))
    }

    /// Digits inside a word ("BA123", "v1.2", "B2B").
    fn in_word(&self, i: usize) -> Option<(usize, String, bool)> {
        let end = self.digits_end(i);
        let int = self.ascii(i..end);
        // "B2B", "P2P": a 2 between single capitals is "to", as misaki read it.
        if int == "2"
            && i >= 1
            && self.t[i - 1].is_ascii_uppercase()
            && (i < 2 || !self.t[i - 2].is_alphabetic())
            && self.is(end, |c| c.is_ascii_uppercase())
            && !self.is(end + 1, char::is_alphabetic)
            && !(self.t[i - 1] == 'H' && self.get(end) == Some('O'))
        {
            return Some((end, "to".to_owned(), false));
        }
        if self.get(i - 1) == Some('.') && self.number_word_before(i - 1) {
            // Decimals after a number already read ("7:09.30", whose minutes the time pass read).
            return Some((end, format!("point {}", digits(&int)), false));
        }
        let (body, body_end) = self.body(i)?;
        if body.commas || body.frac.is_some() {
            return Some((body_end, body.words_plain(), false));
        }
        Some((end, code(&int), false))
    }

    /// Whether the word ending at `i` (exclusive) is a number word.
    fn number_word_before(&self, i: usize) -> bool {
        let mut start = i;
        while start > 0 && self.t[start - 1].is_ascii_alphabetic() {
            start -= 1;
        }
        let word: String = self.t[start..i].iter().collect();
        !word.is_empty()
            && (ONES.contains(&word.as_str())
                || TENS.contains(&word.as_str())
                || SCALES.contains(&word.as_str())
                || word == "oh"
                || word == "hundred")
    }
}

impl Body {
    /// [`Body::words`] without the year rule or leading-zero digits, for numbers with a
    /// multiplier or money's decimals.
    fn words_plain(&self) -> String {
        let whole = cardinal(&self.int);
        match &self.frac {
            None => whole,
            Some(frac) => format!("{whole} point {}", digits(frac)),
        }
    }
}

/// Whether a number right next to `c` is part of a word with it: Latin letters and digits. Next
/// to letters of other scripts, which go to their own G2P, the number stands alone, as misaki
/// read it ("1999年", "Б6").
fn glues(c: char) -> bool {
    is_alnum(c) && (c.is_ascii() || matches!(c as u32, 0xC0..=0x24F | 0x1E00..=0x1EFF))
}

/// Pushes `words` onto `out`, kept apart by a space from a letter or digit on either side. A
/// number inside a word ("A4", "BA123") is joined to its letters by hyphens ("A-four"), which
/// misaki reads as one word, so a lone letter stays a letter name rather than the article "a".
fn push_words(out: &mut String, words: &str, next: Option<char>, in_word: bool) {
    let join = |c: char| if in_word && glues(c) { '-' } else { ' ' };
    if let Some(c) = out.chars().next_back().filter(|&c| is_alnum(c)) {
        out.push(join(c));
    }
    out.push_str(words);
    if let Some(c) = next.filter(|&c| is_alnum(c)) {
        out.push(if c.is_alphabetic() { join(c) } else { ' ' });
    }
}

/// The number pass: every number in the text as words.
pub fn numbers(text: &str) -> String {
    if !text.chars().any(|c| digit_value(c).is_some()) {
        return text.to_owned();
    }
    let t: Vec<char> = text.chars().collect();
    let scanner = Scanner { t: &t };
    let mut out = String::with_capacity(text.len() * 2);
    let mut i = 0;
    let mut plain_at = usize::MAX;
    while i < t.len() {
        let c = t[i];
        let candidate = scanner.digit(i) || matches!(c, '~' | '±' | '-' | '−' | '.' | '#') || currency(c).is_some();
        if candidate && let Some((end, words, operator)) = scanner.expression(i, plain_at == i) {
            let in_word = scanner.digit(i) && plain_at != i && !scanner.free_before(i);
            if in_word && words.starts_with("point ") && out.ends_with('.') {
                // The decimal point of a number read before this pass ("seven oh nine.30").
                out.pop();
                out.push(' ');
            }
            push_words(&mut out, &words, t.get(end).copied(), in_word);
            if operator {
                // An operator was read ("to", "by", "times"): the next number stands on its own.
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                plain_at = end;
            }
            i = end;
            continue;
        }
        // "'80s": the apostrophe of a decade goes.
        if matches!(c, '\'' | '’')
            && scanner.free_before(i)
            && scanner.digits_end(i + 1) == i + 3
            && t.get(i + 3) == Some(&'s')
            && scanner.free_at(i + 4)
        {
            i += 1;
            continue;
        }
        // A list written with commas ("1,2,3") gets a space after each comma.
        if c == ',' && i > 0 && scanner.digit(i - 1) && scanner.digit(i + 1) {
            out.push_str(", ");
            plain_at = i + 1;
            i += 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Whether a character is one the number pass reads: a digit, or a number written another way.
pub fn is_number_char(c: char) -> bool {
    digit_value(c).is_some()
        || vulgar_fraction(c).is_some()
        || superscript_digit(c).is_some()
        || subscript_digit(c).is_some()
        || enclosed_number(c).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cardinals() {
        for (n, w) in [
            ("0", "zero"),
            ("7", "seven"),
            ("15", "fifteen"),
            ("40", "forty"),
            ("99", "ninety-nine"),
            ("100", "one hundred"),
            ("105", "one hundred and five"),
            ("1001", "one thousand and one"),
            ("1100", "one thousand one hundred"),
            ("1234", "one thousand two hundred and thirty-four"),
            ("101000", "one hundred and one thousand"),
            ("1000050", "one million and fifty"),
            ("2000100", "two million one hundred"),
        ] {
            assert_eq!(cardinal(n), w, "{n}");
        }
        assert_eq!(cardinal(&"9".repeat(37)), digits(&"9".repeat(37)));
    }

    #[test]
    fn ordinals_and_plurals() {
        assert_eq!(ordinal_of("twenty-one"), "twenty-first");
        assert_eq!(ordinal_of("one hundred"), "one hundredth");
        assert_eq!(ordinal_of("twelve"), "twelfth");
        assert_eq!(ordinal_of("forty"), "fortieth");
        assert_eq!(plural_of("nineteen ninety"), "nineteen nineties");
        assert_eq!(plural_of("one hundred"), "hundreds");
        assert_eq!(plural_of("six"), "sixes");
    }

    fn n(text: &str) -> String {
        numbers(text)
    }

    #[test]
    fn reads_numbers_in_text() {
        for (text, expected) in [
            ("It costs $1.", "It costs one dollar."),
            ("It costs $1.50 today.", "It costs one dollar and fifty cents today."),
            ("It costs $0.99 now.", "It costs ninety-nine cents now."),
            ("It costs $0.01 now.", "It costs one cent now."),
            ("It costs $5 million.", "It costs five million dollars."),
            ("It costs $2.5B.", "It costs two point five billion dollars."),
            ("£20 and €30 and ¥500", "twenty pounds and thirty euros and five hundred yen"),
            ("$0.00001 each", "zero point zero zero zero zero one dollars each"),
            ("fell -$5 today", "fell minus five dollars today"),
            ("in 1900.", "in nineteen hundred."),
            ("in 1905.", "in nineteen oh five."),
            ("in 2005.", "in two thousand and five."),
            ("in 2100.", "in twenty-one hundred."),
            ("the 1990s and the 1800s", "the nineteen nineties and the eighteen hundreds"),
            ("the 2000s", "the two thousands"),
            ("the 80s", "the eighties"),
            ("100s of people", "hundreds of people"),
            ("1st, 2nd, 3rd, 21st and 100th", "first, second, third, twenty-first and one hundredth"),
            ("pages 10-20", "pages ten to twenty"),
            ("I have 1,234 apples", "I have one thousand two hundred and thirty-four apples"),
            ("1,2,3", "one, two, three"),
            ("3.14159", "three point one four one five nine"),
            ("2.50", "two point five zero"),
            ("-5 degrees", "minus five degrees"),
            (
                "25°C and 77 °F and 90°",
                "twenty-five degrees Celsius and seventy-seven degrees Fahrenheit and ninety degrees",
            ),
            ("1°", "one degree"),
            ("10 km and 10km, 1 km", "ten kilometers and ten kilometers, one kilometer"),
            ("3.5GHz", "three point five gigahertz"),
            ("5 Ω", "five ohms"),
            ("version 2.0.1", "version two point zero point one"),
            ("v1.2", "v-one point two"),
            ("BA123 and A4", "BA-one twenty-three and A-four"),
            ("item #1, MP3s and H2O", "item number one, MP-three-s and H-two-O"),
            ("B2B", "B-to-B"),
            ("1080p", "ten eighty p"),
            ("3:1", "three to one"),
            ("1/2 cup", "one half cup"),
            ("3/4 and 9/11", "three quarters and nine/eleven"),
            ("1.5-2.5 kg", "one point five to two point five kilograms"),
            ("$5-10", "five dollars to ten"),
            ("the '80s", "the eighties"),
            ("50¢ and 1¢", "fifty cents and one cent"),
            ("the 1500s", "the fifteen hundreds"),
            ("他于1934年去世 and Б6", "他于 nineteen thirty-four 年去世 and Б six"),
            ("in seven oh nine.30", "in seven oh nine point three zero"),
            ("£A10,000", "£A-ten thousand"),
            ("5,000 km2", "five thousand square kilometers"),
            ("1½ cups and 2 3/4 miles", "one and a half cups and two and three quarters miles"),
            ("978-3-16-148410-0", "nine seven eight, three, one six, one four eight four one zero, zero"),
            ("2020-05-05", "May fifth, twenty twenty"),
            ("555-1234", "five five five, one two three four"),
            ("PIN 0042", "PIN zero zero four two"),
            ("3x faster", "three times faster"),
            ("1920x1080", "nineteen twenty by one thousand and eighty"),
            ("2 * 3", "two times three"),
            ("~5 minutes", "about five minutes"),
            ("±0.3 mm", "plus or minus zero point three millimeters"),
            ("6'2\" tall", "six feet two inches tall"),
            ("4K TV", "four thousand TV"),
            ("10²", "ten squared"),
            ("COVID-19", "COVID-nineteen"),
            ("1939-1945", "nineteen thirty-nine to nineteen forty-five"),
        ] {
            assert_eq!(n(text), expected, "{text}");
        }
    }

    #[test]
    fn huge_numbers_are_read_digit_by_digit() {
        let text = "1".repeat(50);
        assert_eq!(n(&text), digits(&text));
    }
}
