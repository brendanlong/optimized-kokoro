//! The parts of `inflect` 7.5 the English normalizer calls, with its default arguments:
//! `number_to_words`, and `plural`/`no` for the fixed set of nouns it inflects. inflect is MIT
//! licensed; its notice is in `LICENSE-inflect`.

use crate::py;
use regex::Regex;
use std::sync::LazyLock;

/// `inflect.NumOutOfRangeError` or the `IndexError` its teen table raises: the number has more
/// than 36 digits. The normalizer leaves such numbers as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfRange;

const UNIT: [&str; 10] = ["", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];
const TEEN: [&str; 10] =
    ["ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
const TEN: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
const MILL: [&str; 12] = [
    " ",
    " thousand",
    " million",
    " billion",
    " trillion",
    " quadrillion",
    " quintillion",
    " sextillion",
    " septillion",
    " octillion",
    " nonillion",
    " decillion",
];

fn millfn(index: usize) -> Result<&'static str, OutOfRange> {
    MILL.get(index).copied().ok_or(OutOfRange)
}

fn tenfn(tens: usize, units: usize, mindex: usize) -> Result<String, OutOfRange> {
    if tens != 1 {
        let hyphen = if tens != 0 && units != 0 { "-" } else { "" };
        Ok(format!("{}{hyphen}{}{}", TEN[tens], UNIT[units], millfn(mindex)?))
    } else {
        Ok(format!("{}{}", TEEN[units], millfn(mindex)?))
    }
}

fn hundfn(hundreds: usize, tens: usize, units: usize, mindex: usize) -> Result<String, OutOfRange> {
    if hundreds != 0 {
        let and = if tens != 0 || units != 0 { " and " } else { "" };
        Ok(format!("{} hundred{and}{}{}, ", UNIT[hundreds], tenfn(tens, units, 0)?, millfn(mindex)?))
    } else if tens != 0 || units != 0 {
        Ok(format!("{}{}, ", tenfn(tens, units, 0)?, millfn(mindex)?))
    } else {
        Ok(String::new())
    }
}

/// `enword(num, group=0)` on ASCII digits.
fn enword_whole(num: &[usize]) -> Result<String, OutOfRange> {
    let num = &num[num.iter().position(|&d| d != 0).unwrap_or(num.len())..];
    match num {
        [] => return Ok("zero".into()),
        [1] => return Ok("one".into()),
        _ => {}
    }
    let mut groups = Vec::new();
    let mut rest = num;
    let mut mill_count = 0;
    while rest.len() >= 3 {
        let (head, last) = rest.split_at(rest.len() - 3);
        groups.push(hundfn(last[0], last[1], last[2], mill_count)?);
        mill_count += 1;
        rest = head;
    }
    let lead = match rest {
        [t, u] => format!("{}, ", tenfn(*t, *u, mill_count)?),
        [u] => format!("{}{}, ", UNIT[*u], millfn(mill_count)?),
        _ => String::new(),
    };
    Ok(groups.iter().rev().fold(lead, |acc, g| acc + g))
}

/// `enword(num, group=1)`: digit by digit.
fn enword_digits(num: &[usize]) -> String {
    num.iter()
        .map(|&d| match d {
            0 => " zero, ".to_owned(),
            1 => " one, ".to_owned(),
            d => format!("{}, ", UNIT[d]),
        })
        .collect()
}

static WHITESPACES_COMMA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+,").unwrap());
static COMMA_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r", (\S+)\s+\z").unwrap());
static WHITESPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// `inflect.engine().number_to_words(num)` where `num` is `str()` of the argument: an int, a float
/// or a string of digits.
pub fn number_to_words(num: &str) -> Result<String, OutOfRange> {
    let sign = match num.trim_start_matches(py::is_space).chars().next() {
        Some('+') => "plus",
        Some('-') => "minus",
        _ => "",
    };
    let mut chunks: Vec<&str> = num.splitn(2, '.').collect();
    let finalpoint = chunks.last() == Some(&"");
    if finalpoint {
        chunks.pop();
    }
    let loopstart = chunks[0].is_empty();
    // Python's tri-state `first`: True before the first chunk, None after it, False if there was none.
    let mut first = if loopstart { Some(false) } else { Some(true) };
    let mut words: Vec<String> = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        if index == 0 && loopstart {
            words.push(String::new());
            continue;
        }
        let mut digits: Vec<usize> = chunk.chars().filter_map(py::digit_value).map(|d| d as usize).collect();
        if digits.is_empty() {
            digits.push(0);
        }
        let is_first = first == Some(true);
        let mut chunk = if is_first { enword_whole(&digits)? } else { enword_digits(&digits) };
        if chunk.ends_with(", ") {
            chunk.truncate(chunk.len() - 2);
        }
        let mut chunk = WHITESPACES_COMMA.replace_all(&chunk, ",").into_owned();
        if is_first {
            chunk = COMMA_WORD.replace_all(&chunk, " and $1").into_owned();
        }
        let chunk = WHITESPACES.replace_all(&chunk, " ");
        words.push(chunk.trim().to_owned());
        if is_first {
            first = None;
        }
    }
    let mut numchunks: Vec<&str> = Vec::new();
    if first != Some(false) {
        numchunks.extend(words[0].split(", "));
    }
    for chunk in &words[1..] {
        numchunks.push("point");
        numchunks.extend(chunk.split(", "));
    }
    if finalpoint {
        numchunks.push("point");
    }
    let mut out = if sign.is_empty() { String::new() } else { format!("{sign} ") };
    let mut rest = numchunks.into_iter();
    let first_item = rest.next().ok_or(OutOfRange)?;
    out.push_str(first_item);
    let mut comma = !first_item.ends_with("point");
    for nc in rest {
        if nc == "point" {
            comma = false;
        } else if comma {
            out.push(',');
        }
        out.push(' ');
        out.push_str(nc);
    }
    Ok(out)
}

/// `number_to_words` of an int given as decimal digits (`None` past `u128`, which is out of range).
pub fn int_to_words(digits: Option<&str>) -> Result<String, OutOfRange> {
    number_to_words(digits.ok_or(OutOfRange)?)
}

/// `plural(word, count)` for the nouns the normalizer inflects. A count is singular only when its
/// `str()` is `"1"` (or one of a few words); a float such as `1.0` is plural.
pub fn plural(word: &str, count: &str) -> String {
    if matches!(count, "1" | "a" | "an" | "one" | "each" | "every" | "this" | "that") {
        return word.to_owned();
    }
    match word {
        "foot" => "feet".to_owned(),
        "feet" => "feets".to_owned(),
        "inch" => "inches".to_owned(),
        "pence" => "pence".to_owned(),
        w if w.ends_with("hertz") => w.to_owned(),
        w => {
            assert!(PLAIN_NOUNS.contains(&w), "plural of {w:?} isn't in the table");
            format!("{w}s")
        }
    }
}

/// `no(word, count)`: the count, or "no" for zero, then the noun.
pub fn no(word: &str, count: &str) -> String {
    let count = if matches!(count.to_lowercase().as_str(), "0" | "no" | "zero" | "nil") { "no" } else { count };
    format!("{count} {}", plural(word, count))
}

/// Nouns whose plural adds an "s" (checked against inflect by the parity fixtures).
const PLAIN_NOUNS: &[&str] = &[
    "amp",
    "atmosphere",
    "bit",
    "byte",
    "cent",
    "centiliter",
    "centimeter",
    "deciliter",
    "degree",
    "dollar",
    "euro",
    "gigabit",
    "gigabyte",
    "gram",
    "hour",
    "joule",
    "kelvin",
    "kiloamp",
    "kilobit",
    "kilobyte",
    "kilogram",
    "kilojoule",
    "kilometer",
    "kiloohm",
    "kilopascal",
    "kilovolt",
    "kilowatt",
    "liter",
    "megaamp",
    "megabit",
    "megabyte",
    "megajoule",
    "megaohm",
    "megapascal",
    "megawatt",
    "mergavolt",
    "meter",
    "microfarad",
    "mile",
    "mililiter",
    "milimeter",
    "milligram",
    "millimeter",
    "millisecond",
    "minute",
    "nanofarad",
    "ohm",
    "pascal",
    "petabit",
    "petabyte",
    "picofarad",
    "pixel",
    "pound",
    "second",
    "terabit",
    "terabyte",
    "volt",
    "watt",
    "yard",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_words() {
        for (n, w) in [
            ("0", "zero"),
            ("1", "one"),
            ("05", "five"),
            ("1001", "one thousand and one"),
            ("1100", "one thousand, one hundred"),
            ("2100", "two thousand, one hundred"),
            ("1234.56", "one thousand, two hundred and thirty-four point five six"),
            ("1e-05", "one hundred and five"),
            ("-0.5", "minus zero point five"),
            ("1000000", "one million"),
            ("٣", "three"),
        ] {
            assert_eq!(number_to_words(n).unwrap(), w, "{n}");
        }
        assert_eq!(number_to_words(&"9".repeat(37)), Err(OutOfRange));
        assert!(number_to_words(&"9".repeat(36)).is_ok());
    }
}
