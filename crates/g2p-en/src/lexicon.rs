//! misaki's `Lexicon`: dictionary lookup, stemming and special cases. misaki's number and
//! currency reading is gone: the text normalizer reads every number and symbol before G2P
//! (`docs/divergences.md`).

use crate::text::{self, is_alpha, is_lower, is_upper, lower, upper};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::borrow::Cow;

pub(crate) const PRIMARY_STRESS: char = 'ˈ';
pub(crate) const SECONDARY_STRESS: char = 'ˌ';

/// With e, o and y, and r, x, ç and ɣ, which only names use (`data/names.json`), so "the Eldar" is "thee".
pub(crate) fn is_vowel(c: char) -> bool {
    "aeiouyæɑɒɔəɛɜɪʊʌᵻ".contains(c)
}

pub(crate) fn is_consonant(c: char) -> bool {
    "bdfhjklmnprstvwxzçðŋɡɣɹɾʃʒθ".contains(c)
}

fn is_us_tau(c: char) -> bool {
    "iuæɑəɛɪɹʊʌ".contains(c)
}

fn is_lexicon_char(c: char) -> bool {
    c == '\'' || c == '-' || c.is_ascii_alphabetic()
}

/// What the lexicon reads from a token.
pub(crate) struct LexToken<'a> {
    pub text: &'a str,
    pub tag: &'a str,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TokenContext {
    pub future_vowel: Option<bool>,
    pub future_to: bool,
}

#[derive(Debug, PartialEq)]
enum Entry {
    Plain(&'static str),
    /// Phonemes by tag; always has `DEFAULT`.
    Tagged(Vec<(&'static str, Option<&'static str>)>),
}

impl Deserialize<'static> for Entry {
    fn deserialize<D: serde::Deserializer<'static>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'static> for V {
            type Value = Entry;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("phonemes or a map of tag to phonemes")
            }
            fn visit_borrowed_str<E>(self, v: &'static str) -> Result<Entry, E> {
                Ok(Entry::Plain(v))
            }
            fn visit_map<A: serde::de::MapAccess<'static>>(self, mut map: A) -> Result<Entry, A::Error> {
                let mut out = Vec::new();
                while let Some(kv) = map.next_entry::<&'static str, Option<&'static str>>()? {
                    out.push(kv);
                }
                Ok(Entry::Tagged(out))
            }
        }
        d.deserialize_any(V)
    }
}

/// The JSON object's entries in file order, borrowing from the bundled text.
fn parse(json: &'static str) -> Vec<(&'static str, Entry)> {
    struct V;
    impl serde::de::Visitor<'static> for V {
        type Value = Vec<(&'static str, Entry)>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a lexicon object")
        }
        fn visit_map<A: serde::de::MapAccess<'static>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::with_capacity(map.size_hint().unwrap_or(0));
            while let Some(kv) = map.next_entry::<&'static str, Entry>()? {
                out.push(kv);
            }
            Ok(out)
        }
    }
    let mut de = serde_json::Deserializer::from_str(json);
    serde::Deserializer::deserialize_map(&mut de, V).expect("bundled lexicon is valid")
}

/// A spelling the dictionary has, and which of misaki's two dictionaries has it.
#[derive(Debug)]
struct Variant {
    spelling: &'static str,
    gold: bool,
    entry: Entry,
}

/// Every spelling of a word, gold first.
enum Group {
    One(Variant),
    Many(Vec<Variant>),
}

impl Group {
    fn variants(&self) -> &[Variant] {
        match self {
            Group::One(v) => std::slice::from_ref(v),
            Group::Many(v) => v,
        }
    }
}

/// The words keyed by their lowercase form: so case never hides a word ("mcdonald",
/// "MCDONALD"), and only the words whose spellings read differently ("Polish", "polish") look at
/// how it's written.
type Dict = FxHashMap<Cow<'static, str>, Group>;

/// Whether misaki's `grow_dictionary` gives `key` the spelling `word`: the capitalized form of a
/// lowercase key, or the lowercase form of a capitalized one.
fn grown(key: &str, word: &str) -> bool {
    let (Some(k), Some(w)) = (key.as_bytes().first(), word.as_bytes().first()) else { return false };
    let rest_lower = |s: &str| !s[1..].bytes().any(|b| b.is_ascii_uppercase());
    key.len() >= 2
        && word.is_ascii()
        && key[1..] == word[1..]
        && rest_lower(key)
        && (k.is_ascii_lowercase() && *w == k.to_ascii_uppercase()
            || k.is_ascii_uppercase() && *w == k.to_ascii_lowercase())
}

/// Lowercase abbreviations the lexicons expand without the period or capital that would say
/// which word they stand for (`docs/divergences.md`, "misaki English: lowercase abbreviations"):
/// "dr" is "doctor" in misaki, but also "tl;dr", drams and drives, and "mr" (which also reads
/// "MR") is "mister" but also a merge request or magnetic resonance. Without them, the word is
/// spelled out. "Dr" and "Mr" stay, as do "mrs" and "vs", which have one reading.
const GUESSES: [&str; 2] = ["dr", "mr"];

fn load(gold: &'static str, silver: &'static str) -> Dict {
    let (gold, silver) = (parse(gold), parse(silver));
    let mut dict = Dict::with_capacity_and_hasher(gold.len() + silver.len(), Default::default());
    for (entries, gold) in [(gold, true), (silver, false)] {
        for (spelling, entry) in entries {
            let key = if spelling.bytes().any(|b| b.is_ascii_uppercase()) {
                Cow::Owned(spelling.to_ascii_lowercase())
            } else {
                Cow::Borrowed(spelling)
            };
            let variant = Variant { spelling, gold, entry };
            match dict.entry(key) {
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(Group::One(variant));
                }
                std::collections::hash_map::Entry::Occupied(mut e) => match e.get_mut() {
                    Group::Many(v) => v.push(variant),
                    group => {
                        let Group::One(first) = std::mem::replace(group, Group::Many(Vec::new())) else {
                            unreachable!("matched above")
                        };
                        *group = Group::Many(vec![first, variant]);
                    }
                },
            }
        }
    }
    dict
}

fn misaki_entry<'a>(variants: &'a [Variant], word: &str, gold: bool) -> Option<&'a Entry> {
    if GUESSES.contains(&word) {
        return None;
    }
    let mut variants = variants.iter().filter(|v| v.gold == gold);
    let exact = variants.clone().find(|v| v.spelling == word);
    exact.or_else(|| variants.find(|v| grown(v.spelling, word))).map(|v| &v.entry)
}

static US_GOLD: &str = include_str!("../data/us_gold.json");
static US_SILVER: &str = include_str!("../data/us_silver.json");
static GB_GOLD: &str = include_str!("../data/gb_gold.json");
static GB_SILVER: &str = include_str!("../data/gb_silver.json");

pub(crate) fn parent_tag(tag: Option<&str>) -> Option<&str> {
    let tag = tag?;
    Some(if tag.starts_with("VB") {
        "VERB"
    } else if tag.starts_with("NN") {
        "NOUN"
    } else if tag.starts_with("ADV") || tag.starts_with("RB") {
        "ADV"
    } else if tag.starts_with("ADJ") || tag.starts_with("JJ") {
        "ADJ"
    } else {
        tag
    })
}

/// Moves a stress mark to just before the first vowel after it.
fn restress(mark: char, ps: &str) -> String {
    let mut out = String::with_capacity(ps.len() + 2);
    let at = ps.find(is_vowel).expect("caller checked for a vowel");
    out.push_str(&ps[..at]);
    out.push(mark);
    out.push_str(&ps[at..]);
    out
}

pub(crate) fn apply_stress(ps: &str, stress: Option<f64>) -> String {
    let Some(stress) = stress else { return ps.to_string() };
    let has_stress = ps.contains([PRIMARY_STRESS, SECONDARY_STRESS]);
    if stress < -1.0 {
        ps.replace([PRIMARY_STRESS, SECONDARY_STRESS], "")
    } else if stress == -1.0 || ((stress == 0.0 || stress == -0.5) && ps.contains(PRIMARY_STRESS)) {
        ps.replace(SECONDARY_STRESS, "").replace(PRIMARY_STRESS, "ˌ")
    } else if (stress == 0.0 || stress == 0.5 || stress == 1.0) && !has_stress {
        if !ps.contains(is_vowel) { ps.to_string() } else { restress(SECONDARY_STRESS, ps) }
    } else if stress >= 1.0 && !ps.contains(PRIMARY_STRESS) && ps.contains(SECONDARY_STRESS) {
        ps.replace(SECONDARY_STRESS, "ˈ")
    } else if stress > 1.0 && !has_stress {
        if !ps.contains(is_vowel) { ps.to_string() } else { restress(PRIMARY_STRESS, ps) }
    } else {
        ps.to_string()
    }
}

pub(crate) struct Lexicon {
    british: bool,
    dict: Dict,
}

impl Lexicon {
    pub fn new(british: bool) -> Self {
        let (gold, silver) = if british { (GB_GOLD, GB_SILVER) } else { (US_GOLD, US_SILVER) };
        Lexicon { british, dict: load(gold, silver) }
    }

    fn variants(&self, word: &str) -> &[Variant] {
        // Every key is ASCII (`every_key_is_ascii`).
        if !word.is_ascii() {
            return &[];
        }
        let mut buf = [0; 32];
        let key = match buf.get_mut(..word.len()) {
            _ if !word.bytes().any(|b| b.is_ascii_uppercase()) => Cow::Borrowed(word),
            Some(buf) => {
                buf.copy_from_slice(word.as_bytes());
                buf.make_ascii_lowercase();
                Cow::Borrowed(std::str::from_utf8(buf).expect("ASCII"))
            }
            None => Cow::Owned(word.to_ascii_lowercase()),
        };
        self.dict.get(&*key).map_or(&[], Group::variants)
    }

    /// misaki's `golds.get(word)` (`gold`) or `silvers.get(word)`: the spelling as written, or
    /// as `grow_dictionary` adds it.
    fn misaki_entry(&self, word: &str, gold: bool) -> Option<&Entry> {
        misaki_entry(self.variants(word), word, gold)
    }

    /// The word's entry, from the gold dictionary only if `gold_only`: what misaki finds (gold
    /// first), or else the reading every spelling of it shares, whatever its case.
    fn entry(&self, word: &str, gold_only: bool) -> Option<&Entry> {
        let variants = self.variants(word);
        if let Some(e) = misaki_entry(variants, word, true) {
            return Some(e);
        }
        if !gold_only && let Some(e) = misaki_entry(variants, word, false) {
            return Some(e);
        }
        if GUESSES.iter().any(|g| g.eq_ignore_ascii_case(word)) {
            return None;
        }
        let mut entries = variants.iter().filter(|v| v.gold || !gold_only).map(|v| &v.entry);
        let first = entries.next()?;
        entries.all(|e| e == first).then_some(first)
    }

    /// Whether misaki has no entry for `word` in its case, or lowercase, where one in another case
    /// reads it ("Mori" for "MORI").
    pub(crate) fn found_by_case_alone(&self, word: &str) -> bool {
        let misaki = |w: &str| self.misaki_entry(w, true).is_some() || self.misaki_entry(w, false).is_some();
        self.entry(word, false).is_some() && !misaki(word) && !misaki(&lower(word))
    }

    /// Whether misaki's stress for a word in capitals reads `word` differently (`get`).
    pub(crate) fn shouted_differently(&self, word: &str) -> bool {
        let read = |stress| self.get_word(word, "NN", Some(stress), &TokenContext::default());
        is_upper(word) && !is_lower(word) && read(2.0) != read(0.5)
    }

    fn gold_plain(&self, word: &str) -> &'static str {
        match self.misaki_entry(word, true) {
            Some(Entry::Plain(ps)) => ps,
            _ => panic!("{word} is a plain gold entry"),
        }
    }

    fn gold_tagged(&self, word: &str, tag: &str) -> &'static str {
        match self.misaki_entry(word, true) {
            Some(Entry::Tagged(tags)) => {
                tags.iter().find(|(t, _)| *t == tag).and_then(|(_, ps)| *ps).expect("tag present")
            }
            _ => panic!("{word} is a tagged gold entry"),
        }
    }

    fn get_nnp(&self, word: &str) -> Option<String> {
        let mut ps = String::new();
        for c in word.chars().filter(|&c| is_alpha(c)) {
            let mut buf = [0; 4];
            match self.entry(&upper(c.encode_utf8(&mut buf)), true) {
                Some(Entry::Plain(p)) => ps.push_str(p),
                _ => return None,
            }
        }
        let ps = apply_stress(&ps, Some(0.0));
        Some(match ps.rfind(SECONDARY_STRESS) {
            Some(i) => format!("{}{PRIMARY_STRESS}{}", &ps[..i], &ps[i + SECONDARY_STRESS.len_utf8()..]),
            None => ps,
        })
    }

    fn get_special_case(&self, word: &str, tag: &str, stress: Option<f64>, ctx: &TokenContext) -> Option<String> {
        let found = |ps: &str| Some(ps.to_string());
        if tag == "ADD" && (word == "." || word == "/") {
            return self.lookup(if word == "." { "dot" } else { "slash" }, None, Some(-0.5), ctx);
        }
        if word.trim_matches('.').contains('.')
            && text::str_is_alpha(&word.replace('.', ""))
            && word.split('.').map(|p| p.chars().count()).max().unwrap_or(0) < 3
        {
            return self.get_nnp(word);
        }
        match word {
            "a" | "A" => found(if tag == "DT" { "ɐ" } else { "ˈe͡ɪ" }),
            "am" | "Am" | "AM" => {
                if tag.starts_with("NN") {
                    self.get_nnp(word)
                } else if ctx.future_vowel.is_none() || word != "am" || stress.is_some_and(|s| s > 0.0) {
                    found(self.gold_plain("am"))
                } else {
                    found("ɐm")
                }
            }
            "an" | "An" | "AN" => {
                if word == "AN" && tag.starts_with("NN") {
                    self.get_nnp(word)
                } else {
                    found("ɐn")
                }
            }
            "I" if tag == "PRP" => found("ˌa͡ɪ"),
            "by" | "By" | "BY" if parent_tag(Some(tag)) == Some("ADV") => found("ˈba͡ɪ"),
            "to" | "To" => found(self.to(ctx)),
            "TO" if tag == "TO" || tag == "IN" => found(self.to(ctx)),
            "in" | "In" => found(self.in_(tag, ctx)),
            "IN" if tag != "NNP" => found(self.in_(tag, ctx)),
            "the" | "The" => found(if ctx.future_vowel == Some(true) { "ði" } else { "ðə" }),
            "THE" if tag == "DT" => found(if ctx.future_vowel == Some(true) { "ði" } else { "ðə" }),
            _ if tag == "IN" && is_vs(word) => self.lookup("versus", None, None, ctx),
            "used" | "Used" | "USED" => {
                if (tag == "VBD" || tag == "JJ") && ctx.future_to {
                    found(self.gold_tagged("used", "VBD"))
                } else {
                    found(self.gold_tagged("used", "DEFAULT"))
                }
            }
            _ => None,
        }
    }

    fn to(&self, ctx: &TokenContext) -> &'static str {
        match ctx.future_vowel {
            None => self.gold_plain("to"),
            Some(false) => "tə",
            Some(true) => "tʊ",
        }
    }

    fn in_(&self, tag: &str, ctx: &TokenContext) -> &'static str {
        if ctx.future_vowel.is_none() || tag != "IN" { "ˈɪn" } else { "ɪn" }
    }

    pub(crate) fn is_known(&self, word: &str) -> bool {
        if self.entry(word, false).is_some() {
            return true;
        }
        if !text::str_is_alpha(word) || !word.chars().all(is_lexicon_char) {
            return false;
        }
        if word.len() == 1 {
            return true;
        }
        is_upper(&word[1..])
    }

    fn lookup(&self, word: &str, tag: Option<&str>, stress: Option<f64>, ctx: &TokenContext) -> Option<String> {
        let mut word = Cow::Borrowed(word);
        let mut is_nnp = false;
        if is_upper(&word) && self.misaki_entry(&word, true).is_none() {
            word = Cow::Owned(lower(&word).into_owned());
            is_nnp = tag == Some("NNP");
        }
        let ps = match self.entry(&word, is_nnp) {
            None => None,
            Some(Entry::Plain(ps)) => Some(*ps),
            Some(Entry::Tagged(tags)) => {
                let get = |t: Option<&str>| tags.iter().find(|(k, _)| Some(*k) == t).map(|(_, v)| *v);
                let tag = if ctx.future_vowel.is_none() && get(Some("None")).is_some() {
                    Some("None")
                } else if get(tag).is_none() {
                    parent_tag(tag)
                } else {
                    tag
                };
                get(tag).unwrap_or_else(|| get(Some("DEFAULT")).expect("DEFAULT present"))
            }
        };
        if ps.is_none_or(|ps| is_nnp && !ps.contains(PRIMARY_STRESS))
            && let Some(nnp) = self.get_nnp(&word)
        {
            return Some(nnp);
        }
        ps.map(|ps| apply_stress(ps, stress))
    }

    pub(crate) fn s(&self, stem: Option<String>) -> Option<String> {
        let mut stem = stem.filter(|s| !s.is_empty())?;
        let last = stem.chars().next_back().expect("non-empty");
        // x and ç are only in names (crates/g2p-en/data/names.json): "Anach's".
        if "ptkfθxç".contains(last) {
            stem.push('s');
        } else if "szʃʒ".contains(last) {
            stem.push(if self.british { 'ɪ' } else { 'ᵻ' });
            stem.push('z');
        } else {
            stem.push('z');
        }
        Some(stem)
    }

    fn stem_s(&self, word: &str, tag: Option<&str>, stress: Option<f64>, ctx: &TokenContext) -> Option<String> {
        if clen(word) < 3 || !word.ends_with('s') {
            return None;
        }
        let stem: Cow<str> = if !word.ends_with("ss") && self.is_known(&word[..word.len() - 1]) {
            Cow::Borrowed(&word[..word.len() - 1])
        } else if (word.ends_with("'s") || (clen(word) > 4 && word.ends_with("es") && !word.ends_with("ies")))
            && self.is_known(&word[..word.len() - 2])
        {
            Cow::Borrowed(&word[..word.len() - 2])
        } else if clen(word) > 4 && word.ends_with("ies") && self.is_known(&format!("{}y", &word[..word.len() - 3])) {
            Cow::Owned(format!("{}y", &word[..word.len() - 3]))
        } else {
            return None;
        };
        self.s(self.lookup(&stem, tag, stress, ctx))
    }

    fn ed(&self, stem: Option<String>) -> Option<String> {
        let mut stem = stem.filter(|s| !s.is_empty())?;
        let mut chars = stem.chars().rev();
        let last = chars.next().expect("non-empty");
        let before = chars.next();
        let schwa_or_i = if self.british { 'ɪ' } else { 'ᵻ' };
        if "pkfθʃs".contains(last) {
            stem.push('t');
        } else if last == 'd' {
            stem.push(schwa_or_i);
            stem.push('d');
        } else if last != 't' {
            stem.push('d');
        } else if self.british || before.is_none() {
            stem.push_str("ɪd");
        } else if before.is_some_and(is_us_tau) {
            stem.pop();
            stem.push_str("t̬ᵻd");
        } else {
            stem.push_str("ᵻd");
        }
        Some(stem)
    }

    fn stem_ed(&self, word: &str, tag: Option<&str>, stress: Option<f64>, ctx: &TokenContext) -> Option<String> {
        if clen(word) < 4 || !word.ends_with('d') {
            return None;
        }
        let stem = if !word.ends_with("dd") && self.is_known(&word[..word.len() - 1]) {
            &word[..word.len() - 1]
        } else if clen(word) > 4
            && word.ends_with("ed")
            && !word.ends_with("eed")
            && self.is_known(&word[..word.len() - 2])
        {
            &word[..word.len() - 2]
        } else {
            return None;
        };
        self.ed(self.lookup(stem, tag, stress, ctx))
    }

    fn ing(&self, stem: Option<String>) -> Option<String> {
        let mut stem = stem.filter(|s| !s.is_empty())?;
        let mut chars = stem.chars().rev();
        let last = chars.next().expect("non-empty");
        let before = chars.next();
        if self.british {
            if last == 'ə' || last == 'ː' {
                return None;
            }
        } else if last == 't' && before.is_some_and(is_us_tau) {
            stem.pop();
            stem.push_str("t̬ɪŋ");
            return Some(stem);
        }
        stem.push_str("ɪŋ");
        Some(stem)
    }

    fn stem_ing(&self, word: &str, tag: Option<&str>, stress: Option<f64>, ctx: &TokenContext) -> Option<String> {
        if clen(word) < 5 || !word.ends_with("ing") {
            return None;
        }
        let base = &word[..word.len() - 3];
        let stem: Cow<str> = if clen(word) > 5 && self.is_known(base) {
            Cow::Borrowed(base)
        } else if self.is_known(&format!("{base}e")) {
            Cow::Owned(format!("{base}e"))
        } else if clen(word) > 5 && doubled_before_ing(word) && self.is_known(drop_last_chars(word, 4)) {
            Cow::Borrowed(drop_last_chars(word, 4))
        } else {
            return None;
        };
        self.ing(self.lookup(&stem, tag, stress, ctx))
    }

    fn get_word(&self, word: &str, tag: &str, stress: Option<f64>, ctx: &TokenContext) -> Option<String> {
        if let Some(ps) = self.get_special_case(word, tag, stress, ctx) {
            return Some(ps);
        }
        let wl = lower(word);
        let mut word = Cow::Borrowed(word);
        let t = Some(tag);
        if word.chars().count() > 1
            && text::str_is_alpha(&word.replace('\'', ""))
            && !is_lower(&word)
            && (tag != "NNP" || word.chars().count() > 7)
            && self.misaki_entry(&word, true).is_none()
            && self.misaki_entry(&word, false).is_none()
            && (is_upper(&word) || {
                let mut cs = word.chars();
                cs.next();
                is_lower(cs.as_str())
            })
            && (self.misaki_entry(&wl, true).is_some()
                || self.misaki_entry(&wl, false).is_some()
                || self.stem_s(&wl, t, stress, ctx).is_some()
                || self.stem_ed(&wl, t, stress, ctx).is_some()
                || self.stem_ing(&wl, t, stress, ctx).is_some())
        {
            word = Cow::Owned(wl.into_owned());
        }
        let word = &*word;
        if self.is_known(word) {
            return self.lookup(word, t, stress, ctx);
        }
        if let Some(stem) = word.strip_suffix("s'") {
            let possessive = format!("{stem}'s");
            if self.is_known(&possessive) {
                return self.lookup(&possessive, t, stress, ctx);
            }
        }
        if let Some(stem) = word.strip_suffix('\'')
            && self.is_known(stem)
        {
            return self.lookup(stem, t, stress, ctx);
        }
        if let Some(ps) = self.stem_s(word, t, stress, ctx) {
            return Some(ps);
        }
        if let Some(ps) = self.stem_ed(word, t, stress, ctx) {
            return Some(ps);
        }
        self.stem_ing(word, t, Some(stress.unwrap_or(0.5)), ctx)
    }

    /// `Lexicon.__call__`, without misaki's numbers (a word with digits has no reading here), and
    /// with a word in capitals stressed as if only its first letter were: shouting doesn't change
    /// how a word is said.
    pub fn get(&self, tk: &LexToken, ctx: &TokenContext) -> Option<String> {
        let word = tk.text.replace(['‘', '’'], "'");
        let word = text::nfkc(&word);
        // Every lexicon key is ASCII and the special cases need letters, so a non-ASCII character
        // that isn't a letter leaves no reading. Checking first keeps runs of such characters
        // (combining marks) from paying for every span tried.
        if word.chars().any(|c| !c.is_ascii() && !text::is_alpha(c)) {
            return None;
        }
        let stress = if is_lower(&word) { None } else { Some(0.5) };
        self.get_word(&word, tk.tag, stress, ctx)
    }
}

fn clen(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.chars().count() }
}

fn drop_last_chars(s: &str, n: usize) -> &str {
    s.char_indices().rev().nth(n - 1).map_or("", |(i, _)| &s[..i])
}

/// `re.match(r'(?i)vs\.?$', word)`; case-insensitive `s` also matches `ſ`.
fn is_vs(word: &str) -> bool {
    let word = word.strip_suffix('\n').unwrap_or(word);
    let word = word.strip_suffix('.').unwrap_or(word);
    let mut cs = word.chars();
    matches!((cs.next(), cs.next(), cs.next()), (Some('v' | 'V'), Some('s' | 'S' | 'ſ'), None))
}

/// `re.search(r'([bcdgklmnprstvxz])\1ing$|cking$', word)`
fn doubled_before_ing(word: &str) -> bool {
    let word = word.strip_suffix('\n').unwrap_or(word);
    let Some(stem) = word.strip_suffix("ing") else { return false };
    if stem.ends_with("ck") {
        return true;
    }
    let mut cs = stem.chars().rev();
    matches!((cs.next(), cs.next()), (Some(a), Some(b)) if a == b && "bcdgklmnprstvxz".contains(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress() {
        assert_eq!(apply_stress("hɛlo͡ʊ", Some(0.0)), "hˌɛlo͡ʊ");
        assert_eq!(apply_stress("hˈɛlo͡ʊ", Some(-1.0)), "hˌɛlo͡ʊ");
        assert_eq!(apply_stress("hˌɛlo͡ʊ", Some(2.0)), "hˈɛlo͡ʊ");
        assert_eq!(apply_stress("hˈɛlˌo͡ʊ", Some(-2.0)), "hɛlo͡ʊ");
        assert_eq!(apply_stress("hm", Some(2.0)), "hm");
    }

    /// `get` relies on this to give up early on words with other non-ASCII characters.
    #[test]
    fn every_key_is_ascii() {
        for british in [false, true] {
            let lexicon = Lexicon::new(british);
            assert!(lexicon.dict.values().flat_map(Group::variants).all(|v| v.spelling.is_ascii()));
        }
    }

    #[test]
    fn guessed_abbreviations_are_dropped() {
        for british in [false, true] {
            let lexicon = Lexicon::new(british);
            let known = |w: &str| lexicon.entry(w, false).is_some();
            assert!(!known("dr") && !known("mr") && !known("DR"));
            assert!(known("Dr") && known("Mr") && known("Dr.") && known("mrs"));
        }
    }

    #[test]
    fn case_never_hides_a_word() {
        let lexicon = Lexicon::new(false);
        let read = |w: &str| lexicon.entry(w, false);
        for (word, written) in
            [("mcdonald", "McDonald"), ("MCDONALD", "McDonald"), ("Mcdonald", "McDonald"), ("javascript", "JavaScript")]
        {
            assert!(read(word).is_some_and(|e| Some(e) == read(written)), "{word}");
        }
        // Spellings that read differently keep their own readings.
        assert_ne!(read("Polish"), read("polish"));
        assert_eq!(read("POLISH"), None);
        assert_eq!(read("August"), lexicon.misaki_entry("August", true));
        assert_eq!(read("august"), lexicon.misaki_entry("august", true));
    }

    #[test]
    fn regex_ports() {
        assert!(doubled_before_ing("running") && doubled_before_ing("kicking") && !doubled_before_ing("ring"));
    }
}
