//! Names read in their own language (`data/names.json`, built by `tools/names/build.py`), in text of
//! any language: each name standing as a word of its own, in any case and with or without its
//! accents, becomes a `[Minas Tirith](/ˈminas ˈtiriθ/)` link, so every G2P reads it as the link says.

use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

static NAMES: &str = include_str!("../data/names.json");
static SKIP: &str = include_str!("../data/names_skip.json");

struct Name {
    spelling: &'static str,
    /// None for a spelling that finds a name but isn't it ("Dior" for "Díor").
    phonemes: Option<&'static str>,
}

pub(crate) struct Names {
    /// The names by [`fold`]ed spelling.
    names: FxHashMap<Box<str>, Box<[Name]>>,
    /// For each language whose text espeak reads (`es`, `fr`, ...), the names that are common words
    /// in it ("Este" in Portuguese), which its pipeline reads as words, [`fold`]ed.
    skip: FxHashMap<&'static str, FxHashSet<Box<str>>>,
    /// Every prefix of a folded name, so a match can stop as soon as the text leaves them.
    prefixes: FxHashSet<Box<str>>,
}

/// `c` lowercase and without accents, as a name's key spells it.
fn fold(c: char, out: &mut String) {
    for d in std::iter::once(c).nfd().filter(|&d| !is_combining_mark(d)) {
        out.extend(d.to_lowercase());
    }
}

/// Whether `written` is `name` in any case, with some of the name's accents left out and no
/// others added.
fn accents_within(written: &str, name: &str) -> bool {
    let clusters = |s: &str| {
        let mut out: Vec<(String, Vec<char>)> = Vec::new();
        for c in s.nfd() {
            match out.last_mut() {
                Some((_, marks)) if is_combining_mark(c) => marks.push(c),
                _ => out.push((c.to_lowercase().collect(), Vec::new())),
            }
        }
        out
    };
    let (w, n) = (clusters(written), clusters(name));
    w.len() == n.len() && w.iter().zip(&n).all(|(w, n)| w.0 == n.0 && (w.1.is_empty() || w.1 == n.1))
}

fn folded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    s.chars().for_each(|c| fold(c, &mut out));
    out
}

/// A Latin letter: other scripts end a name's word, as Japanese and Chinese write a Latin word
/// without spaces around it ("私はGaladrielです").
fn is_latin(c: char) -> bool {
    c.is_alphabetic()
        && matches!(c as u32, 0..=0x24f | 0x1e00..=0x1eff | 0x2c60..=0x2c7f | 0xa720..=0xa7ff | 0xab30..=0xab6f | 0xff21..=0xff5a)
}

fn is_word_char(c: char) -> bool {
    is_latin(c) || c.is_numeric() || is_combining_mark(c)
}

/// Whether a name may end where `rest` starts: not inside a word, nor before a contraction
/// ("Sauron'll" is one word), though "'s" is the possessive the caller takes.
fn ends_word(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if is_word_char(c) || c == ']' => false,
        Some('\'' | '’') => match chars.next() {
            Some('s' | 'S') => !chars.next().is_some_and(is_word_char),
            Some(c) => !is_word_char(c),
            None => true,
        },
        _ => true,
    }
}

impl Names {
    pub(crate) fn new() -> Self {
        let list: Vec<(&'static str, Option<&'static str>)> =
            serde_json::from_str::<FxHashMap<_, _>>(NAMES).expect("bundled names are valid").into_iter().collect();
        let mut names: FxHashMap<Box<str>, Vec<Name>> = FxHashMap::default();
        for (spelling, phonemes) in list {
            names.entry(folded(spelling).into_boxed_str()).or_default().push(Name { spelling, phonemes });
        }
        let mut prefixes = FxHashSet::default();
        for name in names.keys() {
            prefixes.extend(name.char_indices().map(|(i, _)| Box::from(&name[..i])).skip(1));
            prefixes.insert(name.clone());
        }
        let skip: FxHashMap<&'static str, Vec<&'static str>> =
            serde_json::from_str(SKIP).expect("bundled name skips are valid");
        let skip =
            skip.into_iter().map(|(l, names)| (l, names.into_iter().map(|n| folded(n).into()).collect())).collect();
        let names = names.into_iter().map(|(k, v)| (k, v.into_boxed_slice())).collect();
        Names { names, prefixes, skip }
    }

    /// The phonemes of the name `written` spells: the one spelled so, else the reading the names
    /// spelled so in another case share, else the one every name whose accents `written` leaves
    /// out, or keeps, shares. An accent the name doesn't have makes another word: Spanish "moría"
    /// isn't Moria, nor Portuguese "âmbar" Ambar.
    fn resolve(names: &[Name], written: &str) -> Option<&'static str> {
        let written: String = written.split_whitespace().collect::<Vec<_>>().join(" ").nfc().collect();
        if let Some(name) = names.iter().find(|n| n.spelling == written) {
            return name.phonemes;
        }
        let one = |mut names: Vec<&Name>| {
            let first = names.pop()?;
            names.iter().all(|n| n.phonemes == first.phonemes).then_some(first.phonemes)?
        };
        let lower = written.to_lowercase();
        let same_case: Vec<_> = names.iter().filter(|n| n.spelling.to_lowercase() == lower).collect();
        if !same_case.is_empty() {
            return one(same_case);
        }
        one(names.iter().filter(|n| accents_within(&written, n.spelling)).collect())
    }

    /// The longest name at the start of `text` that `skip` doesn't hold, as `(bytes of text,
    /// phonemes)`. Whitespace between a name's words may be any run of it but a line break.
    fn longest(&self, text: &str, skip: Option<&FxHashSet<Box<str>>>) -> Option<(usize, &'static str)> {
        let mut key = String::new();
        let mut found = None;
        let mut chars = text.char_indices().peekable();
        while let Some((_, c)) = chars.next() {
            if c.is_whitespace() {
                if c == '\n' || key.ends_with(' ') {
                    break;
                }
                key.push(' ');
                while chars.next_if(|&(_, c)| c.is_whitespace() && c != '\n').is_some() {}
            } else {
                fold(c, &mut key);
            }
            if !self.prefixes.contains(key.as_str()) {
                break;
            }
            let end = chars.peek().map_or(text.len(), |&(j, _)| j);
            if let Some(names) = self.names.get(key.as_str())
                && ends_word(&text[end..])
                && !skip.is_some_and(|s| s.contains(key.as_str()))
                && let Some(phonemes) = Self::resolve(names, &text[..end])
            {
                found = Some((end, phonemes));
            }
        }
        found
    }

    /// `text` with each name made a link, outside the links it has, but for the names that are
    /// common words in `language`. A `[` not yet closed before a name is left out, as the link
    /// would close it (the model has no sound for it). "'s" after a name joins it, with `possessive` adding misaki's
    /// ending to its phonemes ("Sauron's").
    pub(crate) fn link<'t>(
        &self,
        text: &'t str,
        language: &str,
        links: &[(usize, usize)],
        possessive: impl Fn(&str) -> String,
    ) -> Cow<'t, str> {
        let skip = self.skip.get(language);
        let mut out = String::new();
        let mut copied = 0;
        let mut i = 0;
        let mut link = links.iter().peekable();
        let mut open: Option<usize> = None;
        while i < text.len() {
            while link.next_if(|&&(_, end)| end <= i).is_some() {}
            if let Some(&&(start, end)) = link.peek()
                && start <= i
            {
                i = end;
                continue;
            }
            let c = text[i..].chars().next().expect("in the text");
            match c {
                '[' => open = Some(i),
                ']' => open = None,
                _ => {}
            }
            let starts_word = !text[..i].chars().next_back().is_some_and(is_word_char);
            let found = if starts_word && is_latin(c) { self.longest(&text[i..], skip) } else { None };
            let Some((n, phonemes)) = found.filter(|&(n, _)| link.peek().is_none_or(|&&(start, _)| i + n <= start))
            else {
                i += c.len_utf8();
                continue;
            };
            let mut end = i + n;
            let mut phonemes = Cow::Borrowed(phonemes);
            for s in ["'s", "’s", "'S", "’S"] {
                if text[end..].starts_with(s) && !text[end + s.len()..].chars().next().is_some_and(is_word_char) {
                    phonemes = Cow::Owned(possessive(&phonemes));
                    end += s.len();
                    break;
                }
            }
            if let Some(bracket) = open.take() {
                out.push_str(&text[copied..bracket]);
                copied = bracket + 1;
            }
            out.push_str(&text[copied..i]);
            out.push('[');
            out.push_str(&text[i..end]);
            out.push_str("](/");
            out.push_str(&phonemes);
            out.push_str("/)");
            copied = end;
            i = end;
        }
        if copied == 0 {
            return Cow::Borrowed(text);
        }
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::Names;

    #[test]
    fn names_are_found_in_any_case_and_without_their_accents() {
        let names = Names::new();
        let link = |t: &str| names.link(t, "en", &[], |p| format!("{p}z")).into_owned();
        let galadriel = link("Galadriel");
        assert!(galadriel.starts_with("[Galadriel](/"));
        for t in ["GALADRIEL", "galadriel"] {
            assert_eq!(link(t), galadriel.replace("Galadriel]", &format!("{t}]")));
        }
        assert_eq!(link("Eärendil"), link("Earendil").replace("Earendil]", "Eärendil]"));
        assert_eq!(link("Ea\u{308}rendil"), link("Earendil").replace("Earendil]", "Ea\u{308}rendil]"));
        assert_eq!(link("EÄRENDIL"), link("Earendil").replace("Earendil]", "EÄRENDIL]"));
        // Tolkien wrote both Círdan and Cirdan, said differently: each is read as written, and a
        // spelling with accents of neither is left alone.
        assert_ne!(link("CÍRDAN").replace("CÍRDAN", "Círdan"), link("CIRDAN").replace("CIRDAN", "Círdan"));
        assert_eq!(link("Cîrdan"), "Cîrdan");
        // Accents the name doesn't have make another word.
        let spanish = |t: &str| names.link(t, "es", &[], |p| format!("{p}z")).into_owned();
        assert!(spanish("Moria").starts_with("[Moria](/"));
        assert_eq!(spanish("se moría"), "se moría");
        assert_eq!(link("âmbar"), "âmbar");
        // An English word isn't the name it spells without accents.
        assert_eq!(link("Turin"), "Turin");
        assert_eq!(link("TURIN"), "TURIN");
        assert!(link("Túrin").starts_with("[Túrin](/"));
        assert_eq!(link("GALADRIEL'S"), link("Galadriel's").replace("Galadriel's]", "GALADRIEL'S]"));
        // Nor is a name a common word of the text's language, in any case.
        let spanish = |t: &str| names.link(t, "es", &[], |p| format!("{p}z")).into_owned();
        assert_eq!(spanish("fui y ESTE"), "fui y ESTE");
    }
}
