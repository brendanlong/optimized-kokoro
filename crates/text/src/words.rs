//! Word lists the rules share: the closed-class words, which say nothing about the word after
//! them, and the common words of misaki's lexicon, which tell a word that only starts a sentence
//! from a name.

use std::collections::HashSet;
use std::sync::LazyLock;

/// Closed-class words (lowercase, sorted): a capitalised one before a numeral is no evidence that
/// it is a number ("The IV", "His CD", "Then I"), and one after "St." or "Dr." starts a sentence
/// ("on Main St. The next day").
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
    "down",
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
    "one",
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
    "two",
    "under",
    "unless",
    "unlike",
    "until",
    "up",
    "upon",
    "us",
    "versus",
    "very",
    "via",
    "vs",
    "was",
    "we",
    "well",
    "were",
    "what",
    "when",
    "whenever",
    "where",
    "whereas",
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

/// Whether `word` (in any case) is a closed-class word: "the", "His", "THEN".
pub(crate) fn is_function_word(word: &str) -> bool {
    FUNCTION_WORDS.binary_search(&word.to_lowercase().as_str()).is_ok()
}

/// Nouns that name the numbered parts of things, which a number after is a label of: a Roman
/// numeral after one is a number even at a sentence's start ("Chapter XII ends", "Type II
/// diabetes", where a common word may be a verb, "Give IV fluids"), and "part 1/3 of the thread"
/// is a part, not a third.
const NUMBERED_PARTS: &[&str] = &[
    "act", "appendix", "article", "book", "canto", "chapter", "class", "episode", "grade", "level", "part", "phase",
    "plate", "psalm", "round", "scene", "season", "section", "series", "stage", "step", "tier", "title", "type",
    "volume",
];

/// Whether `word` (in any case) is one of [`NUMBERED_PARTS`]: "Chapter", "part".
pub(crate) fn names_numbered_parts(word: &str) -> bool {
    forms(&word.to_lowercase()).any(|w| NUMBERED_PARTS.contains(&w))
}

/// A lowercase word and the stems its common endings may hide ("phases" "phase", "joined" "join",
/// "taking" "take", "boxes" "box"): the lexicon lists few inflected forms. "es" only after the
/// letters it follows in English ("boxes", "wishes"; not "James" as "jam").
fn forms(word: &str) -> impl Iterator<Item = &str> + '_ {
    let stems = ["s", "es", "d", "ed", "ing"].into_iter().filter_map(move |suffix| {
        let stem = word.strip_suffix(suffix)?;
        (suffix != "es" || ["s", "x", "z", "ch", "sh", "o"].iter().any(|e| stem.ends_with(e))).then_some(stem)
    });
    std::iter::once(word).chain(stems).filter(|w| w.len() >= 2)
}

/// The names: what the lexicon has capitalised (`data/names.txt`, from `tools/gen_common_words.py`),
/// which are never taken for an inflection of a common word ("Mars", "Bates").
static NAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| include_str!("../data/names.txt").lines().collect());

/// The common words: what misaki's American gold lexicon has in lowercase and not also capitalised
/// (`data/common_words.txt`, from `tools/gen_common_words.py`).
static COMMON_WORDS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| include_str!("../data/common_words.txt").lines().collect());

/// Whether a capitalised `word` is a common word rather than a name: its lowercase form is a common
/// word ("Give", "Journal", "Part"; not "Henry" or "Louis", which the lexicon also has as names, nor
/// words it doesn't have, "Saturn", "Subaru"), or an inflection of one ("Joined", "Takes",
/// "Taking", the stem with an "e": "tak" + "e").
pub(crate) fn is_common_word(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    let mut capitalised = lower.clone();
    if let Some(first) = capitalised.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    word.is_ascii()
        && !NAMES.contains(capitalised.as_str())
        && forms(&lower)
            .any(|w| COMMON_WORDS.contains(w) || w != lower && COMMON_WORDS.contains(format!("{w}e").as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists() {
        assert!(FUNCTION_WORDS.windows(2).all(|w| w[0] < w[1]), "FUNCTION_WORDS is sorted");
        assert!(is_function_word("The") && is_function_word("then") && !is_function_word("Henry"));
        for word in ["Give", "Buy", "Join", "Journal", "Bridge", "part", "Joined", "Takes", "Taking", "Phases"] {
            assert!(is_common_word(word), "{word}");
        }
        for word in ["Henry", "Louis", "Saturn", "Subaru", "Paul", "Mary", "James", "Mars"] {
            assert!(!is_common_word(word), "{word}");
        }
    }
}
