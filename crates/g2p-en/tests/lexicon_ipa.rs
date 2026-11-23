//! The dictionaries and names are standard IPA that the model's vocabulary reads whole.

use std::collections::HashMap;

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Entry {
    Plain(String),
    Tagged(HashMap<String, Option<String>>),
    /// A spelling in `names.json` that isn't a name.
    None(()),
}

fn entries(json: &str) -> Vec<(String, String)> {
    let lexicon: HashMap<String, Entry> = serde_json::from_str(json).unwrap();
    lexicon
        .into_iter()
        .flat_map(|(word, entry)| match entry {
            Entry::Plain(ps) => vec![(word, ps)],
            Entry::Tagged(tags) => tags.into_values().flatten().map(|ps| (word.clone(), ps)).collect(),
            Entry::None(()) => vec![],
        })
        .collect()
}

#[test]
fn every_pronunciation_is_ipa_the_vocabulary_reads() {
    let files = [
        include_str!("../data/us_gold.json"),
        include_str!("../data/us_silver.json"),
        include_str!("../data/gb_gold.json"),
        include_str!("../data/gb_silver.json"),
        include_str!("../data/names.json"),
    ];
    for (word, ps) in files.into_iter().flat_map(entries) {
        // Kokoro's one-letter stand-ins, and the ɾ and ʔ misaki writes for kokoro to read as the flap and t.
        assert!(!ps.contains(['A', 'I', 'O', 'Q', 'W', 'Y', 'T', 'ʤ', 'ʧ', 'ɾ', 'ʔ']), "{word}: {ps}");
        assert_eq!(kokoro_phonemes::tokenize(&ps).dropped, 0, "{word}: {ps}");
    }
}
