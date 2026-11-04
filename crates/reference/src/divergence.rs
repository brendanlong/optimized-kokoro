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

/// Why `ours` differs from the reference's normalized text of `input`: the divergences that
/// explain every difference, or the first one nothing explains. A number difference needs a
/// number in the input (a digit or another number character: `²`, `½`, `①`), and a symbol
/// difference a symbol.
pub fn explain(input: &str, reference: &str, ours: &str) -> Result<BTreeSet<Kind>, String> {
    let input_numbers = input.chars().any(char::is_numeric);
    let input_symbols = input.chars().any(is_symbol);
    let (a, b) = (tokens(reference), tokens(ours));
    let (ma, mb) = (mask(&a), mask(&b));
    let alignment = align(&ma, &mb);
    let mut kinds = BTreeSet::new();
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
        }
    }
    for (ra, rb) in alignment.hunks {
        let (theirs, mine) = (&ma[ra.clone()], &mb[rb.clone()]);
        let show = |side: &[Masked]| side.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join(" ");
        let what = || format!("reference {:?}, ours {:?}", show(theirs), show(mine));
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
        let other_ok = |side: &[Masked], across: &[Masked]| {
            side.iter()
                .filter(|m| m.class() == Class::Other)
                .all(|m| across.iter().any(|x| glued(&m.text, &x.text) || glued(&x.text, &m.text)))
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
            || (other_ok(theirs, mine) && other_ok(mine, theirs) && numbers_ok && (has_number || near_changed))
        {
            kinds.insert(Kind::Numbers);
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
        assert_eq!(ok("10 km/h,", "ten km slash h,", "ten kilometers per hour,"), numbers);
        assert_eq!(
            ok("1939–1945", "nineteen thirty-nine- nineteen forty-five", "nineteen thirty-nine to nineteen forty-five"),
            numbers
        );
        assert_eq!(ok("same", "same", "same"), BTreeSet::new());
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
    }
}
