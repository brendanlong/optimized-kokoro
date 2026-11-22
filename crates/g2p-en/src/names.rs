//! Names read in their own language (`data/names.json`, built by `tools/names/build.py`), in text of
//! any language: each name, exactly as written and standing as a word of its own, becomes a
//! `[Minas Tirith](/ˈminas ˈtiriθ/)` link, so every G2P reads it as the link says.

use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use unicode_normalization::UnicodeNormalization;

static NAMES: &str = include_str!("../data/names.json");

pub(crate) struct Names {
    phonemes: FxHashMap<Box<str>, &'static str>,
    /// Every prefix of a name, so a match can stop as soon as the text leaves them.
    prefixes: FxHashSet<Box<str>>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || unicode_normalization::char::is_combining_mark(c)
}

impl Names {
    pub(crate) fn new() -> Self {
        let names: FxHashMap<&'static str, &'static str> =
            serde_json::from_str(NAMES).expect("bundled names are valid");
        let mut phonemes = FxHashMap::default();
        for (name, p) in names {
            // Text may write an accent as a separate combining mark ("Eärendil" as e, a, U+0308, ...).
            if !name.is_ascii() {
                phonemes.entry(name.nfd().collect::<String>().into_boxed_str()).or_insert(p);
            }
            phonemes.insert(Box::from(name), p);
        }
        let mut prefixes = FxHashSet::default();
        for name in phonemes.keys() {
            prefixes.extend(name.char_indices().map(|(i, _)| Box::from(&name[..i])).skip(1));
            prefixes.insert(name.clone());
        }
        Names { phonemes, prefixes }
    }

    /// The longest name at the start of `text`, as `(bytes of text, phonemes)`. Whitespace between
    /// a name's words may be any run of it but a line break.
    fn longest(&self, text: &str) -> Option<(usize, &'static str)> {
        let mut spelled = String::new();
        let mut found = None;
        let mut chars = text.char_indices().peekable();
        while let Some((_, c)) = chars.next() {
            if c.is_whitespace() {
                if c == '\n' || spelled.ends_with(' ') {
                    break;
                }
                spelled.push(' ');
                while chars.next_if(|&(_, c)| c.is_whitespace() && c != '\n').is_some() {}
            } else {
                spelled.push(c);
            }
            if !self.prefixes.contains(spelled.as_str()) {
                break;
            }
            let end = chars.peek().map_or(text.len(), |&(j, _)| j);
            if let Some(&p) = self.phonemes.get(spelled.as_str())
                && !text[end..].chars().next().is_some_and(is_word_char)
            {
                found = Some((end, p));
            }
        }
        found
    }

    /// `text` with each name made a link, outside the links it has. "'s" after a name joins it,
    /// with `possessive` adding misaki's ending to its phonemes ("Sauron's").
    pub(crate) fn link<'t>(
        &self,
        text: &'t str,
        links: &[(usize, usize)],
        possessive: impl Fn(&str) -> String,
    ) -> Cow<'t, str> {
        let mut out = String::new();
        let mut copied = 0;
        let mut i = 0;
        let mut link = links.iter().peekable();
        while i < text.len() {
            while link.next_if(|&&(_, end)| end <= i).is_some() {}
            if let Some(&&(start, end)) = link.peek()
                && start <= i
            {
                i = end;
                continue;
            }
            let c = text[i..].chars().next().expect("in the text");
            let starts_word = !text[..i].chars().next_back().is_some_and(is_word_char);
            let found = if starts_word && c.is_alphabetic() { self.longest(&text[i..]) } else { None };
            let Some((n, phonemes)) = found.filter(|&(n, _)| link.peek().is_none_or(|&&(start, _)| i + n <= start))
            else {
                i += c.len_utf8();
                continue;
            };
            let mut end = i + n;
            let mut phonemes = Cow::Borrowed(phonemes);
            for s in ["'s", "’s"] {
                if text[end..].starts_with(s) && !text[end + s.len()..].chars().next().is_some_and(is_word_char) {
                    phonemes = Cow::Owned(possessive(&phonemes));
                    end += s.len();
                    break;
                }
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
