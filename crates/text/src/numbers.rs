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

/// A cardinal from digits: "one thousand two hundred and thirty-four". "and" comes before tens
/// and units, as both readers in the reference had it. Commas (pauses) only between the groups
/// of numbers from a million.
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
    // From a million up, a comma after each group's scale word, before the next group: a short
    // pause (150-175 ms against 75 ms for a word boundary) that lets listeners hear the groups as
    // written ("one million, two hundred and thirty-four thousand, five hundred and sixty-seven").
    let grouped = groups.len() > 2;
    let nonzero: Vec<usize> = (0..groups.len()).rev().filter(|&s| groups[s] != 0).collect();
    for (k, &scale) in nonzero.iter().enumerate() {
        let group = groups[scale];
        let last = k + 1 == nonzero.len();
        if scale == 0 && group < 100 && !words.is_empty() {
            words.push("and".to_owned());
        }
        words.push(under_1000(group));
        if scale > 0 {
            let comma = grouped && !last && !(nonzero[k + 1] == 0 && groups[0] < 100);
            words.push(format!("{}{}", SCALES[scale], if comma { "," } else { "" }));
        }
    }
    words.join(" ")
}

/// Whether a word is one this reader writes for a number ("seven", "twenty", "hundred", "minus").
pub(crate) fn is_number_word(word: &str) -> bool {
    let w = word.to_lowercase();
    ONES.contains(&w.as_str())
        || TENS[2..].contains(&w.as_str())
        || SCALES[1..].contains(&w.as_str())
        || matches!(w.as_str(), "hundred" | "and" | "point" | "minus" | "plus" | "times" | "oh")
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

/// A date: "May fifth, twenty twenty", "January first, two thousand and five".
fn date_words(year: usize, month: usize, day: usize) -> String {
    let year = if year % 1000 > 9 && year >= 1000 { pairs(year) } else { cardinal(&year.to_string()) };
    format!("{} {}, {year}", MONTHS[month - 1], ordinal_of(&under_100(day)))
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

/// " squared", " cubed" or " to the Nth" ("to the sixty-fourth") for an exponent's digits.
pub(crate) fn power(exponent: &str) -> String {
    match exponent {
        "2" => " squared".to_owned(),
        "3" => " cubed".to_owned(),
        e => format!(" to the {}", ordinal_of(&cardinal(e))),
    }
}

/// A negative exponent: " to the minus three".
fn negative_power(exponent: &str) -> String {
    format!(" to the minus {}", cardinal(exponent))
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

/// An amount of money in words: "one dollar", "ninety-nine cents", "one dollar and fifty cents",
/// "one point nine nine nine dollars".
fn money_words(currency: &Currency, body: &Body) -> String {
    let whole = cardinal(&body.int);
    let name = if body.int.trim_start_matches('0') == "1" { currency.one } else { currency.many };
    let cents = body.frac.as_deref().filter(|f| f.bytes().any(|b| b != b'0'));
    match (cents, currency.minor) {
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
    }
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
    // The Greek capital omega and the ohm sign (U+2126).
    ("kΩ", "kilohm", "kilohms"),
    ("k\u{2126}", "kilohm", "kilohms"),
    ("MΩ", "megohm", "megohms"),
    ("M\u{2126}", "megohm", "megohms"),
    ("Ω", "ohm", "ohms"),
    ("\u{2126}", "ohm", "ohms"),
];

/// A unit of time after a slash in a rate ("/s", "/hr", "/month"): its name.
pub(crate) fn time_unit(word: &str) -> Option<&'static str> {
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

/// Whether `word` before a slash and the unit of time `time` (as written) makes a rate. "s", "h"
/// and "min" are also a plural ("file/s"), a pronoun ("he/she") or "minimum" ("max/min"), so before
/// them the word must be a unit ("GB/s", "km/h"), an acronym or bits ("MB/s", "Mbit/s"), or count
/// what is done per second ("tok/s", "req/s").
pub(crate) fn rate_word(word: &str, time: &str) -> bool {
    let ambiguous = matches!(time.to_ascii_lowercase().as_str(), "s" | "h" | "min");
    !ambiguous
        || unit_name(word).is_some()
        // A capital after the first letter ("GB", "kB"), or bits ("Mbit"); not a capitalised word
        // ("Max/min", "Name/s").
        || word.chars().skip(1).any(|c| c.is_ascii_uppercase())
        || word.ends_with("bit")
        || word.ends_with("bits")
        || matches!(word, "tok" | "tokens" | "req" | "requests" | "ops" | "frames" | "samples" | "steps" | "it")
}

/// A unit's plural name ("km" "kilometers"), for a unit written without a number.
pub(crate) fn unit_name(key: &str) -> Option<&'static str> {
    UNITS.iter().find(|(k, _, _)| *k == key).map(|&(_, _, plural)| plural)
}

/// The word for a slash between two numbers that nothing else explains ("4/5 stars", "80/20",
/// "9/11", "part 1/3"): "slash", as the symbol pass reads every other slash. Brendan compared it with
/// "over" by ear and chose "slash" (2026-11-10).
const NUMBER_SLASH: &str = "slash";

/// Nouns that name the numbered parts of things, before which "1/3 of" is a label ("part 1/3 of the
/// thread"), not a fraction.
const NUMBERED_PARTS: &[&str] = &[
    "act", "appendix", "article", "book", "canto", "chapter", "class", "episode", "grade", "level", "part", "phase",
    "plate", "psalm", "round", "scene", "season", "section", "series", "stage", "step", "tier", "title", "type",
    "volume",
];

/// Multipliers written as words after a number (case-insensitive, as the reference matched them).
const MULTIPLIER_WORDS: [&str; 6] = ["hundred", "thousand", "million", "billion", "trillion", "quadrillion"];

/// Words that link an amount to what follows rather than name what it buys: prepositions,
/// conjunctions, determiners, pronouns and scale words ("$5 of", "$5 or $10", "a $5 the").
/// "five dollars" stays plural before them, even after "a".
const LINKING_WORDS: &[&str] = &[
    "of", "in", "to", "for", "from", "and", "or", "nor", "but", "so", "yet", "if", "as", "than", "per", "on", "at",
    "by", "with", "into", "onto", "over", "under", "about", "after", "before", "since", "until", "upon", "via", "vs",
    "versus", "within", "without", "across", "against", "among", "between", "through", "toward", "towards", "off",
    "up", "down", "out", "plus", "minus", "times", "when", "while", "because", "though", "although", "unless",
    "whereas", "a", "an", "the", "it", "its", "that", "this", "these", "those", "i", "you", "we", "they", "he", "she",
    "him", "her", "them", "us", "me", "my", "your", "our", "their", "his", "some", "any", "no", "which", "who", "whom",
    "whose", "what", "where", "hundred", "thousand", "million", "billion", "trillion",
];

/// Words that follow an amount without being what it buys: quantities, verbs and adverbs ("$5
/// more", "$5 each", "$5 is", "$5 now"). Together with words ending in "-ed" ("$5 raised"), "-ly"
/// ("$300 monthly") and "-ing" ("£220 standing to his credit"), "five dollars" stays plural before
/// them unless "a" comes first ("a $26 million funding round").
const AFTER_AMOUNT: &[&str] = &[
    "more",
    "less",
    "worth",
    "apiece",
    "each",
    "every",
    "total",
    "instead",
    "once",
    "twice",
    "back",
    "extra",
    "cash",
    "net",
    "gross",
    "flat",
    "tops",
    "max",
    "min",
    "is",
    "was",
    "were",
    "are",
    "be",
    "been",
    "being",
    "will",
    "would",
    "can",
    "could",
    "should",
    "shall",
    "may",
    "might",
    "must",
    "has",
    "have",
    "had",
    "do",
    "does",
    "did",
    "gets",
    "got",
    "goes",
    "went",
    "buys",
    "bought",
    "pays",
    "paid",
    "costs",
    "cost",
    "makes",
    "made",
    "now",
    "today",
    "tonight",
    "yesterday",
    "tomorrow",
    "ago",
    "later",
    "already",
    "still",
    "just",
    "only",
    "even",
    "too",
    "also",
    "then",
    "here",
    "there",
    "again",
    "anyway",
    "either",
    "neither",
    "not",
    "short",
    "shy",
    "cheaper",
    "higher",
    "lower",
    "richer",
    "poorer",
    "last",
    "next",
    "this",
    "annually",
    "daily",
    "weekly",
    "monthly",
    "yearly",
    "hourly",
    "etc",
    "cut",
    "set",
    "put",
    "spent",
    "left",
    "lost",
    "won",
    "came",
    "comes",
    "due",
    "owed",
    "saw",
    "see",
    "sees",
    "remains",
    "remain",
    "seems",
    "seem",
    "stays",
    "covers",
    "means",
    "sits",
    "buy",
    "pay",
    "go",
    "get",
    "says",
    "say",
    "upfront",
    "abroad",
    "overseas",
    "overall",
    "altogether",
    "whenever",
    "wherever",
    "however",
    "anyway",
    "apiece",
    "outright",
    "instead",
    "beforehand",
    "afterwards",
];

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
    /// Hyphens inside arithmetic ([`arithmetic_minus`]): "minus", not a range's "to".
    minus: Vec<bool>,
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
        // Thousands ("1,234,567"), or Indian grouping ("1,00,000", "12,34,56,789"): only from a
        // first group without a leading zero, and the groups must end the number.
        if (1..=3).contains(&(j - i)) && !int.starts_with('0') {
            let group = |at: usize, n: usize| self.get(at) == Some(',') && self.digits_end(at + 1) == at + 1 + n;
            let mut k = j;
            while group(k, 3) {
                k += 4;
            }
            if k == j && j - i <= 2 {
                let mut m = j;
                // At most eight pairs (past a quadrillion), so a run of "1,11,11,…" stays linear.
                while group(m, 2) && m - j < 3 * 8 {
                    m += 3;
                }
                if m > j && group(m, 3) {
                    k = m + 4;
                }
            }
            if k > j {
                int.extend(
                    self.t[j..k]
                        .iter()
                        .filter(|&&c| c != ',')
                        .map(|&c| char::from(b'0' + digit_value(c).expect("a digit") as u8)),
                );
                commas = true;
                j = k;
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
        Some(date_words(year, month, day))
    }

    /// A date with slashes at `i`, month first (`7/31/2020`, `4/25/24`), or day first when the
    /// first part can't be a month (`31/7/2020`): its words and end. When both parts could be the
    /// month, the month comes first, as in American writing.
    fn slash_date(&self, i: usize) -> Option<(String, usize)> {
        let part = |start: usize, lengths: &[usize]| -> Option<(usize, usize)> {
            let end = self.digits_end(start);
            lengths.contains(&(end - start)).then(|| Some((self.ascii(start..end).parse().ok()?, end))).flatten()
        };
        let (a, a_end) = part(i, &[1, 2])?;
        let (b, b_end) = part(a_end + 1, &[1, 2]).filter(|_| self.get(a_end) == Some('/'))?;
        let (year, y_end) = part(b_end + 1, &[2, 4]).filter(|_| self.get(b_end) == Some('/'))?;
        let after_ok = self.free_at(y_end)
            && !matches!(self.get(y_end), Some('/' | '-'))
            && !(matches!(self.get(y_end), Some('.' | ',')) && self.digit(y_end + 1));
        let before_ok = i == 0 || !matches!(self.t[i - 1], '/' | '-' | '.');
        // With a two-digit year, three numbers are as often a rule or a split ("the 10/20/30 rule",
        // "a 5/10/15 minute workout"): a date only after a word that says so ("on 4/25/24").
        let year_ok = y_end - b_end - 1 == 4 || self.date_word_before(i);
        if !after_ok || !before_ok || !year_ok {
            return None;
        }
        let (month, day) = match (a, b) {
            (1..=12, 1..=31) => (a, b),
            (13..=31, 1..=12) => (b, a),
            _ => return None,
        };
        if day > [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1] {
            return None;
        }
        let words = if y_end - b_end - 1 == 2 {
            let year = if year < 10 { format!("oh {}", ONES[year]) } else { under_100(year) };
            format!("{} {}, {year}", MONTHS[month - 1], ordinal_of(&under_100(day)))
        } else {
            date_words(year, month, day)
        };
        Some((words, y_end))
    }

    /// Whether the word before the number at `i` (a space, or ": " after it) introduces a date: "on",
    /// "since", "dated", "Date:", a weekday ("Monday 4/25/24", "Mon, 4/25/24").
    fn date_word_before(&self, i: usize) -> bool {
        let mut k = i;
        while k > 0 && matches!(self.t[k - 1], ' ' | ',' | ':') {
            k -= 1;
        }
        if k == i {
            return false;
        }
        let start = self.t[..k].iter().rposition(|c| !c.is_alphabetic()).map_or(0, |s| s + 1);
        let word: String = self.t[start..k].iter().collect::<String>().to_lowercase();
        matches!(
            word.as_str(),
            "on" | "since"
                | "until"
                | "till"
                | "dated"
                | "date"
                | "born"
                | "died"
                | "due"
                | "expires"
                | "updated"
                | "posted"
                | "published"
                | "monday"
                | "tuesday"
                | "wednesday"
                | "thursday"
                | "friday"
                | "saturday"
                | "sunday"
                | "mon"
                | "tue"
                | "tues"
                | "wed"
                | "thu"
                | "thurs"
                | "fri"
                | "sat"
                | "sun"
        )
    }

    /// The letters right before `k`: "part" in "part 1/3" (with `k` at the space).
    fn word_ending_at(&self, k: usize) -> String {
        let start = self.t[..k].iter().rposition(|c| !c.is_alphabetic()).map_or(0, |s| s + 1);
        self.t[start..k].iter().collect()
    }

    /// Whether the word of letters at `i` is `word` (lowercase), in any case, as a whole word.
    fn word_is(&self, i: usize, word: &str) -> bool {
        self.at_ignore_case(i, word) && self.free_at(i + word.len())
    }

    /// Whether the number at `i` is part of a name joined by hyphens ("Llama-3.1-8B",
    /// "GPT-4", "H-1B"): right after a hyphen, in a run of letters, digits, dots and hyphens that
    /// starts with a capital. A hyphen after it isn't a range, and a letter glued to it isn't a
    /// multiplier.
    fn in_name(&self, i: usize) -> bool {
        if i < 2 || self.t[i - 1] != '-' {
            return false;
        }
        // At most 32 characters back, so a long run of "a-1a-1a-…" stays linear.
        let mut k = i - 1;
        while k > 0 && i - k < 32 && (is_alnum(self.t[k - 1]) || matches!(self.t[k - 1], '.' | '-')) {
            k -= 1;
        }
        self.t[k].is_ascii_uppercase() && (k == 0 || !is_alnum(self.t[k - 1]) && !matches!(self.t[k - 1], '.' | '-'))
    }

    /// Whether "'s" after the number at `i` is a possessive ("Opus 4's", "GPT-4's", "I-645's"),
    /// not a plural ("the 1990's", "Londoners 1890's style", "1's and 0's"): the number is part of
    /// a name, or comes after a capitalised word and isn't a round number (a decade).
    fn possessive(&self, i: usize, body: &Body) -> bool {
        if self.in_name(i) {
            return true;
        }
        if i < 3 || self.t[i - 1] != ' ' {
            return false;
        }
        let mut start = i - 1;
        while start > 0 && self.t[start - 1].is_alphabetic() {
            start -= 1;
        }
        i - 1 - start >= 2 && self.t[start].is_uppercase() && !body.int.ends_with('0')
    }

    /// The number expression starting at `i`, if one does: its end and its words.
    /// `plain` says a number here isn't glued to what's before (an operator was read there).
    fn expression(&self, i: usize, plain: bool) -> Option<(usize, String, bool)> {
        let c = self.t[i];
        let free = plain || self.free_before(i);
        match c {
            '~' if free
                && (self.number_at(i + 1)
                    || self.get(i + 1).is_some_and(|c| currency(c).is_some()) && self.number_at(i + 2)) =>
            {
                let (end, words, after) = self.expression(i + 1, true)?;
                // "a ~$100k watch": "a roughly …", as "a about" isn't English.
                let about = if self.determiner_before(i) { "roughly" } else { "about" };
                Some((end, format!("{about} {words}"), after))
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
            // "$5", and "£ 4" as old books write it.
            _ if currency(c).is_some()
                && (self.number_at(i + 1) || self.get(i + 1) == Some(' ') && self.digit(i + 2)) =>
            {
                self.money(i)
            }
            _ if self.digit(i) || c == '.' && self.digit(i + 1) && free && (i == 0 || self.t[i - 1] != '.') => {
                self.number(i, free)
            }
            _ => None,
        }
    }

    /// Digits, or `.` and a digit, at `i`.
    fn number_at(&self, i: usize) -> bool {
        self.digit(i) || self.get(i) == Some('.') && self.digit(i + 1)
    }

    /// A word multiplier after a number ending at `i` (" million", " hundred thousand"), or a
    /// glued `k`, `m`, `b` or `bn` (and `t`, `mn` and `tn`, and a hyphen before the word, for
    /// money: "$4.8bn", "$5-million"): its words and end. A glued letter may be followed by a slash
    /// only for money, before a rate ("$1B/year").
    fn multiplier(&self, i: usize, money: bool) -> Option<(String, usize)> {
        let mut words = Vec::new();
        let mut j = i;
        while self.get(j) == Some(' ') || money && j == i && self.get(j) == Some('-') {
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
        let slash_ok = |end: usize| self.get(end) != Some('/') || money && self.rate(end, true).is_some();
        for (letters, word, any) in [("bn", "billion", true), ("mn", "million", false), ("tn", "trillion", false)] {
            if (any || money) && self.at_ignore_case(i, letters) && self.free_at(i + 2) && slash_ok(i + 2) {
                return Some((word.to_owned(), i + 2));
            }
        }
        let word = match self.get(i)?.to_ascii_lowercase() {
            'k' => "thousand",
            'm' => "million",
            'b' => "billion",
            't' if money => "trillion",
            _ => return None,
        };
        (self.free_at(i + 1) && slash_ok(i + 1)).then(|| (word.to_owned(), i + 1))
    }

    /// A rate after an amount ending at `i`: a slash and a word. After money any word ("$20/month"
    /// as "a month", "$5/seat" as "per seat"; spaces around the slash allowed); after other
    /// numbers and their units, a unit of time ("GB/s" as "per second").
    fn rate(&self, i: usize, money: bool) -> Option<(String, usize)> {
        let spaced = money && self.at(i, " / ");
        let start = if spaced {
            i + 3
        } else if self.get(i) == Some('/') {
            i + 1
        } else {
            return None;
        };
        let mut end = start;
        while self.is(end, |c| c.is_ascii_alphabetic()) {
            end += 1;
        }
        if end == start || end - start > 12 || !self.free_at(end) || self.get(end) == Some('/') {
            return None;
        }
        let word: String = self.t[start..end].iter().collect();
        let time = match time_unit(&word) {
            Some(time) => time,
            None if money && end - start >= 2 => return Some((format!("per {word}"), end)),
            None => return None,
        };
        let words = match (money, time) {
            (true, "hour") => "an hour".to_owned(),
            (true, _) => format!("a {time}"),
            (false, _) => format!("per {time}"),
        };
        Some((words, end))
    }

    /// Whether an amount ending at `end`, whose currency symbol is at `symbol`, names the kind of
    /// thing after it ("a $5 bill", "a $600M round", "$20-fee"): followed by a hyphen and a word,
    /// or by a lowercase word that isn't one that follows amounts ([`LINKING_WORDS`],
    /// [`AFTER_AMOUNT`], a word ending in "-ed" or "-ly"); after "a" or "an", by any word but a
    /// linking word, capitalised too ("a $10 Casio"). Returns the end of the amount, past a
    /// hyphen, and whether the word after it is the currency's own name ("$315 million dollar
    /// deal"), which then isn't read twice.
    fn attributive(&self, symbol: usize, end: usize, currency: &Currency) -> Option<(usize, bool)> {
        if self.get(end) == Some('-') && self.is(end + 1, |c| c.is_ascii_alphabetic()) {
            return Some((end + 1, false));
        }
        if self.get(end) != Some(' ') {
            return None;
        }
        let start = end + 1;
        let mut word_end = start;
        while self.is(word_end, |c| c.is_ascii_alphabetic()) {
            word_end += 1;
        }
        if word_end == start || self.is(word_end, char::is_alphanumeric) {
            return None;
        }
        let word: String = self.t[start..word_end].iter().collect::<String>().to_ascii_lowercase();
        if word == currency.one || word == currency.many {
            return Some((end, true));
        }
        if LINKING_WORDS.contains(&word.as_str()) {
            return None;
        }
        let article = self.article_before(if symbol > 0 && self.t[symbol - 1] == '~' { symbol - 1 } else { symbol });
        let capital = self.t[start].is_uppercase();
        let after_amount = AFTER_AMOUNT.contains(&word.as_str())
            || word.len() >= 4 && (word.ends_with("ed") || word.ends_with("ly"))
            || word.len() >= 6 && word.ends_with("wide")
            || word.len() >= 5 && word.ends_with("ing");
        (article || !capital && !after_amount).then_some((end, false))
    }

    /// Whether the word before `k` (one space before it) is "a" or "an".
    fn article_before(&self, k: usize) -> bool {
        self.word_before_is(k, &["a", "an"])
    }

    /// Whether the word before `k` is a determiner ("a", "the", "their"), after which "about" doesn't
    /// read as English ("the about one billion dollars").
    fn determiner_before(&self, k: usize) -> bool {
        self.word_before_is(k, &["a", "an", "the", "its", "their", "his", "her", "our", "your", "my", "this", "that"])
    }

    /// Whether the word before `k` (one space before it) is one of `words` (lowercase), in any case.
    fn word_before_is(&self, k: usize, words: &[&str]) -> bool {
        words.iter().any(|a| {
            let n = a.len();
            k > n
                && self.t[k - 1] == ' '
                && self.t[k - 1 - n..k - 1].iter().map(char::to_ascii_lowercase).eq(a.chars())
                && (k == n + 1 || !self.t[k - n - 2].is_alphanumeric())
        })
    }

    /// Money: a currency symbol and a number, and whether a range's "to" ends it ("$5-10").
    fn money(&self, i: usize) -> Option<(usize, String, bool)> {
        let currency = currency(self.t[i])?;
        let start = if self.get(i + 1) == Some(' ') { i + 2 } else { i + 1 };
        let (body, end) = self.body(start)?;
        if self.get(end) == Some('.') && self.digit(end + 1) {
            // "$1.2.3" isn't money: the symbol and the version are read apart.
            return None;
        }
        let whole = |b: &Body| b.frac.as_deref().is_none_or(|f| f.bytes().all(|d| d == b'0'));
        // The amount: its number in words when it is in whole units or has a multiplier (so it may
        // name the kind of thing after it), where it ends, and its plural reading.
        let (number, end, plural) = if let Some((multiplier, m_end)) = self.multiplier(end, true) {
            (Some(format!("{} {multiplier}", body.words_plain())), m_end, None)
        } else if self.get(end) == Some('+')
            && let Some((multiplier, m_end)) = self.multiplier(end + 1, true)
        {
            // "$100+M": as written, "one hundred plus million dollars".
            (Some(format!("{} plus {multiplier}", body.words_plain())), m_end, None)
        } else if let Some((to, to_end)) = self.money_range(i, end) {
            // A range ("$5-10", "$5-$10", "$1-2 million"): the unit once, at the end.
            match self.multiplier(to_end, true) {
                Some((multiplier, m_end)) => {
                    (Some(format!("{} to {} {multiplier}", body.words_plain(), to.words_plain())), m_end, None)
                }
                None if whole(&body) && whole(&to) => {
                    (Some(format!("{} to {}", cardinal(&body.int), cardinal(&to.int))), to_end, None)
                }
                // With cents, each end in full: "one dollar and fifty cents to two dollars".
                None => {
                    let words = format!("{} to {}", money_words(&currency, &body), money_words(&currency, &to));
                    (None, to_end, Some(words))
                }
            }
        } else if whole(&body) {
            (Some(cardinal(&body.int)), end, Some(money_words(&currency, &body)))
        } else {
            (None, end, Some(money_words(&currency, &body)))
        };
        let plural = plural.unwrap_or_else(|| format!("{} {}", number.as_deref().expect("a number"), currency.many));
        if let Some((rate, r_end)) = self.rate(end, true) {
            // "a $20/month plan": "a twenty dollar a month plan".
            if let Some(number) = &number
                && let Some((a_end, false)) = self.attributive(i, r_end, &currency)
                && a_end == r_end
            {
                return Some((r_end, format!("{number} {} {rate}", currency.one), false));
            }
            return Some((r_end, format!("{plural} {rate}"), false));
        }
        if let Some(number) = number
            && let Some((a_end, named)) = self.attributive(i, end, &currency)
        {
            let words = if named { number } else { format!("{number} {}", currency.one) };
            return Some((a_end, words, false));
        }
        Some((end, plural, false))
    }

    /// The second end of a money range whose symbol is at `i` and whose first number ends at
    /// `end`: "-10" or "-$10".
    fn money_range(&self, i: usize, end: usize) -> Option<(Body, usize)> {
        if self.get(end) != Some('-') || self.minus[end] {
            return None;
        }
        let second = if self.get(end + 1) == Some(self.t[i]) { end + 2 } else { end + 1 };
        if !self.number_at(second) {
            return None;
        }
        let (to, to_end) = self.body(second)?;
        (!(self.get(to_end) == Some('.') && self.digit(to_end + 1))).then_some((to, to_end))
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
        if !self.in_name(i)
            && let Some((multiplier, end)) = self.multiplier(end, false)
        {
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
                if after == end + 2 && self.possessive(i, &body) {
                    return Some((after, format!("{}'s", body.words()), false));
                }
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
            // "22 GB/s": "twenty-two gigabytes per second".
            let (rate, end) = self.rate(end, false).map_or((String::new(), end), |(r, e)| (format!(" {r}"), e));
            return Some((end, format!("{} {words}{rate}", body.words()), false));
        }
        // A word and a unit of time after a slash: "43 tok/s", "24g/day".
        let word_start = if self.get(end) == Some(' ') { end + 1 } else { end };
        let mut word_end = word_start;
        while word_end - word_start < 8 && self.is(word_end, |c| c.is_ascii_alphabetic()) {
            word_end += 1;
        }
        let word: String = self.t[word_start..word_end].iter().collect();
        let time: String =
            self.t[(word_end + 1).min(self.t.len())..].iter().take_while(|c| c.is_ascii_alphabetic()).collect();
        if word_end > word_start
            && rate_word(&word, &time)
            && let Some((rate, r_end)) = self.rate(word_end, false)
        {
            return Some((r_end, format!("{} {word} {rate}", body.words()), false));
        }
        if integer {
            // "1½", "1 ½", "1 3/4"; not the number after a slash ("3/4 1/2" isn't "three slash four and a half").
            let gap = usize::from(self.get(end) == Some(' '));
            let after_slash = i > 0 && matches!(self.t[i - 1], '/' | '⁄');
            if !after_slash && let Some((n, d)) = self.get(end + gap).and_then(vulgar_fraction) {
                return Some((end + gap + 1, format!("{} {}", body.words(), and_fraction(n, d)), false));
            }
            if gap == 1
                && !after_slash
                && let Some((n, d, f_end)) = self.slash_fraction(end + 1)
            {
                return Some((f_end, format!("{} {}", body.words(), and_fraction(n, d)), false));
            }
            if !body.commas
                && let Some((words, d_end)) = self.slash_date(i)
            {
                return Some((d_end, words, false));
            }
            // "24/7", as it is said.
            if body.int == "24"
                && self.at(end, "/7")
                && self.free_at(end + 2)
                && self.get(end + 2) != Some('/')
                && (i == 0 || self.t[i - 1] != '/')
            {
                return Some((end + 2, "twenty-four seven".to_owned(), false));
            }
            // A fraction on its own only before "of" ("3/4 of the vote", not "part 1/3 of the
            // thread") or with the fraction slash ("3⁄4"); "4/5 stars", "part 1/3" or "7/10" may be
            // a rating, a part or a score, so the slash is read as written (in `numbers`).
            let label = i > 0
                && self.t[i - 1] == ' '
                && NUMBERED_PARTS.contains(&self.word_ending_at(i - 1).to_lowercase().as_str());
            if !body.commas
                && let Some((n, d, f_end)) = self.slash_fraction(i)
                && (self.get(end) == Some('⁄')
                    || !label && self.get(f_end) == Some(' ') && self.word_is(f_end + 1, "of"))
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
            if !body.commas && self.get(end) == Some('-') && self.digit(end + 1) && !self.minus[end] {
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
        // A range or a score: "10-20", "1.5-2", "21-14" (in arithmetic, "minus" below).
        // Not in a name ("Llama-3.1-8B").
        if self.get(end) == Some('-') && self.number_at(end + 1) && !self.minus[end] && !self.in_name(i) {
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
        // Powers with a caret: "2^64", "10^-3"; not a LaTeX limit ("\int_0^1", "x^{10}").
        if self.get(end) == Some('^') && (i == 0 || !matches!(self.t[i - 1], '_' | '^' | '{' | '\\')) {
            let negative = matches!(self.get(end + 1), Some('-' | '−'));
            let start = end + 1 + usize::from(negative);
            let e_end = self.digits_end(start);
            if e_end > start && self.free_at(e_end) && !(self.get(e_end) == Some('.') && self.digit(e_end + 1)) {
                let exponent = self.ascii(start..e_end);
                let power = if negative { negative_power(&exponent) } else { power(&exponent) };
                return Some((e_end, format!("{}{power}", body.words_plain()), false));
            }
        }
        // Superscript powers: "10²", "10⁶", "10⁻³".
        let negative = self.get(end) == Some('⁻') && self.get(end + 1).and_then(superscript_digit).is_some();
        let mut sup_end = end + usize::from(negative);
        let mut exponent = String::new();
        while let Some(d) = self.get(sup_end).and_then(superscript_digit) {
            exponent.push(char::from(b'0' + d));
            sup_end += 1;
        }
        if !exponent.is_empty() {
            let power = if negative { negative_power(&exponent) } else { power(&exponent) };
            return Some((sup_end, format!("{}{power}", body.words()), false));
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
/// number inside a word next to a single letter ("A4", "B2B", "H2O") is joined to it by a hyphen
/// ("A-four"), which misaki reads as one word, so the letter stays a letter name rather than the
/// article "a"; next to longer runs of letters ("BA123", "MP3") it is a word of its own.
/// `after` is the text after the number.
fn push_words(out: &mut String, words: &str, after: &[char], in_word: bool) {
    let single = |run: &mut dyn Iterator<Item = &char>| run.take_while(|c| c.is_alphabetic()).take(2).count() == 1;
    let before: Vec<char> = out.chars().rev().take(2).collect();
    if let Some(&c) = before.first().filter(|&&c| is_alnum(c)) {
        let hyphen = in_word && glues(c) && single(&mut before.iter());
        out.push(if hyphen { '-' } else { ' ' });
    }
    out.push_str(words);
    if let Some(&c) = after.first().filter(|&&c| is_alnum(c)) {
        let hyphen = in_word && glues(c) && c.is_alphabetic() && single(&mut after.iter());
        out.push(if hyphen { '-' } else { ' ' });
    }
}

/// The hyphens (and minus signs) that are minus signs in arithmetic: in a stretch of numbers,
/// one-letter variables, operators, brackets and spaces that has an `=` (or `<`, `>`, `≤`, `≥`,
/// `≠`), or at least two operators with spaces around them ("5 - 3 + 2"). Elsewhere a hyphen
/// between numbers is a range or score ("2-3" is "two to three"), and a spaced one a dash.
fn arithmetic_minus(t: &[char]) -> Vec<bool> {
    let mut minus = vec![false; t.len()];
    // "->" is an arrow, read "to" by the symbol pass: neither a minus nor "greater than".
    let arrow = |k: usize| t[k] == '-' && t.get(k + 1) == Some(&'>') || t[k] == '>' && k > 0 && t[k - 1] == '-';
    let operator = |k: usize| matches!(t[k], '+' | '-' | '−' | '×' | '*' | '/' | '÷') && !arrow(k);
    let relation = |k: usize| matches!(t[k], '=' | '<' | '>' | '≤' | '≥' | '≠') && !arrow(k);
    let letter = |i: usize| t[i].is_ascii_alphabetic();
    let in_expression = |i: usize| {
        let c = t[i];
        digit_value(c).is_some()
            || operator(i)
            || relation(i)
            || matches!(c, ' ' | '.' | '(' | ')')
            // A one-letter variable, not part of a word.
            || (letter(i) && (i == 0 || !letter(i - 1)) && (i + 1 == t.len() || !letter(i + 1)))
    };
    let mut i = 0;
    while i < t.len() {
        if !in_expression(i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < t.len() && in_expression(i) {
            i += 1;
        }
        let span = start..i;
        let spaced = |k: usize| k > 0 && k + 1 < t.len() && t[k - 1] == ' ' && t[k + 1] == ' ';
        let has_digit = span.clone().any(|k| digit_value(t[k]).is_some());
        let has_relation = span.clone().any(relation);
        let spaced_operators = span.clone().filter(|&k| operator(k) && spaced(k)).count();
        if has_digit && (has_relation || spaced_operators >= 2) {
            for k in span {
                minus[k] = matches!(t[k], '-' | '−') && !arrow(k);
            }
        }
    }
    // A bracket around math raised to a power ("(x-1)³", "[n-2]²"): its hyphens are minus signs
    // too, as the symbol pass reads the superscript as a power.
    for close in 0..t.len() {
        let open = match t[close] {
            ')' => '(',
            ']' => '[',
            _ => continue,
        };
        if !t.get(close + 1).is_some_and(|&c| superscript_digit(c).is_some()) {
            continue;
        }
        // At most 64 characters back, so runs of "(x)²)²…" stay linear.
        let from = close.saturating_sub(64);
        let Some(start) = t[from..close].iter().rposition(|&c| c == open).map(|k| from + k) else { continue };
        let inside = start + 1..close;
        if !inside.is_empty() && inside.clone().all(&in_expression) {
            for k in inside {
                minus[k] |= matches!(t[k], '-' | '−') && !arrow(k);
            }
        }
    }
    minus
}

/// The number pass: every number in the text as words.
pub fn numbers(text: &str) -> String {
    if !text.chars().any(|c| digit_value(c).is_some()) {
        return text.to_owned();
    }
    let t: Vec<char> = text.chars().collect();
    let scanner = Scanner { t: &t, minus: arithmetic_minus(&t) };
    let mut out = String::with_capacity(text.len() * 2);
    let mut i = 0;
    let mut plain_at = usize::MAX;
    // Where the last number read ended.
    let mut last_end = usize::MAX;
    while i < t.len() {
        let c = t[i];
        // A hyphen between two amounts with units, "70%-30%", "10°-15°", "40k-80k", "$5M-$10M": a
        // range, as between bare numbers (which the number reader reads itself).
        let unit_before = last_end == i || last_end.checked_add(1) == Some(i) && t[last_end] == '%';
        if c == '-'
            && !scanner.minus[i]
            && unit_before
            && !scanner.digit(i - 1)
            && (scanner.number_at(i + 1)
                || currency(t.get(i + 1).copied().unwrap_or(' ')).is_some() && scanner.number_at(i + 2))
        {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str("to ");
            plain_at = i + 1;
            i += 1;
            continue;
        }
        if matches!(c, '-' | '−') && scanner.minus[i] {
            // A minus sign in arithmetic ("3-2=1", "x - 3 = 7", "5 - 3 + 2").
            push_words(&mut out, "minus", &t[i + 1..], false);
            if scanner.digit(i + 1) || t.get(i + 1) == Some(&'.') {
                plain_at = i + 1;
            }
            i += 1;
            continue;
        }
        let candidate = scanner.digit(i) || matches!(c, '~' | '±' | '-' | '−' | '.' | '#') || currency(c).is_some();
        if candidate && let Some((end, words, operator)) = scanner.expression(i, plain_at == i) {
            let in_word = scanner.digit(i) && plain_at != i && !scanner.free_before(i);
            if in_word && words.starts_with("point ") && out.ends_with('.') {
                // The decimal point of a number read before this pass ("seven oh nine.30").
                out.pop();
                out.push(' ');
            }
            push_words(&mut out, &words, &t[end..], in_word);
            last_end = end;
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
        // A citation glued to the text before it, "text.[3]" or "word[12]": silent.
        if c == '[' && i > 0 && !t[i - 1].is_whitespace() && scanner.digit(i + 1) {
            let end = scanner.digits_end(i + 1);
            if t.get(end) == Some(&']') {
                i = end + 1;
                continue;
            }
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
        // A slash between numbers that no reading took ("4/5 stars", "80/20"): as written.
        if c == '/' && last_end == i && scanner.digit(i + 1) {
            push_words(&mut out, NUMBER_SLASH, &t[i + 1..], false);
            plain_at = i + 1;
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
            ("2000100", "two million, one hundred"),
            ("1234567", "one million, two hundred and thirty-four thousand, five hundred and sixty-seven"),
            ("1000000", "one million"),
            ("2000500", "two million, five hundred"),
            ("10000001", "ten million and one"),
            ("3450000", "three million, four hundred and fifty thousand"),
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
            ("The £ 4 a week", "The four pounds a week"),
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
            ("5 Ω, 4.7 kΩ and 1 MΩ", "five ohms, four point seven kilohms and one megohm"),
            ("5 \u{2126}, 4.7 k\u{2126} and 1 M\u{2126}", "five ohms, four point seven kilohms and one megohm"),
            ("version 2.0.1", "version two point zero point one"),
            ("v1.2", "v-one point two"),
            ("BA123 and A4", "BA one twenty-three and A-four"),
            ("item #1, MP3s and H2O", "item number one, MP three-s and H-two-O"),
            ("B2B", "B-to-B"),
            ("1080p", "ten eighty p"),
            ("3:1", "three to one"),
            ("1/2 cup, 3/4 of the vote and 1⁄2", "one slash two cup, three quarters of the vote and one half"),
            (
                "1/3 of it, part 1/3 of the thread, 4/5 stars",
                "one third of it, part one slash three of the thread, four slash five stars",
            ),
            ("3/4 and 9/11", "three slash four and nine slash eleven"),
            ("3/4 1/2 cup, 4/5 ½", "three slash four one slash two cup, four slash five ½"),
            ("1.5-2.5 kg", "one point five to two point five kilograms"),
            ("$5-10", "five to ten dollars"),
            (
                "$5-$10, £5-10, €5-10 and $1-2 million",
                "five to ten dollars, five to ten pounds, five to ten euros and one to two million dollars",
            ),
            (
                "$1.50-2.00 and $1.50-$2.25",
                "one dollar and fifty cents to two dollars and one dollar and fifty cents to two dollars and twenty-five cents",
            ),
            ("$5 to $10, 5-10% and 5-10 km", "five dollars to ten dollars, five to ten% and five to ten kilometers"),
            ("the '80s", "the eighties"),
            ("50¢ and 1¢", "fifty cents and one cent"),
            ("the 1500s", "the fifteen hundreds"),
            (
                "1,00,000 rupees and 12,34,56,789",
                "one hundred thousand rupees and one hundred and twenty-three million, four hundred and fifty-six thousand, seven hundred and eighty-nine",
            ),
            ("0,000 and 10m/s and 5m", "zero, zero zero zero and ten meters per second and five million"),
            ("Wait...5 more", "Wait...five more"),
            (
                "It sold for $3,450,000 last year.",
                "It sold for three million, four hundred and fifty thousand dollars last year.",
            ),
            (
                "$1,250,000.75 and 3,000,000 km. Then 12,345,000.",
                "one million, two hundred and fifty thousand dollars and seventy-five cents and three million kilometers. Then twelve million, three hundred and forty-five thousand.",
            ),
            ("(x-1)³ and [n-2]² but (x-1) and 2-3", "(x minus one)³ and [n minus two]² but (x-one) and two to three"),
            ("It ended.[3] Then word[12] and see [4].", "It ended. Then word and see [four]."),
            (
                "3-2=1, x - 3 = 7, 5 - 3 + 2 and 2−3",
                "three minus two=one, x minus three = seven, five minus three + two and two−three",
            ),
            (
                "2-3, 2 - 3, x-3, -3, 21-14 and 1990-1995",
                "two to three, two - three, x-three, minus three, twenty-one to fourteen and nineteen ninety to nineteen ninety-five",
            ),
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
    fn money_before_what_it_buys_is_singular() {
        for (text, expected) in [
            ("I found a $5 bill.", "I found a five dollar bill."),
            ("They closed a $600M round.", "They closed a six hundred million dollar round."),
            ("the $2 bill and $100k iPhones", "the two dollar bill and one hundred thousand dollar iPhones"),
            ("$20-fee transactions", "twenty dollar fee transactions"),
            ("a 10-year, $5-million contract", "a ten-year, five million dollar contract"),
            ("a $315 million dollar deal", "a three hundred and fifteen million dollar deal"),
            ("a $5-10 fee", "a five to ten dollar fee"),
            (
                "a ~$100k Rolex and ~$10 Casio",
                "a roughly one hundred thousand dollar Rolex and about ten dollars Casio",
            ),
            ("a $300 daily allowance", "a three hundred dollar daily allowance"),
            // Plural: before what links the amount to the rest, a verb, an adverb, a capital or
            // nothing, and with cents.
            (
                "$600M of funding, $5 each and $5 or $10",
                "six hundred million dollars of funding, five dollars each and five dollars or ten dollars",
            ),
            (
                "raised $600M. It cost $5 now, $5 raised and £220 standing",
                "raised six hundred million dollars. It cost five dollars now, five dollars raised and two hundred and twenty pounds standing",
            ),
            ("$7,000 Mom paid and $300 monthly", "seven thousand dollars Mom paid and three hundred dollars monthly"),
            ("the regular $1.89 price", "the regular one dollar and eighty-nine cents price"),
        ] {
            assert_eq!(n(text), expected, "{text}");
        }
    }

    #[test]
    fn money_suffixes_and_rates() {
        for (text, expected) in [
            (
                "$4.8bn and £1.1BN and $3mn",
                "four point eight billion dollars and one point one billion pounds and three million dollars",
            ),
            ("$30M to $100+M rounds", "thirty million dollars to one hundred plus million dollar rounds"),
            (
                "costs $20/month, $1B/year and $3-4B/year",
                "costs twenty dollars a month, one billion dollars a year and three to four billion dollars a year",
            ),
            (
                "$3.80/hour or $185,000 / year",
                "three dollars and eighty cents an hour or one hundred and eighty-five thousand dollars a year",
            ),
            ("$5/seat and a $20/month plan", "five dollars per seat and a twenty dollar a month plan"),
            ("~$10", "about ten dollars"),
        ] {
            assert_eq!(n(text), expected, "{text}");
        }
    }

    #[test]
    fn units_rates_dates_and_powers() {
        for (text, expected) in [
            (
                "22 GB/s, 43.39 tok/s, 24g/day, 5 hours/day",
                "twenty-two gigabytes per second, forty-three point three nine tok per second, twenty-four g per day, five hours per day",
            ),
            ("5 km/s and 3 and/or 4", "five kilometers per second and three and/or four"),
            (
                "on 5/31/2000, 31/7/2020 and on 4/25/24",
                "on May thirty-first, two thousand, July thirty-first, twenty twenty and on April twenty-fifth, twenty-four",
            ),
            (
                "13/13/2020 and 1/2/3/2020",
                "thirteen slash thirteen slash twenty twenty and one slash two slash three slash twenty twenty",
            ),
            (
                "the 10/20/30 rule, 5/10/15 minutes",
                "the ten slash twenty slash thirty rule, five slash ten slash fifteen minutes",
            ),
            (
                "Date: 4/25/24, Mon, 4/25/24",
                "Date: April twenty-fifth, twenty-four, Mon, April twenty-fifth, twenty-four",
            ),
            (
                "open 24/7, 24/7/365",
                "open twenty-four seven, twenty-four slash seven slash three hundred and sixty-five",
            ),
            ("2^64, 2^16 and 10^-3", "two to the sixty-fourth, two to the sixteenth and ten to the minus three"),
            ("10⁻³ and 10⁶", "ten to the minus three and ten to the sixth"),
        ] {
            assert_eq!(n(text), expected, "{text}");
        }
    }

    #[test]
    fn hyphens_names_and_possessives() {
        for (text, expected) in [
            (
                "70%-30%, 10°-15°, 40k-80k and $5M-$10M",
                "seventy% to thirty%, ten degrees to fifteen degrees, forty thousand to eighty thousand and five million dollars to ten million dollars",
            ),
            ("the 1990s-2000s and 3rd-4th", "the nineteen nineties to two thousands and third to fourth"),
            (
                "Llama-3.1-8B, Gemma-3-12B-IT and H-1B",
                "Llama-three point one-eight B, Gemma-three-twelve B-IT and H-one B",
            ),
            (
                "the 2-3B models, pre-2020-21 and Mid-1990s",
                "the two to three billion models, pre-twenty twenty to twenty-one and Mid-nineteen nineties",
            ),
            ("Opus 4's score, GPT-4's and I-645's", "Opus four's score, GPT-four's and I-six hundred and forty-five's"),
            (
                "the 1990's, Londoners 1890's style, 1's and 0's",
                "the nineteen nineties, Londoners eighteen nineties style, ones and zeros",
            ),
            ("from 2048->512 runs", "from twenty forty-eight->five hundred and twelve runs"),
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
