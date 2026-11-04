//! Kokoro-FastAPI's text normalizers (`normalization/base.py` and `english.py`), with numbers
//! and symbols read by this crate's own passes (`numbers.rs`, `symbols.rs`) instead of the
//! reference's (`docs/divergences.md`). The reference's unit option is gone: the number pass
//! reads units.

use crate::english_data::VALID_TLDS;
use crate::numbers::{self, cardinal};
use crate::py;
use crate::pyre::{Captures, Pattern, compile, compile_i};
use crate::{scan, symbols};
use std::ops::Range;
use std::sync::LazyLock;

/// `NormalizationOptions` from the request schema, with its defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub normalize: bool,
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
    dashes: Pattern,
    dotted_acronym: Pattern,
    acronym_dot: Pattern,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
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
    dashes: compile(r" ?-{2,} ?"),
    dotted_acronym: compile(r"(?:[A-Za-z]\.){2,12} [a-z]"),
    acronym_dot: compile_i(r"(?<=[A-Z])\.(?=[A-Z])"),
});

type Pass = fn(&Patterns, &str) -> String;

/// `EnglishNormalizer.pass_order`: the passes the options enable, in order.
fn pass_order(options: &Options) -> Vec<(&'static str, Pass)> {
    let all: [(&str, bool, Pass); 16] = [
        ("caps_words", options.caps_normalization, caps_words),
        ("contractions", true, |p, t| {
            let t = p.contraction.sub(t, |c| format!("{} are", &c[1]));
            p.contraction_bare.sub(&t, |c| format!("{} are", &c[1]))
        }),
        ("emails", options.email_normalization, |_, t| {
            replace_spans(t, scan::emails(t).into_iter().map(|r| (r.clone(), email(&t[r]))))
        }),
        ("urls", options.url_normalization, |p, t| p.url.sub(t, |c| url(p, &c[0]))),
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
        ("numbers", true, |_, t| numbers::numbers(t)),
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

/// The reference's CJK punctuation pass, except that an en dash between digits or inside a word
/// stays a hyphen ("1939–1945", "US–China"), for the number pass to read as "to" or misaki as a
/// compound.
fn cjk_punctuation(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut previous = None;
    while let Some(c) = chars.next() {
        let digit = |c: Option<char>| c.is_some_and(|c| py::digit_value(c).is_some());
        let money = |c: Option<char>| c.is_some_and(|c| "$£€¥₹₩₽₿₱₪₺".contains(c));
        // An en dash inside a word ("US–China") is a hyphen, so the words around it read as in
        // "US-China" (the reference's "- " left "US" alone and read it as the word "us").
        let glued = |c: Option<char>| c.is_some_and(|c| !py::is_space(c));
        let inside = glued(previous) && glued(chars.peek().copied()) && !previous.is_some_and(|c| c == '–');
        if c == '–' && (inside || digit(previous) && (digit(chars.peek().copied()) || money(chars.peek().copied()))) {
            out.push('-');
            previous = Some(c);
            continue;
        }
        previous = Some(c);
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

fn phone(p: &Patterns, c: &Captures) -> String {
    c.groups()
        .flatten()
        .filter(|g| !g.is_empty())
        .map(|g| numbers::digits(&p.non_digit.sub_str(g, "")))
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
    let words = |s: &str| cardinal(py::strip(s));
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
        out.push(format!("and {} {}", words(parts[2]), if seconds == 1 { "second" } else { "seconds" }));
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

fn symbols(p: &Patterns, text: &str) -> String {
    symbols::symbols(&p.dashes.sub_str(text, " — "))
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
    fn dashes() {
        let n = |t: &str| english(t, &Options::default());
        assert_eq!(n("The US–China talks"), "The US-China talks");
        assert_eq!(n("think of - a plan, good – very good"), "think of — a plan, good — very good");
        assert_eq!(n("1939–1945 and $5–$10"), "nineteen thirty-nine to nineteen forty-five and five to ten dollars");
    }

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
