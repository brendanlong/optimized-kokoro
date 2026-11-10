//! Checks that a difference from the reference's normalized text is one `docs/divergences.md`
//! lists: the number reader ("Numbers"), the symbol pass ("Symbols"), dashes ("Dashes"), Roman
//! numerals ("Roman"), abbreviations ("Abbreviations"), footnote marks ("Footnotes"), rates
//! ("Rates") or money before what it buys ("AttributiveMoney").
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
    /// A spaced hyphen read as a dash, an en dash inside a word read as a hyphen (issue #35).
    Dashes,
    /// A Roman numeral the reference left as letters, read as its number ("World War two",
    /// "Henry eight").
    Roman,
    /// An abbreviation read in full: "St." as "Saint" or "Street", "Dr." as "Doctor" or "Drive",
    /// "No." as "number", "Dec." as "December", "pp." as "pages", "tl;dr" as "TLDR".
    Abbreviations,
    /// A footnote or citation mark dropped: "find.6", "[2]", "↩", "¶", "§ § §".
    Footnotes,
    /// A rate's slash read "per", "a" or "an": "twenty dollars a month", "tok per second".
    Rates,
    /// An amount of money in the singular before what it buys: "a five dollar bill".
    AttributiveMoney,
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
    let degree = matches!(word, "°" | "℃" | "℉" | "C" | "F" | "K");
    if degree || word.chars().any(char::is_numeric) || ABBREVIATIONS.contains(&word) || made_of(word, NUMERALS) {
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
            if c == '/' || is_symbol(c) || (is_plain_punctuation(c) && !matches!(c, '\'' | '’')) {
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
        out.push(Masked { token: "<n>", text: tokens[i..end].join(" ") });
        i = end;
    }
    out
}

/// A longest-common-subsequence diff of two masked texts, compared by token: the pairs that line
/// up, and the ranges that differ.
struct Alignment {
    same: Vec<(usize, usize)>,
    hunks: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>,
}

fn align(a: &[Masked], b: &[Masked]) -> Alignment {
    let same = |x: &Masked, y: &Masked| x.token == y.token;
    let prefix = a.iter().zip(b).take_while(|(x, y)| same(x, y)).count();
    let suffix = a[prefix..].iter().rev().zip(b[prefix..].iter().rev()).take_while(|(x, y)| same(x, y)).count();
    let (n, m) = (a.len() - suffix - prefix, b.len() - suffix - prefix);
    let (ca, cb) = (&a[prefix..prefix + n], &b[prefix..prefix + m]);
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[at(i, j)] =
                if same(&ca[i], &cb[j]) { lcs[at(i + 1, j + 1)] + 1 } else { lcs[at(i + 1, j)].max(lcs[at(i, j + 1)]) };
        }
    }
    let mut out = Alignment { same: (0..prefix).map(|i| (i, i)).collect(), hunks: Vec::new() };
    let (mut i, mut j) = (0, 0);
    let mut start: Option<(usize, usize)> = None;
    while i < n || j < m {
        if i < n && j < m && same(&ca[i], &cb[j]) {
            if let Some((si, sj)) = start.take() {
                out.hunks.push((prefix + si..prefix + i, prefix + sj..prefix + j));
            }
            out.same.push((prefix + i, prefix + j));
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
        out.hunks.push((prefix + si..prefix + n, prefix + sj..prefix + m));
    }
    out.same.extend((0..suffix).map(|k| (a.len() - suffix + k, b.len() - suffix + k)));
    out
}

/// The text without spaces, hyphens, commas or periods.
fn squeezed(tokens: &[Masked]) -> String {
    tokens.iter().flat_map(|w| w.text.chars()).filter(|&c| !"-,. ".contains(c)).collect()
}

/// `word` is `other` with number words glued to it: "USten" and "US", "vtwo" and "v".
fn glued(word: &str, other: &str) -> bool {
    !other.is_empty()
        && (word.strip_prefix(other).is_some_and(|rest| made_of(rest, NUMERALS))
            || word.strip_suffix(other).is_some_and(|rest| made_of(rest, NUMERALS)))
}

/// Whether `word` is a `k`, `m` or `b` that the input has glued to a number ("5k", "Figure 3b"):
/// the reference read it as a multiplier ("thousand"), and the number reader reads it as the letter
/// outside money ("five k").
fn glued_letter(input: &str, word: &str) -> bool {
    matches!(word, "k" | "K" | "m" | "M" | "b" | "B")
        && input.match_indices(word).any(|(k, _)| {
            input[..k].chars().next_back().is_some_and(|c| c.is_ascii_digit())
                && input[k + 1..].chars().next().is_none_or(|c| !c.is_alphanumeric())
        })
}

/// The cardinal number words: a changed run of number words must hold one of these (or a digit)
/// on both sides, so a changed "first" or "hour" alone isn't taken for a number.
const CARDINALS: &[&str] = &[
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
];

/// Whether a run's text holds a digit or a cardinal word (in any number: "nineties").
fn has_cardinal(text: &str) -> bool {
    has_number_word(text, false)
}

/// [`has_cardinal`], or an ordinal of one ("first", "nineteenth", "twentieth").
fn has_cardinal_or_ordinal(text: &str) -> bool {
    has_number_word(text, true)
}

fn has_number_word(text: &str, ordinals: bool) -> bool {
    text.chars().any(char::is_numeric)
        || text.split(|c: char| !c.is_alphabetic()).any(|w| {
            let w = w.to_lowercase();
            let mut forms = vec![w.clone()];
            forms.extend(w.strip_suffix("ies").map(|s| format!("{s}y")));
            forms.extend(w.strip_suffix('s').map(str::to_owned));
            if ordinals {
                forms.extend(w.strip_suffix("ieth").map(|s| format!("{s}y")));
                forms.extend(w.strip_suffix("th").map(str::to_owned));
                forms.extend(
                    match w.as_str() {
                        "first" => Some("one"),
                        "second" => Some("two"),
                        "third" => Some("three"),
                        "fifth" => Some("five"),
                        "eighth" => Some("eight"),
                        "ninth" => Some("nine"),
                        "twelfth" => Some("twelve"),
                        _ => None,
                    }
                    .map(str::to_owned),
                );
            }
            forms.iter().any(|f| CARDINALS.contains(&f.as_str()))
        })
}

/// The value of a Roman numeral in standard form ("XIV" 14, "MCMXCIV" 1994), up to 3999, with a
/// possessive "'s" allowed; `None` for anything else ("IIII", "IC"). A copy of the reader's own
/// (`kokoro-text`'s `roman.rs`), so the check stands apart from it.
fn roman_value(token: &str) -> Option<u32> {
    let token = token.strip_suffix("'s").or_else(|| token.strip_suffix("’s")).unwrap_or(token);
    let digit = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let values: Vec<i64> = token.chars().map(digit).collect::<Option<_>>()?;
    let n: i64 =
        values.iter().enumerate().map(|(i, &v)| if values.get(i + 1).is_some_and(|&w| w > v) { -v } else { v }).sum();
    let n = u32::try_from(n).ok().filter(|n| (1..4000).contains(n))?;
    let mut canonical = String::new();
    let mut rest = n;
    for (v, s) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while rest >= v {
            canonical.push_str(s);
            rest -= v;
        }
    }
    (canonical == token).then_some(n)
}

/// The readings of a number below 4000 the checker accepts: the cardinal ("fifty-five", "one
/// thousand two hundred and five"), from 1000 a year in pairs ("nineteen ninety-four", "nineteen
/// hundred", "nineteen oh five"). Built here rather than taken from the number reader, so that the
/// check doesn't trust the code it checks.
fn readings_of(v: u32) -> Vec<String> {
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
    let under_100 = |n: u32| match n {
        0..20 => ONES[n as usize].to_owned(),
        _ if n.is_multiple_of(10) => TENS[n as usize / 10].to_owned(),
        _ => format!("{}-{}", TENS[n as usize / 10], ONES[n as usize % 10]),
    };
    let under_1000 = |n: u32| match (n / 100, n % 100) {
        (0, rest) => under_100(rest),
        (h, 0) => format!("{} hundred", ONES[h as usize]),
        (h, rest) => format!("{} hundred and {}", ONES[h as usize], under_100(rest)),
    };
    let cardinal = match (v / 1000, v % 1000) {
        (0, rest) => under_1000(rest),
        (t, 0) => format!("{} thousand", ONES[t as usize]),
        (t, rest) if rest < 100 => format!("{} thousand and {}", ONES[t as usize], under_100(rest)),
        (t, rest) => format!("{} thousand {}", ONES[t as usize], under_1000(rest)),
    };
    let mut out = vec![cardinal];
    if v >= 1000 {
        let (high, low) = (under_100(v / 100), v % 100);
        out.push(match low {
            0 => format!("{high} hundred"),
            1..10 => format!("{high} oh {}", ONES[low as usize]),
            _ => format!("{high} {}", under_100(low)),
        });
    }
    out
}

/// Whether `ours` is `theirs` with Roman numerals read as their numbers (one of
/// [`readings_of`], keeping the numeral's "'s": "Louis XIV's" against "Louis fourteen's"),
/// and nothing else changed. Each numeral must be a word of the input. `previous` is the numeral
/// just read, if the token before `theirs` was one.
fn roman_read(input: &str, theirs: &[&str], ours: &[&str], previous: Option<&str>) -> bool {
    let Some((&first, rest)) = theirs.split_first() else {
        return ours.is_empty();
    };
    let core = first.strip_suffix("'s").or_else(|| first.strip_suffix("’s")).unwrap_or(first);
    let in_input = || input.split(|c: char| !c.is_alphanumeric()).any(|w| w == core);
    let Some(v) = roman_value(first).filter(|_| in_input()) else {
        // A range's dash between two numerals read "to" ("Phases I-III" as "one to three"): the
        // numeral before it was read, and the input has the two around the dash.
        let range = matches!(first, "-" | "–")
            && ours.first() == Some(&"to")
            && previous.zip(rest.first()).is_some_and(|(p, n)| {
                roman_value(n).is_some() && [format!("{p}-{n}"), format!("{p}–{n}")].iter().any(|s| input.contains(s))
            });
        return (ours.first() == Some(&first) || range) && roman_read(input, rest, &ours[1..], None);
    };
    let suffix = &first[core.len()..];
    // The readings of a value start with different words, so at most one matches: no backtracking.
    readings_of(v).into_iter().any(|reading| {
        let reading = format!("{reading}{suffix}");
        let words = tokens(&reading);
        ours.len() >= words.len()
            && ours.iter().zip(&words).all(|(a, b)| a == b)
            && roman_read(input, rest, &ours[words.len()..], Some(first))
    })
}

/// The reference's tokens with each differing run that `ours` reads as Roman numerals (see
/// [`roman_read`]) replaced by ours, and whether there was one.
fn with_roman_read<'a>(input: &str, a: &[&'a str], b: &[&'a str]) -> (Vec<&'a str>, bool) {
    if !a.iter().any(|w| roman_value(w).is_some()) {
        return (a.to_vec(), false);
    }
    let wrap = |side: &[&'a str]| -> Vec<Masked<'a>> {
        side.iter().map(|&w| Masked { token: w, text: w.to_owned() }).collect()
    };
    let (mut out, mut last, mut roman) = (Vec::new(), 0, false);
    for (ra, rb) in align(&wrap(a), &wrap(b)).hunks {
        if a[ra.clone()].iter().any(|w| roman_value(w).is_some())
            && roman_read(input, &a[ra.clone()], &b[rb.clone()], None)
        {
            out.extend_from_slice(&a[last..ra.start]);
            out.extend_from_slice(&b[rb]);
            last = ra.end;
            roman = true;
        }
    }
    out.extend_from_slice(&a[last..]);
    (out, roman)
}

type Span = std::ops::Range<usize>;

/// The reference's tokens with part of each differing run replaced by ours where `accept` explains
/// it, and whether there was one. `accept` is given the reference's tokens, the run's range in
/// them and our run, and returns the part of each run (relative ranges) it explains; the rest of
/// the run is left for the other checks.
fn with_hunks_read<'a>(
    a: &[&'a str],
    b: &[&'a str],
    accept: impl Fn(&[&str], Span, &[&str]) -> Option<(Span, Span)>,
) -> (Vec<&'a str>, bool) {
    let wrap = |side: &[&'a str]| -> Vec<Masked<'a>> {
        side.iter().map(|&w| Masked { token: w, text: w.to_owned() }).collect()
    };
    let (mut out, mut last, mut any) = (Vec::new(), 0, false);
    for (ra, rb) in align(&wrap(a), &wrap(b)).hunks {
        if let Some((theirs, ours)) = accept(a, ra.clone(), &b[rb.clone()]) {
            out.extend_from_slice(&a[last..ra.start + theirs.start]);
            out.extend_from_slice(&b[rb.start + ours.start..rb.start + ours.end]);
            last = ra.start + theirs.end;
            any = true;
        }
    }
    out.extend_from_slice(&a[last..]);
    (out, any)
}

/// `check` on whole runs, as [`with_hunks_read`] takes it.
fn whole(check: bool, theirs: Span, ours: usize) -> Option<(Span, Span)> {
    check.then_some((0..theirs.len(), 0..ours))
}

/// Whether `word` is a word of the input.
fn input_word(input: &str, word: &str) -> bool {
    input.split(|c: char| !c.is_alphanumeric()).any(|w| w == word)
}

/// The readings of an abbreviation the abbreviation pass expands (`kokoro-text`'s
/// `abbreviations.rs`), written out here so the check stands apart from it.
fn expansions(abbreviation: &str) -> &'static [&'static str] {
    match abbreviation {
        "St" => &["Saint", "Street"],
        "Dr" | "DR" => &["Doctor", "Drive"],
        "Doctor" => &["Drive"],
        "No" => &["Number"],
        "no" => &["number"],
        "Nos" => &["Numbers"],
        "nos" => &["numbers"],
        "MR" => &["Mister"],
        "MRS" => &["Mrs"],
        "Jan" => &["January"],
        "Feb" => &["February"],
        "Mar" => &["March"],
        "Apr" => &["April"],
        "Jun" => &["June"],
        "Jul" => &["July"],
        "Aug" => &["August"],
        "Sep" | "Sept" => &["September"],
        "Oct" => &["October"],
        "Nov" => &["November"],
        "Dec" => &["December"],
        "p" => &["page"],
        "pp" => &["pages"],
        "Vol" => &["Volume"],
        "vol" => &["volume"],
        "ca" => &["circa"],
        "esp" => &["especially"],
        "v" => &["versus"],
        _ => &[],
    }
}

/// Whether `ours` is `theirs` with an abbreviation read in full ("St ." as "Saint", "Dec ." as
/// "December", "tl ; dr" as "TLDR"), periods aside. The abbreviation must be a word of the input.
/// The part of a differing run that [`abbreviation_read`] explains: the whole run, or its start or
/// end ("Jan . 5 th" against "January fifth", whose number is left for the number check).
fn abbreviation_part(input: &str, theirs: &[&str], ours: &[&str]) -> Option<(Span, Span)> {
    let (n, m) = (theirs.len(), ours.len());
    for t in (1..=n.min(4)).rev() {
        for o in (1..=m.min(2)).rev() {
            if abbreviation_read(input, &theirs[..t], &ours[..o]) {
                return Some((0..t, 0..o));
            }
            if abbreviation_read(input, &theirs[n - t..], &ours[m - o..]) {
                return Some((n - t..n, m - o..m));
            }
        }
    }
    None
}

fn abbreviation_read(input: &str, theirs: &[&str], ours: &[&str]) -> bool {
    let dotted = theirs.contains(&".");
    let words = |side: &[&str]| -> Vec<String> { side.iter().filter(|&&t| t != ".").map(|t| t.to_string()).collect() };
    let (theirs, ours) = (words(theirs), words(ours));
    let [reading] = ours.as_slice() else { return false };
    let tldr = |t: &[String]| match t {
        [one] => one.eq_ignore_ascii_case("tldr"),
        [tl, semicolon, dr] => tl.eq_ignore_ascii_case("tl") && semicolon == ";" && dr.eq_ignore_ascii_case("dr"),
        _ => false,
    };
    if reading == "TLDR" && tldr(&theirs) {
        return input.to_lowercase().contains("tldr") || input.to_lowercase().contains("tl;dr");
    }
    let [abbreviation] = theirs.as_slice() else { return false };
    let abbreviation = abbreviation.as_str();
    // Only "St" and "Dr" are expanded without their period ("St Paul"), and a month before a
    // number ("Oct 20"): the input must have the abbreviation, then a space and a digit.
    let month = matches!(
        abbreviation,
        "Jan" | "Feb" | "Mar" | "Apr" | "Jun" | "Jul" | "Aug" | "Sep" | "Sept" | "Oct" | "Nov" | "Dec"
    );
    let before_number = input.match_indices(&format!("{abbreviation} ")).any(|(k, m)| {
        input[k + m.len()..].starts_with(|c: char| c.is_ascii_digit())
            && input[..k].chars().next_back().is_none_or(|c| !c.is_alphanumeric())
    });
    let period_ok = dotted || matches!(abbreviation, "St" | "Dr" | "Doctor") || month && before_number;
    let in_input = if abbreviation == "Doctor" { input.contains("Dr.") } else { input_word(input, abbreviation) };
    in_input && period_ok && expansions(abbreviation).contains(&reading.as_str())
}

/// Whether `ours` drops the reference's run `a[run]` as a footnote or citation mark: a back-link
/// arrow (and its number), a pilcrow, section sign or diamond; a bracketed number or list of them
/// ("[ two ]", "[ 3 , 4 ]"); or one or two digits (or their words) right after the punctuation that
/// ends a word ("find . six"). The input must have such a mark.
fn footnote_dropped(input: &str, a: &[&str], run: std::ops::Range<usize>, ours: &[&str]) -> bool {
    let theirs = &a[run.clone()];
    if !ours.is_empty() || theirs.is_empty() {
        return false;
    }
    let marks = ['↩', '¶', '§', '♦', '◆'];
    let number =
        |t: &&str| (t.chars().all(|c| c.is_ascii_digit()) && t.len() <= 3) || made_of(t, NUMERALS) && has_cardinal(t);
    let mark = |t: &&str| t.chars().all(|c| marks.contains(&c) || matches!(c, '\u{FE0E}' | '\u{FE0F}'));
    // Marks, and the number of a back-link ("↩ 2"); each mark must be in the input.
    let marks_only = theirs.iter().enumerate().all(|(k, t)| {
        mark(t) && t.chars().all(|c| input.contains(c) || matches!(c, '\u{FE0E}' | '\u{FE0F}'))
            || number(t) && k > 0 && theirs[k - 1].contains('↩')
    });
    if theirs.iter().any(mark) && marks_only {
        return true;
    }
    if let ["[", inner @ .., "]"] = theirs {
        let list = |t: &&str| number(t) || matches!(*t, "," | "-" | "–" | "to" | "and");
        return input.contains('[') && !inner.is_empty() && inner.iter().all(list);
    }
    // A marker glued to a word's punctuation: the input has that word, the punctuation and a digit
    // ("find.6"), where the reference has the word, the punctuation and the number.
    // The punctuation (one mark and maybe closing quotes or brackets: ". ”") and the word before.
    let is_mark = |t: &str| !t.is_empty() && t.chars().all(|c| ".,;!?)\"”'’".contains(c));
    let mut start = run.start;
    while start > 0 && run.start - start < 3 && is_mark(a[start - 1]) {
        start -= 1;
    }
    let Some(word) = start.checked_sub(1).map(|k| a[k]) else { return false };
    let punctuation: String = a[start..run.start].concat();
    let punctuation_ok = punctuation.starts_with(['.', ',', ';', '!', '?']);
    let lowercase_word = word.chars().next_back().is_some_and(char::is_lowercase);
    let glued = input
        .match_indices(&format!("{word}{punctuation}"))
        .any(|(k, m)| input[k + m.len()..].starts_with(|c: char| c.is_ascii_digit()));
    punctuation_ok && lowercase_word && glued && theirs.len() == 1 && number(&theirs[0])
}

/// A unit of time's abbreviation and its name, as a rate reads it ("s" "second").
fn time_name(word: &str) -> Option<&'static str> {
    Some(match word.to_ascii_lowercase().as_str() {
        "s" | "sec" | "second" => "second",
        "min" | "minute" => "minute",
        "h" | "hr" | "hour" => "hour",
        "day" => "day",
        "wk" | "week" => "week",
        "mo" | "month" => "month",
        "yr" | "year" => "year",
        _ => return None,
    })
}

/// Whether `ours` is `theirs` with a rate's slash read "per", "a" or "an" ("slash month" as "a
/// month", "GB slash s" as "gigabytes per second"): the word after the slash kept or named, and the
/// one before it kept or read as a unit's name.
fn rate_read(input: &str, a: &[&str], run: Span, ours: &[&str]) -> bool {
    let theirs = &a[run.clone()];
    let Some(i) = theirs.iter().position(|&t| t == "slash" || t == "/") else { return false };
    let Some(j) = ours.iter().position(|&t| matches!(t, "per" | "a" | "an")) else { return false };
    let (before, after) = (&theirs[..i], &theirs[i + 1..]);
    let (our_before, our_after) = (&ours[..j], &ours[j + 1..]);
    let before_ok = match (before, our_before) {
        ([], []) => true,
        // A unit read in full ("GB" as "gigabytes"), not a number.
        ([x], [y]) => x == y || x.len() <= 8 && made_of(y, NUMERALS) && !has_cardinal(y) && !has_cardinal(x),
        _ => false,
    };
    // After the slash, a unit of time ("a month", "per second"), or after money any word ("per
    // seat").
    // Money: the reference's word before the slash is a currency ("dollars slash seat").
    let before_slash = (run.start + i).checked_sub(1).map(|k| a[k]);
    let money = input.contains(['$', '£', '€', '¥', '₹'])
        && before_slash.is_some_and(|w| made_of(w, NUMERALS) && !has_cardinal(w));
    // The word after the run when the run ends at the slash's reading.
    let next = a.get(run.end).copied();
    let (after, our_after) = match (after, our_after, next) {
        ([], [], Some(next)) => (vec![next], vec![next]),
        _ => (after.to_vec(), our_after.to_vec()),
    };
    let after_ok = match (after.as_slice(), our_after.as_slice()) {
        ([x], [y]) => time_name(x) == Some(y) || x == y && (time_name(y).is_some() || money && ours[j] == "per"),
        _ => false,
    };
    input.contains('/') && before_ok && after_ok
}

/// Whether a run of number words in ours reads an amount in the singular before what it buys
/// ("five dollar") where the reference's is plural ("five dollars").
fn attributive_money(theirs: &str, ours: &str) -> bool {
    const SINGULAR: [&str; 11] =
        ["dollar", "pound", "euro", "rupee", "ruble", "bitcoin", "peso", "shekel", "won", "yen", "lira"];
    let words: Vec<&str> = ours.split([' ', '-']).collect();
    words.iter().enumerate().any(|(k, w)| {
        SINGULAR.contains(w) && k > 0 && words[k - 1] != "one" && theirs.split(' ').any(|t| t == format!("{w}s"))
    })
}

/// Whether `text` has "dr" or "mr" in a case other than "Dr" and "Mr" ("DR Congo", "the MR
/// role", "tl;dr"), which misaki's lexicons read "doctor" and "mister" and the port spells out or
/// leaves to espeak (`docs/divergences.md`, "misaki English: lowercase abbreviations").
pub fn has_dropped_abbreviation(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| (w.eq_ignore_ascii_case("dr") || w.eq_ignore_ascii_case("mr")) && !matches!(w, "Dr" | "Mr"))
}

/// Why `ours` differs from the reference's normalized text of `input`: the divergences that
/// explain every difference, or the first one nothing explains. A number difference needs a
/// number in the input (a digit or another number character: `²`, `½`, `①`), and a symbol
/// difference a symbol.
pub fn explain(input: &str, reference: &str, ours: &str) -> Result<BTreeSet<Kind>, String> {
    let input_numbers = input.chars().any(char::is_numeric);
    let input_symbols = input.chars().any(is_symbol);
    let (a, b) = (tokens(reference), tokens(ours));
    let mut kinds = BTreeSet::new();
    // Roman numerals read as numbers are taken out first, word by word: masking would join a
    // reading to the number words around it ("first of the Two", "five . The Five").
    let (a, roman) = with_roman_read(input, &a, &b);
    if roman {
        kinds.insert(Kind::Roman);
    }
    // So are abbreviations read in full, dropped footnote marks and rates' slashes.
    let (a, abbreviations) = with_hunks_read(&a, &b, |a, run, ours| abbreviation_part(input, &a[run], ours));
    let (a, footnotes) =
        with_hunks_read(&a, &b, |a, run, ours| whole(footnote_dropped(input, a, run.clone(), ours), run, ours.len()));
    let (a, rates) =
        with_hunks_read(&a, &b, |a, run, ours| whole(rate_read(input, a, run.clone(), ours), run, ours.len()));
    for (found, kind) in [(abbreviations, Kind::Abbreviations), (footnotes, Kind::Footnotes), (rates, Kind::Rates)] {
        if found {
            kinds.insert(kind);
        }
    }
    let (ma, mb) = (mask(&a), mask(&b));
    let alignment = align(&ma, &mb);
    let unexplained = |what: String| Err(format!("{what} (input {input:?})"));
    // Runs of numbers read differently, where the rest lines up.
    let mut changed_numbers = std::collections::HashSet::new();
    for &(i, j) in &alignment.same {
        if ma[i].token == "<n>" && ma[i].text != mb[j].text {
            // A number glued to the word before it in the reference ("abfive dollars") counts too.
            let glued_before = i > 0
                && ma[i - 1].class() == Class::Other
                && CARDINALS.iter().any(|w| ma[i - 1].text.to_lowercase().ends_with(w));
            let digit = ma[i].text.chars().any(char::is_numeric) || glued_before;
            if !input_numbers || !(digit || has_cardinal(&ma[i].text) && has_cardinal_or_ordinal(&mb[j].text)) {
                return unexplained(format!("number words {:?} became {:?}", ma[i].text, mb[j].text));
            }
            changed_numbers.insert(i);
            kinds.insert(Kind::Numbers);
            if attributive_money(&ma[i].text, &mb[j].text) {
                kinds.insert(Kind::AttributiveMoney);
            }
        }
    }
    for (ra, rb) in alignment.hunks {
        let (theirs, mine) = (&ma[ra.clone()], &mb[rb.clone()]);
        let show = |side: &[Masked]| side.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join(" ");
        let what = || format!("reference {:?}, ours {:?}", show(theirs), show(mine));
        // Dashes: the same text but for hyphens, en dashes, em dashes and spaces ("of - a" against
        // "of — a", "US- China" against "US-China").
        let dashless = |side: &[Masked]| -> String {
            side.iter().flat_map(|m| m.text.chars()).filter(|c| !"-–— ".contains(*c)).collect()
        };
        if input.contains(['-', '–']) && dashless(theirs) == dashless(mine) {
            kinds.insert(Kind::Dashes);
            continue;
        }
        // A range of letters: "a - z" against "A to Z".
        let letter = |m: &Masked| m.text.len() == 1 && m.text.chars().all(|c| c.is_ascii_alphabetic());
        if input.contains(['-', '–'])
            && let [a, to, b] = mine
            && letter(a)
            && to.text == "to"
            && letter(b)
            && dashless(theirs).to_lowercase() == format!("{}{}", a.text, b.text).to_lowercase()
        {
            kinds.insert(Kind::Dashes);
            continue;
        }
        // A symbol the reference left in the text, or read as "number" where this reads "hashtag"
        // or "sharp": the reference's other words must all still be there.
        let symbol_word = |m: &Masked| matches!(m.token, "number" | "hashtag" | "sharp");
        if input_symbols
            && (theirs.iter().any(|m| m.text.chars().any(is_symbol))
                || (!theirs.is_empty() && theirs.iter().chain(mine).all(symbol_word)))
        {
            let kept = theirs
                .iter()
                .filter(|m| m.class() == Class::Other && !m.text.chars().any(is_symbol) && !symbol_word(m));
            if kept.clone().all(|m| mine.iter().any(|x| x.text == m.text)) {
                kinds.insert(Kind::Symbols);
                continue;
            }
            return unexplained(what());
        }
        if !input_numbers {
            return unexplained(what());
        }
        // Words other than numbers must be letters a number was glued to, and number runs must
        // hold a number.
        // A letter the input glued to a number may be read after that number's words ("five k").
        let other_ok = |full: &[Masked], run: std::ops::Range<usize>, across: &[Masked]| {
            run.filter(|&k| full[k].class() == Class::Other).all(|k| {
                let m = &full[k];
                across.iter().any(|x| glued(&m.text, &x.text) || glued(&x.text, &m.text))
                    || k > 0 && full[k - 1].token == "<n>" && glued_letter(input, &m.text)
            })
        };
        let numbers_ok =
            theirs.iter().chain(mine).filter(|m| m.token == "<n>").all(|m| has_cardinal_or_ordinal(&m.text));
        let number = |side: &[Masked]| side.iter().any(|m| m.token == "<n>");
        let has_number = number(theirs) && number(mine);
        // Connectives or punctuation alone changed next to a number that was read differently
        // ("1939–1945": "-" against "to").
        let near_changed =
            ra.start.checked_sub(1).is_some_and(|i| changed_numbers.contains(&i)) || changed_numbers.contains(&ra.end);
        if squeezed(theirs) == squeezed(mine)
            || (other_ok(&ma, ra.clone(), mine)
                && other_ok(&mb, rb.clone(), theirs)
                && numbers_ok
                && (has_number || near_changed))
        {
            kinds.insert(Kind::Numbers);
            if attributive_money(&show(theirs), &show(mine)) {
                kinds.insert(Kind::AttributiveMoney);
            }
        } else {
            return unexplained(what());
        }
    }
    if a != b && kinds.is_empty() {
        if !input_numbers && !input_symbols {
            return unexplained("spacing changed".into());
        }
        // Only the spacing around a number or symbol changed.
        kinds.insert(if input_numbers { Kind::Numbers } else { Kind::Symbols });
    }
    Ok(kinds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explains_number_and_symbol_changes_only() {
        let ok = |input: &str, a: &str, b: &str| explain(input, a, b).unwrap();
        let numbers = BTreeSet::from([Kind::Numbers]);
        assert_eq!(ok("It costs $1.", "It costs one dollars.", "It costs one dollar."), numbers);
        assert_eq!(ok("1205", "one thousand, two hundred and five", "one thousand two hundred and five"), numbers);
        assert_eq!(
            ok("In the 1990s, then ∑ x", "In the 1990s, then ∑ x", "In the nineteen nineties, then sum x"),
            BTreeSet::from([Kind::Numbers, Kind::Symbols])
        );
        assert_eq!(
            ok(
                "ISSN 0023-3919.",
                "issn twenty-three to thirty-nine nineteen.",
                "issn zero zero two three, three nine one nine."
            ),
            numbers
        );
        assert_eq!(ok("US$10 and v2", "USten and vtwo", "US ten and v-two"), numbers);
        assert_eq!(ok("10 km/h,", "ten km slash h,", "ten kilometers per hour,"), BTreeSet::from([Kind::Rates]));
        assert_eq!(
            ok("1939–1945", "nineteen thirty-nine- nineteen forty-five", "nineteen thirty-nine to nineteen forty-five"),
            numbers
        );
        assert_eq!(ok("same", "same", "same"), BTreeSet::new());
        // A glued letter read as a letter, where the reference read a multiplier.
        assert_eq!(
            ok(
                "5k, 3m and Figure 3b",
                "five thousand, three million and Figure three billion",
                "five k, three m and Figure three b"
            ),
            numbers
        );
        let err = |input: &str, a: &str, b: &str| assert!(explain(input, a, b).is_err(), "{a:?} -> {b:?}");
        err("They're here at 5.", "They're here at five.", "They are here at five.");
        err("1900 apples", "nineteen zero apples", "nineteen hundred pears");
        err("the cat", "the cat", "a cat");
        // The review's examples of unrelated changes: none may pass for a number or symbol.
        err("He came 1st and left at 5.", "He came first and left at five.", "He came second and left at five.");
        err("the hour is 5", "the hour is five", "the hours is five");
        err("the s is 5", "the s is five", "the m is five");
        err("I have 1 cat.", "I have one cat.", "I have a cat.");
        err("at 5 the cat", "at five the cat", "at five a cat");
        err("5! Run", "five! Run", "five? Run");
        err("Tom © Jerry Smith", "Tom © Jerry Smith", "Tom copyright Gerry Smyth");
        err("no numbers here", "nineteen zero", "nineteen hundred");
        err("Plan B at 5", "Plan B at five", "Plan b at five");
        err("Plan B at 5", "Plan billion at five", "Plan B at five");
        err("Plan b and 3b", "Plan be and three billion", "Plan b and three billion");
        assert_eq!(
            ok("think of - a, US–China", "think of - a, US- China", "think of — a, US-China"),
            BTreeSet::from([Kind::Dashes])
        );
        assert_eq!(ok("from a–z", "from a - z", "from A to Z"), BTreeSet::from([Kind::Dashes]));
        err("from a z", "from a z", "from A to Z");
        err("think of a", "think of a", "think of — a");
    }

    #[test]
    fn explains_roman_numerals_read_as_numbers_only() {
        let ok = |input: &str, a: &str, b: &str| explain(input, a, b).unwrap();
        let roman = BTreeSet::from([Kind::Roman]);
        assert_eq!(ok("World War II ended.", "World War II ended.", "World War two ended."), roman);
        assert_eq!(ok("Henry VIII's wives", "Henry VIII's wives", "Henry eight's wives"), roman);
        assert_eq!(ok("Super Bowl LV", "Super Bowl LV", "Super Bowl fifty-five"), roman);
        assert_eq!(ok("XXI and XXIV", "XXI and XXIV", "twenty-one and twenty-four"), roman);
        assert_eq!(ok("George II and III", "George II and III", "George two and three"), roman);
        assert_eq!(ok("MCMXCIV.", "MCMXCIV.", "nineteen ninety-four."), roman);
        assert_eq!(ok("MCM.", "MCM.", "nineteen hundred."), roman);
        assert_eq!(ok("MMXXIV", "MMXXIV", "two thousand and twenty-four"), roman);
        assert_eq!(ok("II-era", "II-era", "two-era"), roman);
        assert_eq!(ok("Phases I-III were", "Phases I - III were", "Phases one to three were"), roman);
        let err = |input: &str, a: &str, b: &str| assert!(explain(input, a, b).is_err(), "{a:?} -> {b:?}");
        // The wrong number, or anything else changed.
        err("World War II", "World War II", "World War three");
        err("Wait - I know", "Wait - I know", "Wait to one know");
        err("Phases I and III", "Phases I - III", "Phases one to three");
        err("Henry VIII", "Henry VIII", "Henry the seventh");
        err("Henry VIII", "Henry VIII", "Henry eighth");
        err("Super Bowl LV", "Super Bowl LV", "Super Bowl fifty");
        err("World War II ended", "World War II ended", "World War two began");
        err("World War II", "World War II", "the World War two");
        err("IIII", "IIII", "four");
        err("Henry VIII's", "Henry VIII's", "Henry eight");
        // No ordinals: the reader reads every numeral as a cardinal.
        err("Henry VIII", "Henry VIII", "Henry the eighth");
        // Number words in another order or sum, or with words added at a reading's edges (the
        // review's examples).
        err("LV", "LV", "five fifty");
        err("XIV", "XIV", "four ten");
        err("II", "II", "one one");
        err("II", "II", "oh two");
        err("MCMXCIV", "MCMXCIV", "nineteen four ninety");
        err("Henry II", "Henry II", "Henry the one first");
        err("Henry IV", "Henry IV", "Henry the first third");
        err("Henry II", "Henry II", "Henry The second");
        err("War XXI ended", "War XXI ended", "War and twenty-one ended");
        err("War XXI ended", "War XXI ended", "War twenty-one, ended");
        err("War II then", "War II then", "War two - then");
        // A failing hunk of many numerals fails fast (each reading of a value starts differently).
        let many = |unit: &str, end: &str| format!("{}{end}", unit.repeat(40));
        err(&many("II ", "zzz"), &many("II ", "zzz"), &many("two and ", "yyy"));
        // The numeral must be a word of the input.
        err("World War 2", "World War II", "World War two");
        err("World War III", "World War II", "World War two");
        // Next to other number words, which masking would join to the reading.
        assert_eq!(ok("Mystery V. The Five", "Mystery V. The Five", "Mystery five. The Five"), roman);
        assert_eq!(
            ok("Francis I of the Two Sicilies", "Francis I of the Two Sicilies", "Francis one of the Two Sicilies"),
            roman
        );
        assert_eq!(
            ok("World War II, 1945", "World War II, nineteen forty-five", "World War two, nineteen forty-five"),
            roman
        );
        assert_eq!(
            ok("Louis XIV in 1700", "Louis XIV in one thousand, seven hundred", "Louis fourteen in seventeen hundred"),
            BTreeSet::from([Kind::Numbers, Kind::Roman])
        );
    }

    #[test]
    fn explains_abbreviations_footnotes_rates_and_money_before_nouns_only() {
        let ok = |input: &str, a: &str, b: &str| explain(input, a, b).unwrap();
        let kind = |k: Kind| BTreeSet::from([k]);
        assert_eq!(ok("St. Louis", "St. Louis", "Saint Louis"), kind(Kind::Abbreviations));
        assert_eq!(ok("on Main St. and", "on Main St. and", "on Main Street and"), kind(Kind::Abbreviations));
        assert_eq!(ok("Elm Dr. The", "Elm Doctor The", "Elm Drive. The"), kind(Kind::Abbreviations));
        assert_eq!(ok("tl;dr: yes", "tl;dr: yes", "TLDR: yes"), kind(Kind::Abbreviations));
        assert_eq!(ok("No. 5", "No. five", "Number five"), kind(Kind::Abbreviations));
        assert_eq!(
            ok("on Jan. 5th.", "on Jan. 5th.", "on January fifth."),
            BTreeSet::from([Kind::Numbers, Kind::Abbreviations])
        );
        assert_eq!(ok("find.6 None", "find.6 None", "find. None"), kind(Kind::Footnotes));
        assert_eq!(ok("Google [2]. And", "Google [two]. And", "Google. And"), kind(Kind::Footnotes));
        assert_eq!(ok("the end ↩", "the end ↩", "the end"), kind(Kind::Footnotes));
        assert_eq!(ok("he said “so.”4 Then", "he said “so.”4 Then", "he said “so.” Then"), kind(Kind::Footnotes));
        assert_eq!(
            ok("on Oct 20, 2022", "on Oct twenty, twenty twenty-two", "on October twenty, twenty twenty-two"),
            kind(Kind::Abbreviations)
        );
        assert_eq!(ok("$20/month", "twenty dollars slash month", "twenty dollars a month"), kind(Kind::Rates));
        assert_eq!(ok("43 tok/s", "forty-three tok slash s", "forty-three tok per second"), kind(Kind::Rates));
        assert_eq!(ok("$5/seat", "five dollars slash seat", "five dollars per seat"), kind(Kind::Rates));
        assert_eq!(
            ok("a $5 bill", "a five dollars bill", "a five dollar bill"),
            BTreeSet::from([Kind::Numbers, Kind::AttributiveMoney])
        );
        let err = |input: &str, a: &str, b: &str| assert!(explain(input, a, b).is_err(), "{a:?} -> {b:?}");
        // The wrong expansion, or one the input doesn't have.
        err("St. Louis", "St. Louis", "Doctor Louis");
        err("St. Louis", "St. Louis", "Saint Lewis");
        err("Dec. 5", "Dec. five", "November five");
        err("the cat", "the cat", "the Saint");
        err("Dr. X", "Doctor X", "Saint X");
        // Words dropped that aren't a footnote mark, or more than one.
        err("he has 5 cats", "he has five cats", "he has cats");
        err("find.6 None", "find.6 None", "find. One");
        err("find.6 7 None", "find.6 seven None", "find. None");
        err("Google [2]. And", "Google [two] now. And", "Google. And");
        // A slash that isn't a rate's.
        err("he/she", "he slash she", "he per she");
        err("speed in km/h", "speed in km slash h", "speed in km per minute");
        // The review's examples: a word that is also an abbreviation, or a mark or slash elsewhere
        // in the input.
        err("say no to 5 deals", "say no to five deals", "say number to five deals");
        err("p < 0.05", "p less than zero point zero five", "page less than zero point zero five");
        err("Jan said 5 things", "Jan said five things", "January said five things");
        err("no 5 star reviews", "no five star reviews", "number five star reviews");
        err(
            "Fig.2 shows that, 12 people left",
            "Fig.2 shows that, twelve people left",
            "Fig.2 shows that, people left",
        );
        err("he/she paid $5", "he slash she paid five dollars", "he per she paid five dollars");
    }
}
