//! misaki's `Lexicon`: dictionary lookup, stemming and special cases. misaki's number and
//! currency reading is gone: the text normalizer reads every number and symbol before G2P
//! (`docs/divergences.md`).

use crate::text::{self, is_alpha, is_lower, is_upper, lower, upper};
use rustc_hash::FxHashMap;
use serde::Deserialize;
use std::borrow::Cow;

pub(crate) const PRIMARY_STRESS: char = 'ˈ';
pub(crate) const SECONDARY_STRESS: char = 'ˌ';

pub(crate) fn is_vowel(c: char) -> bool {
    "AIOQWYaiuæɑɒɔəɛɜɪʊʌᵻ".contains(c)
}

pub(crate) fn is_consonant(c: char) -> bool {
    "bdfhjklmnpstvwzðŋɡɹɾʃʒʤʧθ".contains(c)
}

fn is_us_tau(c: char) -> bool {
    "AIOWYiuæɑəɛɪɹʊʌ".contains(c)
}

fn is_lexicon_char(c: char) -> bool {
    c == '\'' || c == '-' || c.is_ascii_alphabetic()
}

/// What the lexicon reads from a token.
pub(crate) struct LexToken<'a> {
    /// The alias if there is one, else the text.
    pub text: &'a str,
    pub tag: &'a str,
    pub stress: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TokenContext {
    pub future_vowel: Option<bool>,
    pub future_to: bool,
}

#[derive(Debug, Clone)]
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

type Dict = FxHashMap<Cow<'static, str>, Entry>;

/// `Lexicon.grow_dictionary`: adds the capitalized form of lowercase keys and the lowercase form of
/// capitalized keys, without overriding existing keys. Keys are ASCII.
fn grow(raw: Vec<(&'static str, Entry)>) -> Dict {
    let mut dict = Dict::with_capacity_and_hasher(raw.len() * 2, Default::default());
    let mut extra = Vec::new();
    for (k, v) in &raw {
        if k.len() < 2 {
            continue;
        }
        let first = k.as_bytes()[0];
        let rest_lower = !k[1..].bytes().any(|b| b.is_ascii_uppercase());
        if !first.is_ascii_uppercase() && rest_lower {
            if first.is_ascii_lowercase() {
                let mut cap = k.to_string();
                cap[..1].make_ascii_uppercase();
                extra.push((cap, v.clone()));
            }
        } else if first.is_ascii_uppercase() && rest_lower {
            extra.push((k.to_ascii_lowercase(), v.clone()));
        }
    }
    for (k, v) in extra {
        dict.insert(Cow::Owned(k), v);
    }
    for (k, v) in raw {
        dict.insert(Cow::Borrowed(k), v);
    }
    dict
}

fn load(json: &'static str) -> Dict {
    grow(parse(json))
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
    golds: Dict,
    silvers: Dict,
}

impl Lexicon {
    pub fn new(british: bool) -> Self {
        let (gold, silver) = if british { (GB_GOLD, GB_SILVER) } else { (US_GOLD, US_SILVER) };
        Lexicon { british, golds: load(gold), silvers: load(silver) }
    }

    fn gold_plain(&self, word: &str) -> &'static str {
        match self.golds.get(word) {
            Some(Entry::Plain(ps)) => ps,
            _ => panic!("{word} is a plain gold entry"),
        }
    }

    fn gold_tagged(&self, word: &str, tag: &str) -> &'static str {
        match self.golds.get(word) {
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
            match self.golds.get(&*upper(c.encode_utf8(&mut buf))) {
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

    fn get_special_case(
        &self,
        word: &str,
        tag: &str,
        stress: Option<f64>,
        ctx: &TokenContext,
    ) -> Option<Option<String>> {
        let found = |ps: &str| Some(Some(ps.to_string()));
        if tag == "ADD" && (word == "." || word == "/") {
            return Some(self.lookup(if word == "." { "dot" } else { "slash" }, None, Some(-0.5), Some(ctx)));
        }
        if word.trim_matches('.').contains('.')
            && text::str_is_alpha(&word.replace('.', ""))
            && word.split('.').map(|p| p.chars().count()).max().unwrap_or(0) < 3
        {
            return Some(self.get_nnp(word));
        }
        match word {
            "a" | "A" => found(if tag == "DT" { "ɐ" } else { "ˈA" }),
            "am" | "Am" | "AM" => {
                if tag.starts_with("NN") {
                    Some(self.get_nnp(word))
                } else if ctx.future_vowel.is_none() || word != "am" || stress.is_some_and(|s| s > 0.0) {
                    found(self.gold_plain("am"))
                } else {
                    found("ɐm")
                }
            }
            "an" | "An" | "AN" => {
                if word == "AN" && tag.starts_with("NN") {
                    Some(self.get_nnp(word))
                } else {
                    found("ɐn")
                }
            }
            "I" if tag == "PRP" => found("ˌI"),
            "by" | "By" | "BY" if parent_tag(Some(tag)) == Some("ADV") => found("bˈI"),
            "to" | "To" => found(self.to(ctx)),
            "TO" if tag == "TO" || tag == "IN" => found(self.to(ctx)),
            "in" | "In" => found(self.in_(tag, ctx)),
            "IN" if tag != "NNP" => found(self.in_(tag, ctx)),
            "the" | "The" => found(if ctx.future_vowel == Some(true) { "ði" } else { "ðə" }),
            "THE" if tag == "DT" => found(if ctx.future_vowel == Some(true) { "ði" } else { "ðə" }),
            _ if tag == "IN" && is_vs(word) => Some(self.lookup("versus", None, None, Some(ctx))),
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

    fn is_known(&self, word: &str) -> bool {
        if self.golds.contains_key(word) || self.silvers.contains_key(word) {
            return true;
        }
        if !text::str_is_alpha(word) || !word.chars().all(is_lexicon_char) {
            return false;
        }
        if word.len() == 1 {
            return true;
        }
        if is_upper(word) && self.golds.contains_key(&*lower(word)) {
            return true;
        }
        is_upper(&word[1..])
    }

    fn lookup(&self, word: &str, tag: Option<&str>, stress: Option<f64>, ctx: Option<&TokenContext>) -> Option<String> {
        let mut word = Cow::Borrowed(word);
        let mut is_nnp = false;
        if is_upper(&word) && !self.golds.contains_key(&*word) {
            word = Cow::Owned(lower(&word).into_owned());
            is_nnp = tag == Some("NNP");
        }
        let mut entry = self.golds.get(&*word);
        if entry.is_none() && !is_nnp {
            entry = self.silvers.get(&*word);
        }
        let ps = match entry {
            None => None,
            Some(Entry::Plain(ps)) => Some(*ps),
            Some(Entry::Tagged(tags)) => {
                let get = |t: Option<&str>| tags.iter().find(|(k, _)| Some(*k) == t).map(|(_, v)| *v);
                let tag = if ctx.is_some_and(|c| c.future_vowel.is_none()) && get(Some("None")).is_some() {
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

    fn s(&self, stem: Option<String>) -> Option<String> {
        let mut stem = stem.filter(|s| !s.is_empty())?;
        let last = stem.chars().next_back().expect("non-empty");
        if "ptkfθ".contains(last) {
            stem.push('s');
        } else if "szʃʒʧʤ".contains(last) {
            stem.push(if self.british { 'ɪ' } else { 'ᵻ' });
            stem.push('z');
        } else {
            stem.push('z');
        }
        Some(stem)
    }

    fn stem_s(&self, word: &str, tag: Option<&str>, stress: Option<f64>, ctx: Option<&TokenContext>) -> Option<String> {
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
        if "pkfθʃsʧ".contains(last) {
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
            stem.push_str("ɾᵻd");
        } else {
            stem.push_str("ᵻd");
        }
        Some(stem)
    }

    fn stem_ed(
        &self,
        word: &str,
        tag: Option<&str>,
        stress: Option<f64>,
        ctx: Option<&TokenContext>,
    ) -> Option<String> {
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
            stem.push_str("ɾɪŋ");
            return Some(stem);
        }
        stem.push_str("ɪŋ");
        Some(stem)
    }

    fn stem_ing(
        &self,
        word: &str,
        tag: Option<&str>,
        stress: Option<f64>,
        ctx: Option<&TokenContext>,
    ) -> Option<String> {
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
        if let Some(ps) = self.get_special_case(word, tag, stress, ctx)
            && ps.is_some()
        {
            return ps;
        }
        let wl = lower(word);
        let mut word = Cow::Borrowed(word);
        let t = Some(tag);
        if word.chars().count() > 1
            && text::str_is_alpha(&word.replace('\'', ""))
            && !is_lower(&word)
            && (tag != "NNP" || word.chars().count() > 7)
            && !self.golds.contains_key(&*word)
            && !self.silvers.contains_key(&*word)
            && (is_upper(&word) || {
                let mut cs = word.chars();
                cs.next();
                is_lower(cs.as_str())
            })
            && (self.golds.contains_key(&*wl)
                || self.silvers.contains_key(&*wl)
                || self.stem_s(&wl, t, stress, Some(ctx)).is_some_and(|s| !s.is_empty())
                || self.stem_ed(&wl, t, stress, Some(ctx)).is_some_and(|s| !s.is_empty())
                || self.stem_ing(&wl, t, stress, Some(ctx)).is_some_and(|s| !s.is_empty()))
        {
            word = Cow::Owned(wl.into_owned());
        }
        let word = &*word;
        if self.is_known(word) {
            return self.lookup(word, t, stress, Some(ctx));
        }
        if let Some(stem) = word.strip_suffix("s'") {
            let possessive = format!("{stem}'s");
            if self.is_known(&possessive) {
                return self.lookup(&possessive, t, stress, Some(ctx));
            }
        }
        if let Some(stem) = word.strip_suffix('\'')
            && self.is_known(stem)
        {
            return self.lookup(stem, t, stress, Some(ctx));
        }
        if let Some(ps) = self.stem_s(word, t, stress, Some(ctx)) {
            return Some(ps);
        }
        if let Some(ps) = self.stem_ed(word, t, stress, Some(ctx)) {
            return Some(ps);
        }
        self.stem_ing(word, t, Some(stress.unwrap_or(0.5)), Some(ctx))
    }

    /// `Lexicon.__call__`, without misaki's numbers: a word with digits has no reading here.
    pub fn get(&self, tk: &LexToken, ctx: &TokenContext) -> Option<String> {
        let word = tk.text.replace(['‘', '’'], "'");
        let word = text::nfkc(&word);
        // Every lexicon key is ASCII and the special cases need letters, so a non-ASCII character
        // that isn't a letter leaves no reading. Checking first keeps runs of such characters
        // (combining marks) from paying for every span tried.
        if word.chars().any(|c| !c.is_ascii() && !text::is_alpha(c)) {
            return None;
        }
        let stress = if is_lower(&word) {
            None
        } else if is_upper(&word) {
            Some(2.0)
        } else {
            Some(0.5)
        };
        self.get_word(&word, tk.tag, stress, ctx).map(|ps| apply_stress(&ps, tk.stress))
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
        assert_eq!(apply_stress("hɛlO", Some(0.0)), "hˌɛlO");
        assert_eq!(apply_stress("hˈɛlO", Some(-1.0)), "hˌɛlO");
        assert_eq!(apply_stress("hˌɛlO", Some(2.0)), "hˈɛlO");
        assert_eq!(apply_stress("hˈɛlˌO", Some(-2.0)), "hɛlO");
        assert_eq!(apply_stress("hm", Some(2.0)), "hm");
    }

    /// `get` relies on this to give up early on words with other non-ASCII characters.
    #[test]
    fn every_key_is_ascii() {
        for british in [false, true] {
            let lexicon = Lexicon::new(british);
            assert!(lexicon.golds.keys().chain(lexicon.silvers.keys()).all(|k| k.is_ascii()));
        }
    }

    #[test]
    fn regex_ports() {
        assert!(doubled_before_ing("running") && doubled_before_ing("kicking") && !doubled_before_ing("ring"));
    }
}
