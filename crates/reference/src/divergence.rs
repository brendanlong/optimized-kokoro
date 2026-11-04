//! Checks that a difference from the reference's normalized text is one `docs/divergences.md`
//! lists: the number reader ("Numbers") or the symbol pass ("Symbols").
//!
//! Both texts are split into words and punctuation and compared token by token. Every run of
//! tokens that differs must hold, on the reference's side, a digit (a number the reference left
//! to misaki) or a symbol (a character other than letters and the punctuation Kokoro reads), or
//! consist only of number words ("one thousand, two hundred" against "one thousand two hundred",
//! "nineteen zero" against "nineteen hundred", "dollars" against "dollar"), or be the same
//! characters split differently ("USten" against "US ten"). Where the diff pairs the wrong words,
//! each run of number words (with "and", "point", commas and the like inside it) is masked as
//! one `<n>` and the masked runs must be equal. Anything else, such as a changed word elsewhere
//! in the sentence, is unexplained.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Numbers,
    Symbols,
}

/// Punctuation both normalizers leave as it is.
fn is_plain_punctuation(c: char) -> bool {
    ".,;:!?'\"()[]{}-—–…“”‘’«»¡¿`".contains(c)
}

fn is_symbol(c: char) -> bool {
    !c.is_alphanumeric() && !c.is_whitespace() && !is_plain_punctuation(c)
}

/// Words that only ever come from reading a number.
const NUMERALS: &[&str] = &[
    "zero",
    "oh",
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
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
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
    "first",
    "second",
    "third",
    "fif",
    "eigh",
    "nin",
    "twelf",
    "half",
    "halve",
    "quarter",
    "dollar",
    "cent",
    "pound",
    "penny",
    "pence",
    "euro",
    "yen",
    "rupee",
    "degree",
    "celsius",
    "fahrenheit",
    "percent",
    "squared",
    "cubed",
    "feet",
    "foot",
    "inch",
    "inche",
    "kilometer",
    "centimeter",
    "millimeter",
    "micrometer",
    "nanometer",
    "meter",
    "mile",
    "yard",
    "kilogram",
    "milligram",
    "microgram",
    "ounce",
    "milliliter",
    "kilowatt",
    "megawatt",
    "gigawatt",
    "milliamp",
    "hour",
    "hertz",
    "kilohertz",
    "megahertz",
    "gigahertz",
    "kilobit",
    "megabit",
    "gigabit",
    "kilobyte",
    "megabyte",
    "gigabyte",
    "terabyte",
    "petabyte",
    "millisecond",
    "microsecond",
    "nanosecond",
    "minute",
    "ohm",
    "kilohm",
    "megohm",
    "o'clock",
    // The units of the reference's unit option, which is gone (the number reader reads units).
    "volt",
    "kilovolt",
    "mergavolt",
    "amp",
    "kiloamp",
    "megaamp",
    "watt",
    "joule",
    "kilojoule",
    "megajoule",
    "pascal",
    "kilopascal",
    "megapascal",
    "atmosphere",
    "liter",
    "mililiter",
    "centiliter",
    "deciliter",
    "gram",
    "milimeter",
    "bit",
    "byte",
    "terabit",
    "petabit",
    "kiloohm",
    "megaohm",
    "microfarad",
    "nanofarad",
    "picofarad",
    "pixel",
    "kelvin",
    "feets",
];

/// Words that also come from reading a number, but are common elsewhere: masked only inside a
/// run of numerals.
const CONNECTIVES: &[&str] = &[
    "and",
    "point",
    "minus",
    "plus",
    "to",
    "by",
    "times",
    "about",
    "a",
    "the",
    "of",
    "per",
    "square",
    "cubic",
    "number",
    "hashtag",
    "sharp",
    "slash",
    "equals",
    "at",
    "power",
    "x",
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// The unit abbreviations the number reader reads, which the reference left as letters, and
/// the ordinal suffixes it split off ("1ST" as "one ST").
const ABBREVIATIONS: &[&str] = &[
    "km/h",
    "km",
    "cm",
    "mm",
    "nm",
    "ft",
    "yd",
    "mi",
    "mph",
    "kph",
    "kg",
    "mg",
    "lbs",
    "lb",
    "oz",
    "mL",
    "ml",
    "kWh",
    "kW",
    "MW",
    "GW",
    "mAh",
    "kHz",
    "MHz",
    "GHz",
    "Hz",
    "kbps",
    "Kbps",
    "Mbps",
    "Gbps",
    "KB",
    "kB",
    "MB",
    "GB",
    "TB",
    "PB",
    "ms",
    "ns",
    "min",
    "hrs",
    "hr",
    "secs",
    "sec",
    "h",
    "m",
    "s",
    "S",
    "st",
    "nd",
    "rd",
    "th",
    "ST",
    "ND",
    "RD",
    "TH",
    "Ω",
    "kΩ",
    "MΩ",
    "\u{2126}",
    "k\u{2126}",
    "M\u{2126}",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Numeral,
    Connective,
    Other,
}

/// Whether every part of a word (split at hyphens and colons) is one of `words`, in any number
/// or as an ordinal: "twenty-first", "nineties", "eighty's", "kilometers".
fn made_of(word: &str, words: &[&str]) -> bool {
    let word = word.strip_suffix("'s").unwrap_or(word).to_lowercase();
    !word.is_empty()
        && word.split(['-', ':']).all(|part| {
            let forms = [
                Some(part.to_owned()),
                part.strip_suffix("ies").map(|s| format!("{s}y")),
                part.strip_suffix('s').map(str::to_owned),
                part.strip_suffix("ieth").map(|s| format!("{s}y")),
                part.strip_suffix("th").map(str::to_owned),
                part.strip_suffix("ths").map(str::to_owned),
            ];
            forms.into_iter().flatten().any(|w| words.contains(&w.as_str()))
        })
}

fn class(word: &str) -> Class {
    if word.chars().any(char::is_numeric) || ABBREVIATIONS.contains(&word) || made_of(word, NUMERALS) {
        Class::Numeral
    } else if made_of(word, CONNECTIVES) || word.chars().all(is_plain_punctuation) {
        Class::Connective
    } else {
        Class::Other
    }
}

/// Han, kana and Hangul, which are written without spaces.
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xAC00..=0xD7AF
        | 0xFF66..=0xFF9F | 0x20000..=0x3FFFF)
}

/// Words and punctuation: punctuation at a word's edges, hyphens and commas are tokens of their
/// own, and a word splits where digits meet other characters ("1999年", "S1D1", "late-13th") and
/// where Han, kana or Hangul meet other characters.
fn tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for word in text.split_whitespace() {
        let start = word.len() - word.trim_start_matches(is_plain_punctuation).len();
        let core = word[start..].trim_end_matches(is_plain_punctuation);
        let end = start + core.len();
        out.extend(word[..start].char_indices().map(|(i, c)| &word[i..i + c.len_utf8()]));
        let mut piece = start;
        let mut previous: Option<char> = None;
        for (i, c) in core.char_indices() {
            let at = start + i;
            if matches!(c, '-' | ',' | '.' | ':' | '/') {
                if piece < at {
                    out.push(&word[piece..at]);
                }
                out.push(&word[at..at + c.len_utf8()]);
                piece = at + c.len_utf8();
                previous = None;
                continue;
            }
            if previous.is_some_and(|p| p.is_numeric() != c.is_numeric() || is_cjk(p) != is_cjk(c)) && piece < at {
                out.push(&word[piece..at]);
                piece = at;
            }
            previous = Some(c);
        }
        if piece < end {
            out.push(&word[piece..end]);
        }
        out.extend(word[end..].char_indices().map(|(i, c)| &word[end + i..end + i + c.len_utf8()]));
    }
    out
}

/// A token after masking, with the text it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Masked<'a> {
    token: &'a str,
    text: String,
}

impl Masked<'_> {
    fn class(&self) -> Class {
        if self.token == "<n>" { Class::Numeral } else { class(self.token) }
    }
}

/// Each run of numerals and connectives that starts and ends with a numeral, as one `<n>`.
fn mask<'a>(tokens: &[&'a str]) -> Vec<Masked<'a>> {
    let classes: Vec<Class> = tokens.iter().map(|t| class(t)).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if classes[i] != Class::Numeral {
            out.push(Masked { token: tokens[i], text: tokens[i].to_owned() });
            i += 1;
            continue;
        }
        let mut end = i + 1;
        let mut j = i + 1;
        while j < tokens.len() && classes[j] != Class::Other {
            j += 1;
            if classes[j - 1] == Class::Numeral {
                end = j;
            }
        }
        out.push(Masked { token: "<n>", text: tokens[i..end].concat() });
        i = end;
    }
    out
}

/// The ranges of `a` and `b` that differ, compared by token: a longest-common-subsequence diff.
fn hunks(a: &[Masked], b: &[Masked]) -> Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let same = |x: &Masked, y: &Masked| x.token == y.token;
    let prefix = a.iter().zip(b).take_while(|(x, y)| same(x, y)).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| same(x, y)).count();
    let (n, m) = (a.len() - suffix - prefix, b.len() - suffix - prefix);
    let (a, b) = (&a[prefix..prefix + n], &b[prefix..prefix + m]);
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] =
                if same(&a[i], &b[j]) { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    let mut start: Option<(usize, usize)> = None;
    while i < n || j < m {
        if i < n && j < m && same(&a[i], &b[j]) {
            if let Some((si, sj)) = start.take() {
                out.push((prefix + si..prefix + i, prefix + sj..prefix + j));
            }
            i += 1;
            j += 1;
            continue;
        }
        start.get_or_insert((i, j));
        if j < m && (i == n || lcs[at(i, j + 1)] >= lcs[at(i + 1, j)]) {
            j += 1;
        } else {
            i += 1;
        }
    }
    if let Some((si, sj)) = start {
        out.push((prefix + si..prefix + n, prefix + sj..prefix + m));
    }
    out
}

/// The text without spaces, hyphens, commas or periods.
fn squeezed(tokens: &[Masked]) -> String {
    tokens.iter().flat_map(|w| w.text.chars()).filter(|&c| !"-,.".contains(c)).collect()
}

/// `word` is `other` with number words glued to it: "USten" and "US", "vtwo" and "v".
fn glued(word: &str, other: &str) -> bool {
    !other.is_empty()
        && (word.strip_prefix(other).is_some_and(|rest| made_of(rest, NUMERALS))
            || word.strip_suffix(other).is_some_and(|rest| made_of(rest, NUMERALS)))
}

/// Why `ours` differs from the reference's normalized text: the divergences that explain every
/// differing run of tokens, or the first run nothing explains.
pub fn explain(reference: &str, ours: &str) -> Result<BTreeSet<Kind>, String> {
    let (a, b) = (tokens(reference), tokens(ours));
    let (ma, mb) = (mask(&a), mask(&b));
    let mut kinds = BTreeSet::new();
    let numbers = |m: &[Masked]| m.iter().filter(|m| m.token == "<n>").map(|m| m.text.clone()).collect::<Vec<_>>();
    if numbers(&ma) != numbers(&mb) {
        kinds.insert(Kind::Numbers);
    }
    for (ra, rb) in hunks(&ma, &mb) {
        let (theirs, mine) = (&ma[ra.clone()], &mb[rb.clone()]);
        let show = |side: &[Masked]| side.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join(" ");
        // The words the two symbol passes write for `#`: "number" in the reference, "hashtag" or
        // "sharp" here.
        let symbol_word = |m: &Masked| matches!(m.token, "number" | "hashtag" | "sharp");
        if theirs.iter().any(|m| m.text.chars().any(is_symbol))
            || (!theirs.is_empty() && theirs.iter().chain(mine).all(symbol_word))
        {
            kinds.insert(Kind::Symbols);
            continue;
        }
        // Words other than numbers must be letters a number was glued to.
        let other_ok = |side: &[Masked], across: &[Masked]| {
            side.iter()
                .filter(|m| m.class() == Class::Other)
                .all(|m| across.iter().any(|x| glued(&m.text, &x.text) || glued(&x.text, &m.text)))
        };
        let near_number = |range: &std::ops::Range<usize>, side: &[Masked]| {
            let before = range.start.checked_sub(1).and_then(|i| side.get(i));
            [before, side.get(range.end)].into_iter().flatten().any(|m| m.token == "<n>")
        };
        let has_number = theirs.iter().chain(mine).any(|m| m.token == "<n>");
        if squeezed(theirs) == squeezed(mine)
            || (other_ok(theirs, mine)
                && other_ok(mine, theirs)
                && (has_number || near_number(&ra, &ma) || near_number(&rb, &mb)))
        {
            kinds.insert(Kind::Numbers);
        } else {
            return Err(format!("reference {:?}, ours {:?}", show(theirs), show(mine)));
        }
    }
    if a != b && kinds.is_empty() {
        // Only the spacing around a number changed.
        kinds.insert(Kind::Numbers);
    }
    Ok(kinds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_number_and_symbol_changes_only() {
        let ok = |a: &str, b: &str| explain(a, b).unwrap();
        let numbers = BTreeSet::from([Kind::Numbers]);
        assert_eq!(ok("It costs one dollars.", "It costs one dollar."), numbers);
        assert_eq!(ok("one thousand, two hundred and five", "one thousand two hundred and five"), numbers);
        assert_eq!(
            ok("In the 1990s, then ∑ x", "In the nineteen nineties, then sum x"),
            BTreeSet::from([Kind::Numbers, Kind::Symbols])
        );
        assert_eq!(
            ok("issn twenty-three to thirty-nine nineteen.", "issn zero zero two three, three nine one nine."),
            numbers
        );
        assert_eq!(ok("USten and vtwo", "US ten and v-two"), numbers);
        assert_eq!(ok("ten km slash h,", "ten kilometers per hour,"), numbers);
        assert_eq!(ok("same", "same"), BTreeSet::new());
        assert!(explain("They're here at five.", "They are here at five.").is_err());
        assert!(explain("nineteen zero apples", "nineteen hundred pears").is_err());
        assert!(explain("the cat", "a cat").is_err());
    }
}
