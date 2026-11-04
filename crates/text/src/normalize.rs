//! Kokoro-FastAPI's text normalizers (`normalization/base.py` and `english.py`).

use crate::english_data::{MONEY_UNITS, SYMBOL_REPLACEMENTS, VALID_TLDS, VALID_UNITS, unit};
use crate::inflect::{self, OutOfRange, int_to_words, number_to_words};
use crate::py::{self, NotFinite, float_repr, int_str, parse_float};
use crate::pyre::{Captures, Pattern, compile, compile_i};
use crate::scan;
use std::ops::Range;
use std::sync::LazyLock;

/// `NormalizationOptions` from the request schema, with its defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub normalize: bool,
    pub unit_normalization: bool,
    pub url_normalization: bool,
    pub email_normalization: bool,
    pub optional_pluralization_normalization: bool,
    pub phone_normalization: bool,
    pub caps_normalization: bool,
    pub replace_remaining_symbols: bool,
    pub remove_emoji: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            normalize: true,
            unit_normalization: false,
            url_normalization: true,
            email_normalization: true,
            optional_pluralization_normalization: true,
            phone_normalization: true,
            caps_normalization: true,
            replace_remaining_symbols: true,
            remove_emoji: false,
        }
    }
}

/// The normalizer registered for a pipeline language code, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Normalizer {
    English,
}

impl Normalizer {
    pub fn for_lang(lang_code: &str) -> Option<Self> {
        matches!(lang_code, "a" | "b" | "en-us" | "en-gb").then_some(Self::English)
    }

    pub fn normalize(self, text: &str, options: &Options) -> String {
        match self {
            Self::English => english(text, options),
        }
    }
}

struct Patterns {
    multi_space: Pattern,
    non_space_whitespace: Pattern,
    ellipsis_word: Pattern,
    spaces: Pattern,
    caps_word: Pattern,
    long_caps: Pattern,
    contraction: Pattern,
    contraction_bare: Pattern,
    url: Pattern,
    url_scheme: Pattern,
    url_www: Pattern,
    url_port: Pattern,
    phone: Pattern,
    non_digit: Pattern,
    doctor: Pattern,
    mister: Pattern,
    miss: Pattern,
    missus: Pattern,
    etc: Pattern,
    yeah: Pattern,
    digit_s: Pattern,
    comma_run: Pattern,
    thousands: Pattern,
    range_dash: Pattern,
    version: Pattern,
    money: Pattern,
    number: Pattern,
    dashes: Pattern,
    dotted_acronym: Pattern,
    acronym_dot: Pattern,
    /// Unit keys in `UNIT_PATTERN`'s alternation order.
    units: Vec<&'static str>,
}

const NOT_GLUED_BEFORE: &str = r"(?<![^\W_])(?<![^\W_][.,])";
const NOT_GLUED_AFTER: &str = r"(?![^\W_]|\.\d|,\d)";
const NUMBER_BODY: &str = r"(\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?|\.\d+)";
const NUMBER_MULTIPLIER: &str = r"((?: hundred| thousand| (?:[bm]|tr|quadr)illion|k|m|b)*)";
const MONEY_MULTIPLIER: &str = r"((?: hundred| thousand| (?:[bm]|tr|quadr)illion|k|m|b|t)*)";

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| {
    let mut units: Vec<&str> = VALID_UNITS.iter().map(|(k, _)| *k).collect();
    units.sort_unstable_by(|a, b| b.cmp(a));
    let money_symbols: String = MONEY_UNITS.iter().map(|(c, _, _)| *c).collect();
    Patterns {
        units,
        multi_space: compile(r"\s{2,}"),
        non_space_whitespace: compile(r"[^\S ]"),
        ellipsis_word: compile(r"…(?=\w)"),
        spaces: compile(r"  +"),
        caps_word: compile(r"(?<![0-9A-Za-z])[A-Z]{4,}(?![0-9A-Za-z])"),
        long_caps: compile(r"(?<![0-9A-Za-z])[A-Z]{6,}(?![0-9A-Za-z])"),
        contraction: compile_i(r"\b(how|what|where|who|when|why|there|these|those)['’]re\b"),
        contraction_bare: compile_i(r"\b(how|what|where|when|why|there|these|those|you|they)re\b"),
        url: compile_i(&format!(
            r"(?<![a-zA-Z0-9.-])(?:https?://)?(?:www\.)?(localhost|[a-zA-Z0-9.-]{{1,253}}(\.(?:{}))+|[0-9]{{1,3}}\.[0-9]{{1,3}}\.[0-9]{{1,3}}\.[0-9]{{1,3}})(:[0-9]+)?([/?][^\s]*)?",
            VALID_TLDS.join("|")
        )),
        url_scheme: compile_i(r"^https?://"),
        url_www: compile_i(r"^www\."),
        url_port: compile(r":(\d+)(?=/|$)"),
        phone: compile(r"(?:(\+?\d{1,2})[ .-]?)?(\(?\d{3}\)?)[\s.-](\d{3})[\s.-](\d{4})"),
        non_digit: compile(r"\D"),
        doctor: compile(r"\bD[Rr]\.(?= [A-Z])"),
        mister: compile(r"\b(?:Mr\.|MR\.(?= [A-Z]))"),
        miss: compile(r"\b(?:Ms\.|MS\.(?= [A-Z]))"),
        missus: compile(r"\b(?:Mrs\.|MRS\.(?= [A-Z]))"),
        etc: compile(r"\betc\.(?! [A-Z])"),
        yeah: compile_i(r"\b(y)eah?\b"),
        digit_s: compile(r"(?<=\d)S"),
        comma_run: compile(r"(?<!\d)\d+(?:,\d+)+"),
        thousands: compile(r"^\d{1,3}(?:,\d{3})+$"),
        range_dash: compile(r"(?<=\d)-(?=\d)"),
        version: compile(r"(?<!\d)\d+(?:\.\d+){2,}"),
        money: compile_i(&format!(r"(-?)([{money_symbols}]){NUMBER_BODY}{MONEY_MULTIPLIER}{NOT_GLUED_AFTER}")),
        number: compile_i(&format!(r"{NOT_GLUED_BEFORE}(-?){NUMBER_BODY}{NUMBER_MULTIPLIER}{NOT_GLUED_AFTER}")),
        dashes: compile(r" ?-{2,} ?"),
        dotted_acronym: compile(r"(?:[A-Za-z]\.){2,12} [a-z]"),
        acronym_dot: compile_i(r"(?<=[A-Z])\.(?=[A-Z])"),
    }
});

type Pass = fn(&Patterns, &str) -> String;

/// `EnglishNormalizer.pass_order`: the passes the options enable, in order.
fn pass_order(options: &Options) -> Vec<(&'static str, Pass)> {
    let all: [(&str, bool, Pass); 17] = [
        ("caps_words", options.caps_normalization, caps_words),
        ("contractions", true, |p, t| {
            let t = p.contraction.sub(t, |c| format!("{} are", &c[1]));
            p.contraction_bare.sub(&t, |c| format!("{} are", &c[1]))
        }),
        ("emails", options.email_normalization, |_, t| {
            replace_spans(t, scan::emails(t).into_iter().map(|r| (r.clone(), email(&t[r]))))
        }),
        ("urls", options.url_normalization, |p, t| p.url.sub(t, |c| url(p, &c[0]))),
        ("units", options.unit_normalization, |p, t| {
            replace_spans(
                t,
                scan::units(t, &p.units).into_iter().map(|m| (m.range, units(p, &t[m.number], &t[m.unit]))),
            )
        }),
        ("optional_plurals", options.optional_pluralization_normalization, |_, t| t.replace("(s)", "s")),
        ("phone_numbers", options.phone_normalization, |p, t| p.phone.sub(t, |c| phone(p, c))),
        ("quotes", true, |_, t| quotes(t)),
        ("cjk_punctuation", true, |_, t| cjk_punctuation(t)),
        ("times", true, |_, t| {
            replace_spans(t, scan::times(t).into_iter().map(|m| (m.range, time(&t[m.time], m.half.map(|h| &t[h])))))
        }),
        ("whitespace", true, whitespace),
        ("abbreviations", true, abbreviations),
        ("common_words", true, |p, t| p.yeah.sub(t, |c| format!("{}e'a", &c[1]))),
        ("numbers", true, numbers),
        ("symbols", options.replace_remaining_symbols, symbols),
        ("acronyms", true, acronyms),
        ("multiple_spaces", true, |p, t| p.multi_space.sub_str(t, " ")),
    ];
    all.into_iter().filter(|(_, on, _)| *on).map(|(name, _, pass)| (name, pass)).collect()
}

/// `EnglishNormalizer().normalize(text, options)`.
pub fn english(text: &str, options: &Options) -> String {
    let p = &*PATTERNS;
    pass_order(options).into_iter().fold(text.to_owned(), |text, (_, pass)| pass(p, &text))
}

/// Each enabled pass of [`english`] on its own, by name, for timing tests.
#[doc(hidden)]
pub fn english_passes(options: &Options) -> Vec<(&'static str, impl Fn(&str) -> String + use<>)> {
    pass_order(options).into_iter().map(|(name, pass)| (name, move |text: &str| pass(&PATTERNS, text))).collect()
}

fn quotes(text: &str) -> String {
    text.replace(['\u{2018}', '\u{2019}'], "'")
        .replace('\u{ab}', "\u{201c}")
        .replace('\u{bb}', "\u{201d}")
        .replace(['\u{201c}', '\u{201d}'], "\"")
}

const CJK_PUNCTUATION: [(char, &str); 8] =
    [('、', ", "), ('。', ". "), ('！', "! "), ('，', ", "), ('：', ": "), ('；', "; "), ('？', "? "), ('–', "- ")];

fn cjk_punctuation(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match CJK_PUNCTUATION.iter().find(|(from, _)| *from == c) {
            Some((_, to)) => out.push_str(to),
            None => out.push(c),
        }
    }
    out
}

fn whitespace(p: &Patterns, text: &str) -> String {
    let text = p.non_space_whitespace.sub_str(text, " ");
    let text = p.ellipsis_word.sub_str(&text, "… ");
    p.spaces.sub_str(&text, " ")
}

fn caps_words(p: &Patterns, text: &str) -> String {
    let lower = |c: &Captures| {
        let word = &c[0];
        if word.chars().all(|c| "IVXLCDM".contains(c)) { word.to_owned() } else { word.to_lowercase() }
    };
    let runs = scan::caps_runs(text);
    let text = replace_spans(text, runs.into_iter().map(|r| (r.clone(), p.caps_word.sub(&text[r], lower))));
    p.long_caps.sub(&text, lower)
}

/// The text with each (sorted, disjoint) span replaced.
fn replace_spans(text: &str, spans: impl IntoIterator<Item = (Range<usize>, String)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, replacement) in spans {
        out.push_str(&text[last..range.start]);
        out.push_str(&replacement);
        last = range.end;
    }
    out.push_str(&text[last..]);
    out
}

fn email(email: &str) -> String {
    let (user, domain) = email.split_once('@').expect("the pattern has one @");
    format!("{user} at {}", domain.replace('.', " dot "))
}

fn url(p: &Patterns, url: &str) -> String {
    let url = py::strip(url);
    let url = p.url_scheme.sub(url, |c| if c[0].contains("https") { "https " } else { "http " }.to_owned());
    let url = p.url_www.sub_str(&url, "www ");
    let url = p.url_port.sub(&url, |c| format!(" colon {}", &c[1]));
    let (domain, path) = url.split_once('/').unwrap_or((&url, ""));
    let domain = domain.replace('.', " dot ");
    let url = if path.is_empty() { domain } else { format!("{domain} slash {path}") };
    let url = url
        .replace('-', " dash ")
        .replace('_', " underscore ")
        .replace('?', " question-mark ")
        .replace('=', " equals ")
        .replace('&', " ampersand ")
        .replace('%', " percent ")
        .replace(':', " colon ")
        .replace('/', " slash ");
    url.split(py::is_space).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
}

fn units(p: &Patterns, number: &str, unit_string: &str) -> String {
    let unit_string = py::strip(unit_string);
    let Some(expansion) = unit(&unit_string.to_lowercase()) else {
        return join_chars(unit_string);
    };
    let mut words: Vec<String> = expansion.split(' ').map(str::to_owned).collect();
    if words[0].ends_with("bit") {
        let b_case = unit_string.chars().nth(1.min(unit_string.chars().count() - 1));
        if b_case == Some('B') {
            words[0] = format!("{}byte", &words[0][..words[0].len() - 3]);
        }
    }
    let mut number = py::strip(number).to_owned();
    if number.contains(',') && !p.thousands.is_match(&number) {
        number = number.replace(',', ".");
    }
    words[0] = inflect::no(&words[0], &number);
    words.join(" ")
}

/// `" ".join(s)` on a string: its characters, space-separated.
fn join_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for (i, c) in s.chars().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn phone(p: &Patterns, c: &Captures) -> String {
    c.groups()
        .flatten()
        .filter(|g| !g.is_empty())
        .map(|g| {
            let digits = p.non_digit.sub_str(g, "");
            digits.chars().map(|d| number_to_words(&d.to_string()).expect("one digit")).collect::<Vec<_>>().join(" ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Python's `int()` on ASCII digits with optional surrounding spaces.
fn small_int(s: &str) -> u32 {
    py::strip(s).parse().expect("pattern guarantees digits")
}

fn time(time: &str, half: Option<&str>) -> String {
    let parts: Vec<&str> = time.split(':').collect();
    let minute = small_int(parts[1]);
    let words = |s: &str| number_to_words(py::strip(s)).expect("at most two digits");
    let mut out = vec![words(parts[0])];
    let minute_words = words(parts[1]);
    if (1..10).contains(&minute) {
        out.push(format!("oh {minute_words}"));
    } else if minute >= 10 {
        out.push(minute_words);
    }
    let half = half.filter(|h| !h.is_empty());
    if parts.len() > 2 {
        let seconds = small_int(parts[2]);
        out.push(format!("and {} {}", words(parts[2]), inflect::plural("second", &seconds.to_string())));
    } else if half.is_none() && minute == 0 {
        out.push("o'clock".into());
    }
    if let Some(half) = half {
        out.push(half.to_owned());
    }
    out.join(" ")
}

fn abbreviations(p: &Patterns, text: &str) -> String {
    let text = p.doctor.sub_str(text, "Doctor");
    let text = p.mister.sub_str(&text, "Mister");
    let text = p.miss.sub_str(&text, "Miss");
    let text = p.missus.sub_str(&text, "Mrs");
    p.etc.sub_str(&text, "etc")
}

fn numbers(p: &Patterns, text: &str) -> String {
    let text = p.digit_s.sub_str(text, " S");
    let text =
        p.comma_run.sub(&text, |c| if p.thousands.is_match(&c[0]) { c[0].to_owned() } else { c[0].replace(',', "") });
    let text = p.range_dash.sub_str(&text, " to ");
    let text = p.version.sub(&text, |c| unchanged_on_error(c, versions));
    let text = p.money.sub(&text, |c| unchanged_on_error(c, money));
    p.number.sub(&text, |c| unchanged_on_error(c, number_words))
}

/// An error inflect, `int()` or `round()` raises; the reference leaves the matched text as written.
struct Unspeakable;

impl From<OutOfRange> for Unspeakable {
    fn from(_: OutOfRange) -> Self {
        Self
    }
}

impl From<NotFinite> for Unspeakable {
    fn from(_: NotFinite) -> Self {
        Self
    }
}

fn unchanged_on_error(c: &Captures, handler: fn(&Captures) -> Result<String, Unspeakable>) -> String {
    handler(c).unwrap_or_else(|_| c[0].to_owned())
}

/// Python's `str(int(s))` for a run of decimal digits; past 4300 digits `int()` itself refuses.
fn int_digits(s: &str) -> Result<String, Unspeakable> {
    if s.chars().count() > 4300 {
        return Err(Unspeakable);
    }
    let digits: String =
        s.chars().filter_map(py::digit_value).map(|d| char::from(b'0' + d as u8)).skip_while(|&c| c == '0').collect();
    Ok(if digits.is_empty() { "0".into() } else { digits })
}

fn versions(c: &Captures) -> Result<String, Unspeakable> {
    let words = c[0]
        .split('.')
        .map(|part| Ok(number_to_words(&int_digits(part)?)?))
        .collect::<Result<Vec<_>, Unspeakable>>()?;
    Ok(words.join(" point "))
}

fn translate_multiplier(multiplier: &str) -> String {
    match multiplier.to_lowercase().as_str() {
        "k" => "thousand".into(),
        "m" => "million".into(),
        "b" => "billion".into(),
        "t" => "trillion".into(),
        _ => py::strip(multiplier).to_owned(),
    }
}

/// `conditional_int(number)`: an int when within 1e-5 of one, else the float.
#[derive(Clone, Copy)]
enum Number {
    Int(f64),
    Float(f64),
}

impl Number {
    fn new(x: f64) -> Result<Self, NotFinite> {
        if !x.is_finite() {
            return Err(NotFinite);
        }
        Ok(if (x.round() - x).abs() < 0.00001 { Self::Int(x.round()) } else { Self::Float(x) })
    }

    fn str(self) -> Result<Option<String>, NotFinite> {
        match self {
            Self::Int(x) => int_str(x),
            Self::Float(x) => Ok(Some(float_repr(x))),
        }
    }

    fn words(self) -> Result<String, Unspeakable> {
        Ok(int_to_words(self.str()?.as_deref())?)
    }
}

fn signed(sign: &str, body: &str) -> f64 {
    let number = parse_float(&body.replace(',', ""));
    if sign == "-" { -number } else { number }
}

fn money(c: &Captures) -> Result<String, Unspeakable> {
    let symbol = c[2].chars().next().unwrap();
    let &(_, bill, coin) =
        MONEY_UNITS.iter().find(|(s, _, _)| *s == symbol).expect("pattern only matches known symbols");
    let number = signed(&c[1], &c[3]);
    let mut multiplier = translate_multiplier(&c[4]);
    if !multiplier.is_empty() {
        multiplier.insert(0, ' ');
    }
    let bills = inflect::plural(bill, &float_repr(number));
    if number % 1.0 == 0.0 || !multiplier.is_empty() {
        let whole = Number::new(number)?.words()?;
        return Ok(format!("{whole}{multiplier} {bills}"));
    }
    let whole = int_to_words(int_str(number.floor())?.as_deref())?;
    let repr = float_repr(number);
    let decimals = repr.rsplit('.').next().unwrap();
    let fraction: u64 = format!("{decimals:0<2}").parse().map_err(|_| Unspeakable)?;
    let coins = inflect::plural(coin, &fraction.to_string());
    Ok(format!("{whole} {bills} and {} {coins}", number_to_words(&fraction.to_string())?))
}

fn number_words(c: &Captures) -> Result<String, Unspeakable> {
    let raw = &c[2];
    let number = Number::new(signed(&c[1], raw))?;
    let mut multiplier = translate_multiplier(&c[3]);
    if !multiplier.is_empty() {
        multiplier.insert(0, ' ');
    } else if let Number::Int(x) = number
        && !raw.contains(',')
        && x > 1500.0
        && x < 10000.0
        && x % 1000.0 > 9.0
    {
        let digits = int_str(x)?.expect("four digits");
        let (century, year) = digits.split_at(2);
        return Ok(format!("{} {}", number_to_words(century)?, number_to_words(year)?));
    }
    Ok(format!("{}{multiplier}", number.words()?))
}

fn symbols(p: &Patterns, text: &str) -> String {
    let mut text = p.dashes.sub_str(text, " — ");
    for (symbol, replacement) in SYMBOL_REPLACEMENTS {
        text = text.replace(symbol, replacement);
    }
    text
}

fn acronyms(p: &Patterns, text: &str) -> String {
    let text = p.dotted_acronym.sub(text, |c| c[0].replace('.', "-"));
    p.acronym_dot.sub_str(&text, "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::tests::random_texts;

    /// Every pattern's fast paths (native conditions, relaxed matches) against the plain
    /// backtracker, on random text made of the pieces the patterns react to.
    #[test]
    fn patterns_match_the_backtracker() {
        let p = &*PATTERNS;
        let patterns = [
            &p.multi_space,
            &p.non_space_whitespace,
            &p.ellipsis_word,
            &p.spaces,
            &p.caps_word,
            &p.long_caps,
            &p.contraction,
            &p.contraction_bare,
            &p.url,
            &p.url_scheme,
            &p.url_www,
            &p.url_port,
            &p.phone,
            &p.non_digit,
            &p.doctor,
            &p.mister,
            &p.miss,
            &p.missus,
            &p.etc,
            &p.yeah,
            &p.digit_s,
            &p.comma_run,
            &p.thousands,
            &p.range_dash,
            &p.version,
            &p.money,
            &p.number,
            &p.dashes,
            &p.dotted_acronym,
            &p.acronym_dot,
        ];
        let alphabet = [
            "a",
            "A",
            "1",
            "٣",
            " ",
            ".",
            ",",
            ":",
            "-",
            "/",
            "?",
            "$",
            "€",
            "_",
            "'",
            "’",
            "…",
            "é",
            "\u{301}",
            "²",
            "\n",
            "\t",
            "\u{1c}",
            "ı",
            "İ",
            "ſ",
            "K",
            "k",
            "m",
            "b",
            "t",
            " thousand",
            " million",
            "how",
            "they",
            "re",
            "'re",
            "yea",
            "h",
            "Dr",
            "DR",
            "Mr",
            "MR",
            "Ms",
            "Mrs",
            "MRS",
            "etc",
            " T",
            "S",
            "www.",
            "http://",
            "https://",
            "x.com",
            ".io",
            ":80",
            "555",
            "(555)",
            "1234",
            "+1",
            "AAAA",
            "ABCDEF",
            "IV",
            "--",
        ];
        let references: Vec<_> = patterns.iter().map(|p| p.reference()).collect();
        for text in random_texts(&alphabet, 20000, 5) {
            for (i, pattern) in patterns.iter().enumerate() {
                assert_eq!(pattern.matches(&text), references[i](&text), "pattern {i} on {text:?}");
            }
        }
    }
}
