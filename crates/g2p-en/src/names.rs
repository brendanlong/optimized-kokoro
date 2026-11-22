//! Names read in their own language (`data/names.json`, built by `tools/names/build.py`): a run of
//! tokens that spells a name, exactly as written, gets the name's phonemes the way a
//! `[Minas Tirith](/mˈinas tˈiriθ/)` link would, and the lexicon and espeak never see it.

use rustc_hash::{FxHashMap, FxHashSet};

static NAMES: &str = include_str!("../data/names.json");

pub(crate) struct Names {
    phonemes: FxHashMap<&'static str, &'static str>,
    /// Every way a name can start that ends where a token can: "Minas", "Gil", "Gil-".
    prefixes: FxHashSet<&'static str>,
}

/// Where spaCy may end a token inside a name: before a space, and around hyphens and apostrophes.
fn token_ends(name: &str) -> impl Iterator<Item = usize> + '_ {
    let joiner = |c: char| c == '-' || c == '\'';
    name.char_indices().filter_map(move |(i, c)| {
        let before = name[..i].chars().next_back();
        (i > 0 && (c == ' ' || joiner(c) || before.is_some_and(joiner))).then_some(i)
    })
}

impl Names {
    pub(crate) fn new() -> Self {
        let phonemes: FxHashMap<&'static str, &'static str> =
            serde_json::from_str(NAMES).expect("bundled names are valid");
        let mut prefixes = FxHashSet::default();
        for name in phonemes.keys() {
            prefixes.extend(token_ends(name).map(|i| &name[..i]));
        }
        Names { phonemes, prefixes }
    }

    pub(crate) fn get(&self, name: &str) -> Option<&'static str> {
        self.phonemes.get(name).copied()
    }

    /// The longest name the first `n` tokens spell (each `(text, whitespace after it)`), as `(n, phonemes)`.
    pub(crate) fn longest<'t>(
        &self,
        mut tokens: impl Iterator<Item = (&'t str, &'t str)>,
    ) -> Option<(usize, &'static str)> {
        let (first, whitespace) = tokens.next()?;
        let single = self.phonemes.get(first).map(|p| (1, *p));
        if whitespace.contains('\n') || !self.prefixes.contains(first) {
            return single;
        }
        let mut spelled = String::from(first);
        if !whitespace.is_empty() {
            spelled.push(' ');
        }
        let mut found = single;
        let mut n = 1;
        for (text, whitespace) in tokens {
            spelled.push_str(text);
            n += 1;
            if let Some(p) = self.phonemes.get(spelled.as_str()) {
                found = Some((n, *p));
            }
            if whitespace.contains('\n') || !self.prefixes.contains(spelled.as_str()) {
                break;
            }
            if !whitespace.is_empty() {
                spelled.push(' ');
            }
        }
        found
    }
}
