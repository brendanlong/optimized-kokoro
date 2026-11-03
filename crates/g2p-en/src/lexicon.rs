//! misaki's `Lexicon`: dictionary lookup, stemming, special cases and numbers.

use crate::num2words;
use crate::text::{self, is_alpha, is_ascii_digits, is_lower, is_upper, lower, upper};
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

const ORDINALS: [&str; 4] = ["st", "nd", "rd", "th"];

/// `CURRENCIES`: symbol to (unit, subunit).
pub(crate) fn currency_units(symbol: char) -> Option<(&'static str, &'static str)> {
    match symbol {
        '$' => Some(("dollar", "cent")),
        '£' => Some(("pound", "pence")),
        '€' => Some(("euro", "cent")),
        _ => None,
    }
}

fn symbol_word(word: &str) -> Option<&'static str> {
    match word {
        "%" => Some("percent"),
        "&" => Some("and"),
        "+" => Some("plus"),
        "@" => Some("at"),
        _ => None,
    }
}

/// What the lexicon reads from a token.
pub(crate) struct LexToken<'a> {
    /// The alias if there is one, else the text.
    pub text: &'a str,
    pub tag: &'a str,
    pub stress: Option<f64>,
    pub currency: Option<char>,
    pub is_head: bool,
    pub num_flags: &'a str,
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

/// `re.split(r'[^a-z]+', s)`
fn split_words(s: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(s);
    std::iter::from_fn(move || {
        let s = rest?;
        match s.find(|c: char| !c.is_ascii_lowercase()) {
            None => {
                rest = None;
                Some(s)
            }
            Some(i) => {
                let after = &s[i..];
                let skip = after.find(|c: char| c.is_ascii_lowercase()).unwrap_or(after.len());
                rest = Some(&after[skip..]);
                Some(&s[..i])
            }
        }
    })
}

/// The longest suffix of `[a-z']` characters.
fn number_suffix(word: &str) -> &str {
    let start = word.trim_end_matches(|c: char| c.is_ascii_lowercase() || c == '\'').len();
    &word[start..]
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
        if let Some(w) = symbol_word(word) {
            return Some(self.lookup(w, None, None, Some(ctx)));
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
        if self.golds.contains_key(word) || symbol_word(word).is_some() || self.silvers.contains_key(word) {
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

    fn is_currency(word: &str) -> bool {
        match word.split_once('.') {
            None => true,
            Some((_, cents)) => !cents.contains('.') && cents.chars().count() < 3,
        }
    }

    fn get_number(
        &self,
        word: &str,
        currency: Option<char>,
        is_head: bool,
        num_flags: &str,
    ) -> Result<Option<String>, num2words::Error> {
        let suffix = number_suffix(word);
        let suffix = (!suffix.is_empty()).then_some(suffix);
        let mut word = &word[..word.len() - suffix.map_or(0, str::len)];
        let mut result: Vec<String> = Vec::new();
        if let Some(rest) = word.strip_prefix('-') {
            result.push(self.lookup("minus", None, None, None).unwrap_or_default());
            word = rest;
        }
        let is_ordinal = suffix.is_some_and(|s| ORDINALS.contains(&s));
        if is_ascii_digits(word) && is_ordinal {
            self.extend_num(&mut result, &num2words::ordinal(word.trim_end_matches('\n'))?, true, num_flags);
        } else if result.is_empty() && word.chars().count() == 4 && currency.is_none() && is_ascii_digits(word) {
            let n: u64 = word.trim_end_matches('\n').parse().expect("digits");
            self.extend_num(&mut result, &num2words::year(n), true, num_flags);
        } else if !is_head && !word.contains('.') {
            let num = word.replace(',', "");
            let chars: Vec<char> = num.chars().collect();
            if chars[0] == '0' || chars.len() > 3 {
                for c in &chars {
                    self.extend_num(&mut result, &cardinal_char(*c)?, false, num_flags);
                }
            } else if chars.len() == 3 && !num.ends_with("00") {
                self.extend_num(&mut result, &cardinal_char(chars[0])?, true, num_flags);
                if chars[1] == '0' {
                    result.push(self.lookup("O", None, Some(-2.0), None).unwrap_or_default());
                    self.extend_num(&mut result, &cardinal_char(chars[2])?, false, num_flags);
                } else {
                    self.extend_num(&mut result, &num2words::cardinal(&num[1..])?, false, num_flags);
                }
            } else {
                self.extend_num(&mut result, &num2words::cardinal(&num)?, true, num_flags);
            }
        } else if word.matches('.').count() > 1 || !is_head {
            let mut first = true;
            for num in word.replace(',', "").split('.') {
                if num.is_empty() {
                } else if num.starts_with('0') || (num.chars().count() != 2 && num.chars().skip(1).any(|c| c != '0')) {
                    for c in num.chars() {
                        self.extend_num(&mut result, &cardinal_char(c)?, false, num_flags);
                    }
                } else {
                    self.extend_num(&mut result, &num2words::cardinal(num)?, first, num_flags);
                }
                first = false;
            }
        } else if let Some((unit, subunit)) = currency.and_then(currency_units).filter(|_| Self::is_currency(word)) {
            let plain = word.replace(',', "");
            let mut pairs: Vec<(&str, &str)> = plain.split('.').zip([unit, subunit]).collect();
            let is_zero = |n: &str| n.bytes().all(|b| b == b'0');
            if pairs.len() > 1 {
                if is_zero(pairs[1].0) {
                    pairs.truncate(1);
                } else if is_zero(pairs[0].0) {
                    pairs.remove(0);
                }
            }
            for (i, (num, unit)) in pairs.into_iter().enumerate() {
                if i > 0 {
                    result.push(self.lookup("and", None, None, None).unwrap_or_default());
                }
                let num = if num.is_empty() { "0" } else { num };
                self.extend_num(&mut result, &num2words::cardinal(num)?, i == 0, num_flags);
                let one = num.trim_start_matches('0') == "1";
                let ps = if !one && unit != "pence" {
                    self.stem_s(&format!("{unit}s"), None, None, None)
                } else {
                    self.lookup(unit, None, None, None)
                };
                result.push(ps.unwrap_or_default());
            }
        } else {
            let words = if is_ascii_digits(word) {
                num2words::cardinal(word.trim_end_matches('\n'))?
            } else if !word.contains('.') {
                let digits = word.replace(',', "");
                if is_ordinal { num2words::ordinal(&digits)? } else { num2words::cardinal(&digits)? }
            } else {
                let plain = word.replace(',', "");
                if let Some(frac) = plain.strip_prefix('.') {
                    let mut w = String::from("point ");
                    for (i, c) in frac.chars().enumerate() {
                        if i > 0 {
                            w.push(' ');
                        }
                        w.push_str(&cardinal_char(c)?);
                    }
                    w
                } else {
                    num2words::float(&plain)?
                }
            };
            self.extend_num(&mut result, &words, true, num_flags);
        }
        if result.is_empty() {
            return Ok(None);
        }
        let joined = result.join(" ");
        Ok(match suffix {
            Some("s" | "'s") => self.s(Some(joined)),
            Some("ed" | "'d") => self.ed(Some(joined)),
            Some("ing") => self.ing(Some(joined)),
            _ => Some(joined),
        })
    }

    fn extend_num(&self, result: &mut Vec<String>, words: &str, first: bool, num_flags: &str) {
        let splits: Vec<&str> = split_words(words).collect();
        for (i, w) in splits.iter().enumerate() {
            if *w != "and" || num_flags.contains('&') {
                if first && i == 0 && splits.len() > 1 && *w == "one" && num_flags.contains('a') {
                    result.push("ə".to_string());
                } else {
                    let stress = (*w == "point").then_some(-2.0);
                    result.push(self.lookup(w, None, stress, None).unwrap_or_default());
                }
            } else if num_flags.contains('n')
                && let Some(last) = result.last_mut()
            {
                last.push_str("ən");
            }
        }
    }

    fn append_currency(&self, ps: String, currency: Option<char>) -> String {
        let Some(symbol) = currency else { return ps };
        match currency_units(symbol).and_then(|(unit, _)| self.stem_s(&format!("{unit}s"), None, None, None)) {
            Some(unit) => format!("{ps} {unit}"),
            None => ps,
        }
    }

    fn is_number(word: &str, is_head: bool) -> bool {
        if !word.chars().any(|c| c.is_ascii_digit()) {
            return false;
        }
        let mut word = word;
        for s in ["ing", "'d", "ed", "'s", "st", "nd", "rd", "th", "s"] {
            if let Some(w) = word.strip_suffix(s) {
                word = w;
                break;
            }
        }
        word.chars()
            .enumerate()
            .all(|(i, c)| c.is_ascii_digit() || c == ',' || c == '.' || (is_head && i == 0 && c == '-'))
    }

    /// `Lexicon.__call__`
    pub fn get(&self, tk: &LexToken, ctx: &TokenContext) -> Result<Option<String>, num2words::Error> {
        let word = tk.text.replace(['‘', '’'], "'");
        let word = text::nfkc(&word);
        let word: Cow<str> = if word.is_ascii() {
            word
        } else {
            Cow::Owned(word.chars().map(|c| text::digit_value(c).map_or(c, |d| char::from(b'0' + d))).collect())
        };
        // Every lexicon key is ASCII, numbers are ASCII digits by now, and the special cases need
        // letters, so a non-ASCII character that isn't a letter leaves no reading. Checking first
        // keeps runs of such characters (combining marks) from paying for every span tried.
        if word.chars().any(|c| !c.is_ascii() && !text::is_alpha(c)) {
            return Ok(None);
        }
        let stress = if is_lower(&word) {
            None
        } else if is_upper(&word) {
            Some(2.0)
        } else {
            Some(0.5)
        };
        if let Some(ps) = self.get_word(&word, tk.tag, stress, ctx) {
            return Ok(Some(apply_stress(&self.append_currency(ps, tk.currency), tk.stress)));
        }
        if Self::is_number(&word, tk.is_head) {
            let ps = self.get_number(&word, tk.currency, tk.is_head, tk.num_flags)?;
            return Ok(ps.map(|ps| apply_stress(&ps, tk.stress)));
        }
        Ok(None)
    }
}

fn clen(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.chars().count() }
}

fn drop_last_chars(s: &str, n: usize) -> &str {
    s.char_indices().rev().nth(n - 1).map_or("", |(i, _)| &s[..i])
}

fn cardinal_char(c: char) -> Result<String, num2words::Error> {
    let mut buf = [0; 4];
    num2words::cardinal(c.encode_utf8(&mut buf))
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
        assert_eq!(
            split_words("one hundred and twenty-one").collect::<Vec<_>>(),
            ["one", "hundred", "and", "twenty", "one"]
        );
        assert_eq!(split_words(",a b,").collect::<Vec<_>>(), ["", "a", "b", ""]);
        assert_eq!(number_suffix("1980's"), "'s");
        assert_eq!(number_suffix("12"), "");
        assert!(doubled_before_ing("running") && doubled_before_ing("kicking") && !doubled_before_ing("ring"));
    }
}
