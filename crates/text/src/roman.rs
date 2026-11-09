//! Roman numerals, read as numbers only where the text shows that they are numbers
//! (`docs/divergences.md`, "Roman numerals"). Anything else stays as written, so an acronym
//! ("an IV", "a CD player") or the pronoun "I" reads as before: a literal reading beats a wrong
//! one.
//!
//! A token of the capitals I V X L C D M that is a numeral in standard form (so not "IIII", "VV"
//! or "IC") is a number when:
//! 1. the word right before it, with only spaces between, is a capitalised word that isn't a
//!    function word ("The", "His", "Then", …), a contraction ("Aren't I?") or a possessive
//!    ("Vuitton's LV"): "World War II", "Super Bowl LV", "Louis XIV";
//! 2. it has two letters or more, starts a line, the text or a sentence, and is followed by "."
//!    or ")": a numbered heading or list item ("IV. Results"; "I. Introduction" can't be told
//!    from "V. Putin said"). Lines are joined into sentences before the normalizer runs
//!    (`split::join_lines`), so a sentence start stands in for a line start;
//! 3. it continues a list of numerals ("Super Bowls XXI and XXIV", "George II and III"; after a
//!    bare comma the list must then end or go on, so not "World War II, IV drug use").
//!
//! A number is read as a cardinal: "Saturn five", "World War two", and after a name too ("Henry
//! eight", "Elizabeth two"). An ordinal would need to know a name from another capitalised word,
//! and Brendan chose the plain reading (2026-11-09).
//!
//! Some tokens need more: a single letter is a numeral only if it is I or V (a single C, D, L, M
//! or X is far more often a letter: "Vitamin D", "Type C", "Malcolm X", "Generation X"); a
//! numeral with C, D or M needs four letters (two or three are acronyms: "CD", "DC", "MD", "CLI",
//! "MMM"; years are long: "MCMXCIV"); a single letter before ". Capital" may be an initial as
//! much as a sentence's end ("Vladimir V. Putin", "World War I. The"), so it stays a letter; and
//! "I", which is far more often the pronoun, must be followed by punctuation, the end of the
//! text, or "and", "or", "of" ("World War I,", "Elizabeth I of England", "Class I or Class II"),
//! not by a word ("In Paris I saw"); before `,` `;` `?` `!` the word before it mustn't start the
//! sentence ("Thinks I, I'll wait", "Dare I?"); and it doesn't close a list ("Charles III and
//! I.").

use crate::numbers::numbers;
use std::ops::Range;

/// Closed-class words: a capitalised one before a numeral is no evidence that it is a number
/// ("The IV", "His CD", "Then I").
const FUNCTION_WORDS: &[&str] = &[
    "a",
    "about",
    "above",
    "across",
    "after",
    "against",
    "all",
    "also",
    "although",
    "am",
    "among",
    "an",
    "and",
    "another",
    "any",
    "are",
    "as",
    "at",
    "be",
    "because",
    "been",
    "before",
    "behind",
    "being",
    "below",
    "beside",
    "between",
    "beyond",
    "both",
    "but",
    "by",
    "can",
    "could",
    "did",
    "do",
    "does",
    "during",
    "each",
    "either",
    "even",
    "every",
    "except",
    "few",
    "for",
    "from",
    "had",
    "has",
    "have",
    "he",
    "hence",
    "her",
    "here",
    "hers",
    "him",
    "his",
    "how",
    "however",
    "i",
    "if",
    "in",
    "including",
    "inside",
    "into",
    "is",
    "it",
    "its",
    "just",
    "least",
    "less",
    "like",
    "many",
    "may",
    "maybe",
    "me",
    "might",
    "mine",
    "more",
    "most",
    "much",
    "must",
    "my",
    "near",
    "neither",
    "no",
    "nor",
    "not",
    "now",
    "of",
    "off",
    "often",
    "oh",
    "on",
    "once",
    "only",
    "onto",
    "or",
    "other",
    "our",
    "ours",
    "out",
    "over",
    "per",
    "perhaps",
    "please",
    "same",
    "several",
    "shall",
    "she",
    "should",
    "since",
    "so",
    "some",
    "still",
    "such",
    "than",
    "that",
    "the",
    "their",
    "theirs",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "though",
    "through",
    "thus",
    "to",
    "too",
    "toward",
    "towards",
    "under",
    "unless",
    "unlike",
    "until",
    "up",
    "upon",
    "us",
    "very",
    "via",
    "was",
    "we",
    "well",
    "were",
    "what",
    "when",
    "whenever",
    "where",
    "whether",
    "which",
    "while",
    "who",
    "whom",
    "whose",
    "why",
    "will",
    "with",
    "within",
    "without",
    "would",
    "yes",
    "yet",
    "you",
    "your",
    "yours",
];

/// The value of a numeral in standard form (each power of ten as I, II, III, IV, V, …, IX), up to
/// 3999; `None` for anything else ("IIII", "VV", "IC", "XM").
fn value(token: &str) -> Option<u32> {
    // The longest numeral in standard form is MMMDCCCLXXXVIII.
    if token.len() > 15 {
        return None;
    }
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
    let values: Vec<u32> = token.chars().map(digit).collect::<Option<_>>()?;
    let mut n = 0;
    for (i, &v) in values.iter().enumerate() {
        if values.get(i + 1).is_some_and(|&next| next > v) { n -= v as i32 } else { n += v as i32 }
    }
    let n = u32::try_from(n).ok().filter(|n| (1..4000).contains(n))?;
    (to_roman(n) == token).then_some(n)
}

fn to_roman(mut n: u32) -> String {
    const PARTS: [(u32, &str); 13] = [
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
    ];
    let mut out = String::new();
    for (v, s) in PARTS {
        while n >= v {
            out.push_str(s);
            n -= v;
        }
    }
    out
}

/// A numeral that may be read at all, whatever the context.
fn readable(token: &str) -> bool {
    match token.len() {
        1 => matches!(token, "I" | "V"),
        n => n >= 4 || !token.contains(['C', 'D', 'M']),
    }
}

fn is_space(c: char) -> bool {
    c.is_whitespace() && c != '\n' && c != '\r'
}

/// The evidence of the word before (rule 1): whether the numeral after it is a number. For
/// "I" before a comma, semicolon, "?" or "!", a word that starts a sentence is capitalised for
/// that alone ("Thinks I, I'll wait", "Dare I?"; "BOOK I." and "Elizabeth I of England" still
/// read).
fn word_before(before: &str, pronoun: bool) -> bool {
    let trimmed = before.trim_end_matches(is_space);
    if trimmed.len() == before.len() {
        return false;
    }
    let start = trimmed
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_alphabetic() || matches!(c, '-' | '\'' | '’')))
        .map_or(0, |(i, c)| i + c.len_utf8());
    let word = trimmed[start..].trim_start_matches(['-', '\'', '’']);
    let capitalised = word.chars().next().is_some_and(char::is_uppercase);
    let possessive = ["'s", "’s", "'S", "’S"].iter().any(|s| word.ends_with(s));
    if !capitalised || possessive || !word.chars().next_back().is_some_and(char::is_alphabetic) {
        return false;
    }
    // A contraction is a verb or pronoun with its auxiliary ("Aren't I?", "I'm II").
    let contraction = word.contains(['\'', '’']);
    if contraction || FUNCTION_WORDS.binary_search(&word.to_lowercase().as_str()).is_ok() {
        return false;
    }
    // Here any period ends a sentence, even one after a letter ("Haven't I. Dare I?").
    let opening = trimmed[..start].trim_end_matches(['"', '“', '\'', '‘', '(', '[']);
    let sentence = opening.trim_end_matches(char::is_whitespace);
    !(pronoun && (sentence.is_empty() || sentence.ends_with(['.', '!', '?', ':']) && sentence.len() < opening.len()))
}

/// Whether `before` ends where a line, the text or a sentence starts (rule 2). A period after a
/// single letter is an initial's, not a sentence's end ("K. V. Puttappa").
fn starts_line(before: &str) -> bool {
    let trimmed = before.trim_end_matches(is_space);
    let initial = trimmed.strip_suffix('.').is_some_and(|rest| {
        let mut letters = rest.chars().rev();
        letters.next().is_some_and(char::is_alphabetic) && letters.next().is_none_or(|c| !c.is_alphanumeric())
    });
    trimmed.is_empty()
        || trimmed.ends_with(['\n', '\r'])
        || trimmed.len() < before.len() && trimmed.ends_with(['.', '!', '?', ':']) && !initial
}

/// Whether the text after a token lets it end there: an end of the text, a space, punctuation, a
/// possessive "'s" or a hyphen before a word ("II-era").
fn ends_word(after: &str) -> bool {
    let mut chars = after.chars();
    let next = chars.next();
    let then = chars.next();
    let free = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    match next {
        None => true,
        Some(c) if c.is_whitespace() || ",;!?)]}\"”—".contains(c) => true,
        // Not "III:16" or "V.9221".
        Some('.' | ':') => then.is_none_or(|c| c.is_whitespace() || "\"”)]'’".contains(c)),
        Some('\'' | '’') => free(then) || (then == Some('s') && free(chars.next())),
        Some('-' | '–') => then.is_some_and(char::is_alphabetic),
        _ => false,
    }
}

/// "I" is a numeral only before punctuation, the end, or "and", "or", "of" ("Elizabeth I of
/// England"); before any other word it is the pronoun.
fn i_may_be_numeral(after: &str) -> bool {
    let rest = after.trim_start_matches(is_space);
    if rest.len() == after.len() {
        return true;
    }
    let word: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '&').collect();
    let followed = rest[word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
    rest.is_empty() || matches!(word.as_str(), "and" | "or" | "of" | "&") && followed
}

/// Separators that continue a list of numerals (rule 3).
fn continues_list(between: &str) -> bool {
    matches!(between, ", " | " and " | " or " | " & " | ", and " | ", or " | " to ")
}

/// Whether a numeral after a comma in a list ends it or goes on with it: punctuation, the end, or
/// another separator ("XII, XIII and its"; not "World War II, IV drug use").
fn ends_list(after: &str) -> bool {
    let next = after.chars().next();
    next.is_none_or(|c| !c.is_alphanumeric() && !c.is_whitespace())
        || [", ", " and ", " or ", " & ", " to "].iter().any(|s| after.starts_with(s))
}

/// ". " and a capital: after a single letter, an initial as much as a sentence's end.
fn initial(after: &str) -> bool {
    after.strip_prefix(". ").and_then(|rest| rest.chars().next()).is_some_and(char::is_uppercase)
}

/// Whether the numeral is a number in its context. `previous` is where the last numeral read as
/// a number ended.
fn is_number(text: &str, start: usize, end: usize, previous: Option<usize>) -> bool {
    let (token, before, after) = (&text[start..end], &text[..start], &text[end..]);
    if value(token).is_none() || !readable(token) || !ends_word(after) {
        return false;
    }
    if token == "I" && !i_may_be_numeral(after) || token.len() == 1 && initial(after) {
        return false;
    }
    // A single letter isn't a heading: "I. Introduction" can't be told from "V. Putin said" or
    // "I. M. Pei".
    let heading = token.len() > 1 && starts_line(before) && after.starts_with(['.', ')']);
    // "I" closing a list is the pronoun as often as not ("Charles III and I.").
    let list = token != "I" || after.starts_with(|c: char| c.is_whitespace());
    previous.is_some_and(|previous_end| {
        let between = &text[previous_end..start];
        // After a bare comma the list must go on or end ("II, IV drug use" isn't one); a
        // conjunction already closes it ("World Wars I and II were").
        list && continues_list(between) && (between != ", " || ends_list(after))
    }) || word_before(before, token == "I" && after.starts_with([',', ';', '?', '!']))
        || heading
}

/// Each numeral the context shows to be a number, as its span in `text` and its words.
///
/// The normalizer decides on the text before its caps pass, which lowercases the words of an
/// all-caps phrase ("CHAPTER XXXV." as "chapter XXXV.") and would hide the capital that is the
/// evidence, and replaces the spans after it (the caps pass changes no lengths). Reading first
/// would hide the phrase from the caps pass instead ("BOOK one" would stay in capitals).
pub fn readings(text: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    if !text.contains(['I', 'V', 'X', 'L', 'C', 'D', 'M']) {
        return out;
    }
    let mut previous = None;
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        if !c.is_alphanumeric() {
            continue;
        }
        let mut end = start + c.len_utf8();
        while let Some(&(i, c)) = chars.peek().filter(|(_, c)| c.is_alphanumeric()) {
            end = i + c.len_utf8();
            chars.next();
        }
        if !is_number(text, start, end, previous) {
            continue;
        }
        let n = value(&text[start..end]).expect("is_number checks the value");
        out.push((start..end, numbers(&n.to_string())));
        previous = Some(end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roman_numerals(text: &str) -> String {
        crate::normalize::replace_spans(text, readings(text))
    }

    #[test]
    fn values_in_standard_form() {
        assert_eq!(value("XIV"), Some(14));
        assert_eq!(value("MCMXCIV"), Some(1994));
        assert_eq!(value("LV"), Some(55));
        assert_eq!(value("MMMCMXCIX"), Some(3999));
        for bad in ["IIII", "VV", "IC", "XM", "IIV", "VX", "LL", "DD", "MMMM", "IL", "XXL", ""] {
            assert_eq!(value(bad), None, "{bad}");
        }
        assert!(FUNCTION_WORDS.windows(2).all(|w| w[0] < w[1]), "FUNCTION_WORDS is sorted");
    }

    #[test]
    fn numerals_with_evidence() {
        let r = |t: &str| roman_numerals(t);
        // After a name too, a cardinal.
        assert_eq!(r("John V was king."), "John five was king.");
        assert_eq!(r("Queen Elizabeth II died."), "Queen Elizabeth two died.");
        assert_eq!(r("Henry VIII's wives"), "Henry eight's wives");
        assert_eq!(r("the Versailles of Louis XIV."), "the Versailles of Louis fourteen.");
        assert_eq!(r("Elizabeth I of England"), "Elizabeth one of England");
        assert_eq!(r("the Saturn V rocket"), "the Saturn five rocket");
        assert_eq!(r("After World War II, prices"), "After World War two, prices");
        assert_eq!(r("World War I-era"), "World War one-era");
        assert_eq!(r("Final Fantasy VII is"), "Final Fantasy seven is");
        assert_eq!(r("Super Bowl LV was"), "Super Bowl fifty-five was");
        assert_eq!(r("Part IV of"), "Part four of");
        assert_eq!(r("CHAPTER XXXV."), "CHAPTER thirty-five.");
        assert_eq!(r("Copyright MCMXCIV"), "Copyright nineteen ninety-four");
        assert_eq!(r("Class I or Class II"), "Class one or Class two");
        // A list of numerals.
        assert_eq!(r("Super Bowls XXI and XXIV"), "Super Bowls twenty-one and twenty-four");
        assert_eq!(r("George II and III"), "George two and three");
        assert_eq!(r("World Wars I and II were"), "World Wars one and two were");
        assert_eq!(r("Super Bowls XXI and XXIV were played"), "Super Bowls twenty-one and twenty-four were played");
        assert_eq!(r("Louis XIV and MCMLXXXIV"), "Louis fourteen and nineteen eighty-four");
        // Headings.
        assert_eq!(r("IV. Results"), "four. Results");
        assert_eq!(r("Intro. II. Methods"), "Intro. two. Methods");
        assert_eq!(r("World War II—the war"), "World War two—the war");
        assert_eq!(r("World Wars I and II."), "World Wars one and two.");
        assert_eq!(r("Methods. II) Data"), "Methods. two) Data");
        assert_eq!(r("Intro\nIII. Data"), "Intro\nthree. Data");
    }

    #[test]
    fn literal_without_evidence() {
        let same = |t: &str| assert_eq!(roman_numerals(t), t);
        // Determiners, function words and lowercase words.
        same("He needed an IV drip.");
        same("Start the IV now.");
        same("IV drip");
        same("They bought a CD player in the DC area.");
        same("His IV was out.");
        same("Then I left.");
        same("Then I, too, left.");
        // A list must go on or end after a numeral that continues it.
        assert_eq!(roman_numerals("After World War II, IV drug use rose."), "After World War two, IV drug use rose.");
        assert_eq!(roman_numerals("Charles III and I."), "Charles three and I.");
        same("in XIX. century");
        // Possessives.
        same("Vuitton's LV bags");
        same("Elizabeth's IV");
        same("Vuitton’s LV bags");
        // Punctuation between.
        same("Baltimore, MD");
        same("Washington, DC");
        same("(XXXII and XXXIII)");
        // Not numerals, or ones that read as letters.
        same("Vitamin D and Type C and Malcolm X");
        same("Washington DC and Sony CD and Droid CLI");
        same("Planet IIII and Mega VV");
        // The pronoun.
        same("In Paris I saw it.");
        same("The Republicans I'm aware of");
        same("When Henry I was king");
        same("Thinks I, I'll wait. \"Says I, no.");
        // Initials.
        same("Vladimir V. Putin");
        same("I. M. Pei designed it.");
        same("V. Putin said. Authors: I. Asimov wrote. I. Introduction");
        same("Aren't I? Didn't I? Well? Haven't I. Dare I? Need I! I'm II.");
        same("John III:16");
        same("Adiga, K. V. Puttappa and V. K. Gokak");
        // Glued to other things.
        same("Pop V.9221");
        same("Mark IV/V");
        same("Rocky II2");
    }
}
