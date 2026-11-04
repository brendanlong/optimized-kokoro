//! The one place symbols are read: every symbol, emoji and typographic mark in English text
//! becomes a spoken name, plain punctuation, or nothing, before spaCy and misaki see it
//! (`docs/divergences.md`, "Symbols").
//!
//! In the reference, symbols the normalizer didn't replace went to misaki: spaCy tagged some as
//! punctuation, which misaki turned into a stray punctuation phoneme (`∑` as `”`, `⅚` as `)`) or
//! nothing (`✓`), and the rest went to espeak, which named some (`∑` as "sum sign"), read others
//! as code points ("letter 2107") and dropped the rest. Here the ASCII symbols keep the
//! reference's words; marks, math, arrows, currency and letterlike symbols have a name or are
//! silent ([`curated`]); emoji have Unicode CLDR's text-to-speech names (`data/emoji.tsv`); and
//! anything else in the symbol blocks is silent, so only letters, digits' words and the
//! punctuation Kokoro reads are left.

use crate::numbers::{cardinal, enclosed_number, power, subscript_digit, superscript_digit, vulgar_fraction_words};
use crate::py::{is_alnum, is_space};
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// How a symbol is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reading {
    /// Words, kept apart from what's around them.
    Say(&'static str),
    /// Nothing (decorative marks).
    Silent,
    /// Plain punctuation, in place.
    Mark(&'static str),
}

use Reading::{Mark, Say, Silent};

/// The symbols with a reading of their own, outside ASCII and the emoji table.
fn curated(c: char) -> Option<Reading> {
    Some(match c {
        // Latin-1
        '¢' => Say("cents"),
        '£' => Say("pounds"),
        '¥' => Say("yen"),
        '¤' | '¦' | '¨' | '¯' | '´' | '·' | '¸' => Silent,
        '§' => Say("section"),
        '©' => Say("copyright"),
        '¬' => Say("not"),
        '®' => Say("registered"),
        '°' => Say("degrees"),
        '±' => Say("plus or minus"),
        '¶' => Say("paragraph"),
        '×' => Say("times"),
        '÷' => Say("divided by"),
        // General punctuation
        '\u{2010}' | '\u{2011}' | '\u{2012}' => Mark("-"),
        '\u{2015}' | '\u{2E3A}' | '\u{2E3B}' => Mark("—"),
        '\u{201A}' | '\u{201B}' | '‹' | '›' | '❛' | '❜' => Mark("'"),
        '\u{201E}' | '\u{201F}' | '❝' | '❞' => Mark("\""),
        '․' => Mark("."),
        '‥' | '⋯' => Mark("…"),
        '‰' => Say("per mille"),
        '‱' => Say("per ten thousand"),
        '′' => Say("prime"),
        '″' => Say("double prime"),
        '‴' => Say("triple prime"),
        '‼' | '⁉' => Mark("!"),
        '‽' | '⁇' | '⁈' => Mark("?"),
        '⁄' | '∕' => Say("slash"),
        '⁅' | '⟨' | '⟦' | '❨' | '❪' | '❬' | '❮' | '❰' | '❲' | '❴' => Mark("("),
        '⁆' | '⟩' | '⟧' | '❩' | '❫' | '❭' | '❯' | '❱' | '❳' | '❵' => Mark(")"),
        // Currency, on its own (with a number it's read as money)
        '₡' => Say("colones"),
        '₣' => Say("francs"),
        '₤' | '₺' => Say("lira"),
        '₦' => Say("naira"),
        '₧' => Say("pesetas"),
        '₨' | '₹' => Say("rupees"),
        '₩' => Say("won"),
        '₪' => Say("shekels"),
        '₫' => Say("dong"),
        '€' => Say("euros"),
        '₭' => Say("kip"),
        '₮' => Say("tugrik"),
        '₯' => Say("drachmas"),
        '₱' => Say("pesos"),
        '₲' => Say("guarani"),
        '₴' => Say("hryvnia"),
        '₵' => Say("cedi"),
        '₸' => Say("tenge"),
        '₼' => Say("manat"),
        '₽' => Say("rubles"),
        '₾' => Say("lari"),
        '₿' => Say("bitcoin"),
        '\u{20C0}' => Say("som"),
        // Letterlike
        '℃' => Say("degrees Celsius"),
        '℉' => Say("degrees Fahrenheit"),
        '℅' => Say("care of"),
        '№' => Say("number"),
        '℞' => Say("prescription"),
        '℠' => Say("service mark"),
        '™' => Say("trademark"),
        '℡' => Say("telephone"),
        '\u{2126}' => Say("ohm"),
        'Ⅎ' => Silent,
        '℮' => Say("estimated"),
        'ℏ' => Say("h bar"),
        'ℇ' => Say("Euler's constant"),
        'ℵ' => Say("aleph"),
        // Arrows
        '→' | '⟶' | '➡' | '⇾' => Say("to"),
        '←' | '⟵' | '⬅' => Say("left arrow"),
        '↑' | '⬆' => Say("up arrow"),
        '↓' | '⬇' => Say("down arrow"),
        '↔' | '⟷' => Say("left right arrow"),
        '↕' => Say("up down arrow"),
        '⇒' | '⟹' => Say("implies"),
        '⇐' | '⟸' => Say("is implied by"),
        '⇔' | '⟺' => Say("if and only if"),
        '↦' | '⟼' => Say("maps to"),
        '⇧' => Say("shift"),
        // Music
        '♯' => Say("sharp"),
        '♭' => Say("flat"),
        '♮' => Say("natural"),
        // Mathematics
        '∀' => Say("for all"),
        '∂' => Say("partial"),
        '∃' => Say("there exists"),
        '∄' => Say("there does not exist"),
        '∅' => Say("empty set"),
        '∆' => Say("delta"),
        '∇' => Say("nabla"),
        '∈' | '∊' => Say("in"),
        '∉' => Say("not in"),
        '∋' | '∍' => Say("contains"),
        '∌' => Say("does not contain"),
        '∏' => Say("product"),
        '∐' => Say("coproduct"),
        '∑' | '⅀' => Say("sum"),
        '−' => Say("minus"),
        '∓' => Say("minus or plus"),
        '∔' => Say("plus"),
        '∖' => Say("set minus"),
        '∘' => Say("composed with"),
        '√' => Say("square root of"),
        '∛' => Say("cube root of"),
        '∜' => Say("fourth root of"),
        '∝' => Say("proportional to"),
        '∞' => Say("infinity"),
        '∟' => Say("right angle"),
        '∠' | '∡' => Say("angle"),
        '∣' => Say("divides"),
        '∤' => Say("does not divide"),
        '∥' => Say("parallel to"),
        '∦' => Say("not parallel to"),
        '∧' => Say("and"),
        '∨' => Say("or"),
        '∩' => Say("intersection"),
        '∪' => Say("union"),
        '∫' => Say("integral"),
        '∬' => Say("double integral"),
        '∭' => Say("triple integral"),
        '∮' => Say("contour integral"),
        '∴' => Say("therefore"),
        '∵' => Say("because"),
        '∶' => Say("to"),
        '∼' => Say("similar to"),
        '≃' => Say("asymptotically equal to"),
        '≅' => Say("congruent to"),
        '≈' => Say("approximately"),
        '≉' => Say("not approximately"),
        '≠' => Say("not equal to"),
        '≡' => Say("identical to"),
        '≢' => Say("not identical to"),
        '≤' | '≦' | '⩽' => Say("less than or equal to"),
        '≥' | '≧' | '⩾' => Say("greater than or equal to"),
        '≪' => Say("much less than"),
        '≫' => Say("much greater than"),
        '≮' => Say("not less than"),
        '≯' => Say("not greater than"),
        '⊂' => Say("subset of"),
        '⊃' => Say("superset of"),
        '⊄' => Say("not a subset of"),
        '⊅' => Say("not a superset of"),
        '⊆' => Say("subset of or equal to"),
        '⊇' => Say("superset of or equal to"),
        '⊕' => Say("circled plus"),
        '⊗' => Say("circled times"),
        '⊥' => Say("perpendicular to"),
        '⊢' => Say("proves"),
        '⊨' => Say("entails"),
        '⋅' => Say("times"),
        '⌀' => Say("diameter"),
        // Keys
        '⌘' => Say("command"),
        '⌥' => Say("option"),
        '⌃' => Say("control"),
        '⌫' => Say("backspace"),
        '⌦' => Say("delete"),
        '⏎' => Say("return"),
        '⎋' => Say("escape"),
        // Checks and crosses
        '✓' => Say("check mark"),
        '✗' | '✘' => Say("cross mark"),
        // Decorative: bullets, stars and shapes used as bullets, which the emoji table names
        '▪' | '▫' | '▶' | '◀' | '◻' | '◼' | '◽' | '◾' | '⬛' | '⬜' | '★' | '☆' => Silent,
        _ => return None,
    })
}

/// Whether `before` ends in a bracket around math: numbers (already words here), one-letter
/// variables and operators, as in "(a+b)²", "(x-1)³" or "[2n]²". "(see Smith)²" is a footnote.
fn bracketed_math(before: &[char]) -> bool {
    let (close, open) = match before.last() {
        Some(')') => (')', '('),
        Some(']') => (']', '['),
        _ => return false,
    };
    // At most 64 characters back, so runs of "(x)²)²…" stay linear.
    let from = before.len().saturating_sub(65);
    let Some(start) = before[from..before.len() - 1].iter().rposition(|&c| c == open).map(|k| from + k) else {
        return false;
    };
    let inside = &before[start + 1..before.len() - 1];
    if inside.is_empty() || inside.contains(&close) {
        return false;
    }
    let text: String = inside.iter().collect();
    let symbols_ok = inside.iter().all(|&c| c.is_alphanumeric() || c.is_whitespace() || "+-−×*/÷=^.,".contains(c));
    symbols_ok
        && text
            .split(|c: char| !c.is_alphabetic())
            .filter(|w| !w.is_empty())
            .all(|w| w.chars().count() == 1 || crate::numbers::is_number_word(w))
}

/// Units a superscript after which is a power ("km²", "s²").
const POWER_UNITS: &[&str] = &["m", "cm", "mm", "km", "nm", "µm", "μm", "ft", "in", "yd", "mi", "s", "ms", "g", "kg"];

/// The blocks whose characters this pass reads: punctuation and symbols outside ASCII. Letters
/// and numbers in them (`ℂ`, `Ⅻ`) are left alone.
fn in_symbol_block(c: char) -> bool {
    matches!(c as u32,
        0x00A1..=0x00BF | 0x00D7 | 0x00F7
        | 0x2010..=0x2027 | 0x2030..=0x205E
        | 0x2070..=0x20CF
        | 0x2100..=0x2BFF
        | 0x2E00..=0x2E7F
        | 0x1F000..=0x1FAFF)
}

/// Characters that only change how the character before them looks: emoji and text variation
/// selectors, skin tones, the keycap mark, and the tag characters of subdivision flags.
fn is_modifier(c: char) -> bool {
    matches!(c as u32, 0xFE0E | 0xFE0F | 0x1F3FB..=0x1F3FF | 0x20E3 | 0xE0020..=0xE007F)
}

/// Punctuation Kokoro reads (or that misaki handles as punctuation): kept as it is.
fn is_kept_punctuation(c: char) -> bool {
    matches!(c, '—' | '–' | '…' | '‘' | '’' | '“' | '”' | '«' | '»' | '¡' | '¿')
}

struct Emoji {
    names: HashMap<&'static str, &'static str>,
    first: HashSet<char>,
    longest: usize,
}

static EMOJI: LazyLock<Emoji> = LazyLock::new(|| {
    let mut names = HashMap::new();
    let mut first = HashSet::new();
    let mut longest = 0;
    for line in include_str!("../data/emoji.tsv").lines() {
        let (key, name) = line.split_once('\t').expect("sequence, tab, name");
        names.insert(key, name);
        first.extend(key.chars().next());
        longest = longest.max(key.chars().count());
    }
    Emoji { names, first, longest }
});

/// The longest emoji sequence at `i`, ignoring variation selectors and skin tones: its name, end
/// (after any modifiers that follow it), and whether it is more than one character.
fn emoji_at(t: &[char], i: usize) -> Option<(&'static str, usize, bool)> {
    let emoji = &*EMOJI;
    if !emoji.first.contains(&t[i]) {
        return None;
    }
    let mut key = String::new();
    let mut count = 0;
    let mut best = None;
    let mut j = i;
    while j < t.len() && count < emoji.longest {
        let c = t[j];
        j += 1;
        if matches!(c as u32, 0xFE0E | 0xFE0F | 0x1F3FB..=0x1F3FF) {
            continue;
        }
        key.push(c);
        count += 1;
        if let Some(&name) = emoji.names.get(key.as_str()) {
            best = Some((name, j, count > 1));
        }
    }
    best.map(|(name, mut end, sequence)| {
        while end < t.len() && is_modifier(t[end]) {
            end += 1;
        }
        (name, end, sequence)
    })
}

/// Output being built, with the spacing rules for spoken names and silent marks.
struct Out {
    text: String,
}

impl Out {
    fn ends_open(&self) -> bool {
        self.text.chars().next_back().is_none_or(|c| is_space(c) || "([{\"“‘'".contains(c))
    }

    /// Words kept apart from what's around them.
    fn say(&mut self, words: &str, next: Option<char>) {
        if !self.ends_open() {
            self.text.push(' ');
        }
        self.text.push_str(words);
        if next.is_some_and(|c| !is_space(c) && !".,;:!?)]}\"”’'…".contains(c)) {
            self.text.push(' ');
        }
    }

    /// Nothing, but a space if the mark stood between two words.
    fn silent(&mut self, next: Option<char>) {
        if !self.ends_open() && next.is_some_and(is_alnum) {
            self.text.push(' ');
        }
    }
}

/// Whether the text around `i` (one space allowed on each side) is letters or digits: `a < b`.
fn between_words(t: &[char], i: usize) -> bool {
    let before = match i.checked_sub(1).map(|j| t[j]) {
        Some(' ') => i.checked_sub(2).map(|j| t[j]),
        c => c,
    };
    let after = match t.get(i + 1).copied() {
        Some(' ') => t.get(i + 2).copied(),
        c => c,
    };
    before.is_some_and(is_alnum) && after.is_some_and(is_alnum)
}

/// The symbol pass.
pub fn symbols(text: &str) -> String {
    let t: Vec<char> = text.chars().collect();
    let mut out = Out { text: String::with_capacity(text.len() + 16) };
    let mut i = 0;
    while i < t.len() {
        let c = t[i];
        let next = t.get(i + 1).copied();
        // A bracketed footnote mark, "[²]": silent with its brackets.
        if c == '[' && t.get(i + 1).copied().and_then(superscript_digit).is_some() {
            let mut end = i + 1;
            while t.get(end).copied().and_then(superscript_digit).is_some() {
                end += 1;
            }
            if t.get(end) == Some(&']') {
                out.silent(t.get(end + 1).copied());
                i = end + 1;
                continue;
            }
        }
        if c.is_ascii() {
            // The reference's replacements, with their spacing.
            let replacement = match c {
                '~' | '^' | '*' | '_' | '|' | '\\' => " ",
                '@' => " at ",
                '$' => " dollar ",
                '%' => " percent ",
                '&' => " and ",
                '/' => " slash ",
                '=' => " equals ",
                '+' => " plus ",
                '#' => {
                    let single_letter =
                        i >= 1 && ('A'..='G').contains(&t[i - 1]) && (i < 2 || !t[i - 2].is_alphanumeric());
                    if single_letter && !next.is_some_and(is_alnum) {
                        " sharp "
                    } else if next.is_some_and(char::is_alphabetic) {
                        " hashtag "
                    } else {
                        " number "
                    }
                }
                '-' if next == Some('>') => {
                    out.say("to", t.get(i + 2).copied());
                    i += 2;
                    continue;
                }
                '<' if between_words(&t, i) => " less than ",
                '>' if between_words(&t, i) => " greater than ",
                _ => {
                    out.text.push(c);
                    i += 1;
                    continue;
                }
            };
            out.text.push_str(replacement);
            i += 1;
            continue;
        }
        if c == '\u{200D}' {
            // A zero-width joiner left between emoji (a sequence the table doesn't have) or
            // symbols goes; elsewhere (Indic scripts) it stays.
            let symbol = |c: Option<&char>| c.is_some_and(|&c| in_symbol_block(c) || is_modifier(c));
            if symbol(i.checked_sub(1).and_then(|j| t.get(j))) || symbol(t.get(i + 1)) {
                out.silent(next);
            } else {
                out.text.push(c);
            }
            i += 1;
            continue;
        }
        // Variation selectors and the like, and combining marks on a symbol (`😀\u{301}`), go
        // with the symbol.
        let combining = matches!(c as u32, 0x300..=0x36F | 0x1AB0..=0x1AFF | 0x20D0..=0x20FF);
        if is_modifier(c) || (combining && i > 0 && (in_symbol_block(t[i - 1]) || is_modifier(t[i - 1]))) {
            i += 1;
            continue;
        }
        if c == '·' && i > 0 && t[i - 1].is_alphabetic() && next.is_some_and(char::is_alphabetic) {
            // A middle dot inside a word or name ("col·lecció", "烏蘇拉·馮德萊恩") stays.
            out.text.push(c);
            i += 1;
            continue;
        }
        // A sequence (a flag, a family) is read whole; a single character's own reading comes
        // first ("‼" is "!" with or without its emoji variation selector).
        if let Some((name, end, _)) = emoji_at(&t, i).filter(|&(_, _, sequence)| sequence || curated(c).is_none()) {
            out.say(name, t.get(end).copied());
            i = end;
            continue;
        }
        if let Some(reading) = curated(c) {
            match reading {
                Say(words) => out.say(words, next),
                Silent => out.silent(next),
                Mark(mark) => out.text.push_str(mark),
            }
            i += 1;
            continue;
        }
        if let Some(words) = vulgar_fraction_words(c) {
            out.say(&words, next);
            i += 1;
            continue;
        }
        if let Some(n) = enclosed_number(c) {
            out.say(&cardinal(&n.to_string()), next);
            i += 1;
            continue;
        }
        if superscript_digit(c).is_some() || subscript_digit(c).is_some() {
            let sub = subscript_digit(c).is_some();
            let mut end = i;
            let mut run = String::new();
            while let Some(d) = t.get(end).and_then(|&c| if sub { subscript_digit(c) } else { superscript_digit(c) }) {
                run.push(char::from(b'0' + d));
                end += 1;
            }
            // A power after a one-letter variable ("x²"), a unit ("km²", "m/s²") or a word right
            // after `=` ("E=mc²"); a footnote mark after any other word ("Smith²") or after
            // punctuation ("end.²", "[²]"). Powers after numbers ("10²") were read by the number
            // pass.
            let word: String = {
                let mut start = i;
                while start > 0 && t[start - 1].is_alphabetic() {
                    start -= 1;
                }
                let after_equals = start > 0 && t[start - 1] == '=';
                let word: String = t[start..i].iter().collect();
                if after_equals { format!("={word}") } else { word }
            };
            let after_variable = word.chars().count() == 1
                || POWER_UNITS.contains(&word.as_str())
                || (word.starts_with('=') && word.len() > 1)
                || bracketed_math(&t[..i]);

            if sub {
                // "H₂O" as "H-two-O", one word, as the number pass joins numbers inside words.
                if out.text.chars().next_back().is_some_and(char::is_alphanumeric) {
                    out.text.push('-');
                    out.text.push_str(&cardinal(&run));
                    if t.get(end).is_some_and(|c| c.is_alphanumeric()) {
                        out.text.push('-');
                    }
                } else {
                    out.say(&cardinal(&run), t.get(end).copied());
                }
            } else if after_variable {
                // "x²"; after a longer word it's a footnote mark.
                out.text.push_str(&power(&run));
                if t.get(end).is_some_and(|&c| is_alnum(c)) {
                    out.text.push(' ');
                }
            } else {
                out.silent(t.get(end).copied());
            }
            i = end;
            continue;
        }
        if is_kept_punctuation(c) || !in_symbol_block(c) || c.is_alphabetic() || c.is_numeric() {
            out.text.push(c);
            i += 1;
            continue;
        }
        if matches!(c as u32, 0x2190..=0x21FF | 0x27F0..=0x27FF | 0x2900..=0x297F | 0x2B00..=0x2B11 | 0x2B30..=0x2B4F) {
            // Arrows without a reading of their own.
            out.say("arrow", next);
            i += 1;
            continue;
        }
        out.silent(next);
        i += 1;
    }
    out.text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_symbols() {
        for (text, expected) in [
            ("Math ∑ and ∫ signs.", "Math sum and integral signs."),
            ("Emoji 😀 and ✓ here.", "Emoji grinning face and check mark here."),
            ("Great 👍🏽!", "Great thumbs up!"),
            ("I ❤️ NY", "I red heart NY"),
            ("🇺🇸 and 🇬🇧", "United States flag and United Kingdom flag"),
            ("a ≠ b", "a not equal to b"),
            ("A → B", "A to B"),
            ("• one • two", "one two"),
            ("word† and ‡", "word and"),
            ("Wait‼", "Wait!"),
            ("x² and H₂O and CO₂.", "x squared and H-two-O and CO-two."),
            ("Smith² said, E=mc²", "Smith said, E equals mc squared"),
            ("m², x², km² and 3 m/s²", "m squared, x squared, km squared and 3 m slash s squared"),
            ("word², end.², Smith³ and [²] and x²³", "word, end., Smith and and x to the power of twenty-three"),
            ("(a+b)², (x-one)³ and [two n]²", "(a plus b) squared, (x-one) cubed and [two n] squared"),
            ("(x minus one)³", "(x minus one) cubed"),
            ("(see Smith)² and (end.)²", "(see Smith) and (end.)"),
            ("① and ⑳", "one and twenty"),
            ("½ cup", "one half cup"),
            ("C# and #hashtag and # 1", "C sharp and hashtag hashtag and number 1"),
            ("3 < 5", "3 less than 5"),
            ("<html>", "<html>"),
            ("a -> b", "a to b"),
            ("★★★☆", ""),
            ("ℇ and ℏ", "Euler's constant and h bar"),
            ("Ω and \u{2126}", "Ω and ohm"),
            ("↠ and ↩", "arrow and right arrow curving left"),
            ("■ ▲ ● ◊", ""),
            ("‼\u{FE0F}", "!"),
            ("in ₽", "in rubles"),
            ("👨‍👩‍👧 family", "family man woman girl family"),
            ("Хорошо\u{200D}", "Хорошо\u{200D}"),
        ] {
            // Runs of spaces are squeezed by a later pass.
            let squeezed = symbols(text).split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
            assert_eq!(squeezed, expected, "{text}");
        }
    }

    #[test]
    fn every_emoji_name_is_speakable() {
        for (key, name) in &EMOJI.names {
            assert!(!key.is_empty() && !name.is_empty());
            assert!(name.chars().all(|c| c.is_alphabetic() || " '-".contains(c)), "{key}: {name}");
        }
    }
}
