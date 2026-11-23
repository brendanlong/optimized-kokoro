//! Names read in their own language (`data/names.json`, built by `tools/names/build.py`), in text of
//! any language: each name, exactly as written and standing as a word of its own, becomes a
//! `[Minas Tirith](/ˈminas ˈtiriθ/)` link, so every G2P reads it as the link says.

use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use unicode_normalization::UnicodeNormalization;

static NAMES: &str = include_str!("../data/names.json");
static SKIP: &str = include_str!("../data/names_skip.json");

pub(crate) struct Names {
    phonemes: FxHashMap<Box<str>, &'static str>,
    /// For each language whose text espeak reads (`es`, `fr`, ...), the names that are common words
    /// in it ("Este" in Portuguese), which its pipeline reads as words.
    skip: FxHashMap<&'static str, FxHashSet<&'static str>>,
    /// Every prefix of a name, so a match can stop as soon as the text leaves them.
    prefixes: FxHashSet<Box<str>>,
}

/// A Latin letter: other scripts end a name's word, as Japanese and Chinese write a Latin word
/// without spaces around it ("私はGaladrielです").
fn is_latin(c: char) -> bool {
    c.is_alphabetic()
        && matches!(c as u32, 0..=0x24f | 0x1e00..=0x1eff | 0x2c60..=0x2c7f | 0xa720..=0xa7ff | 0xab30..=0xab6f | 0xff21..=0xff5a)
}

fn is_word_char(c: char) -> bool {
    is_latin(c) || c.is_numeric() || unicode_normalization::char::is_combining_mark(c)
}

/// Whether a name may end where `rest` starts: not inside a word, nor before a contraction
/// ("Sauron'll" is one word), though "'s" is the possessive the caller takes.
fn ends_word(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if is_word_char(c) || c == ']' => false,
        Some('\'' | '’') => match chars.next() {
            Some('s') => !chars.next().is_some_and(is_word_char),
            Some(c) => !is_word_char(c),
            None => true,
        },
        _ => true,
    }
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
        let skip: FxHashMap<&'static str, Vec<&'static str>> =
            serde_json::from_str(SKIP).expect("bundled name skips are valid");
        let skip = skip.into_iter().map(|(l, names)| (l, names.into_iter().collect())).collect();
        Names { phonemes, prefixes, skip }
    }

    /// The longest name at the start of `text` that `skip` doesn't hold, as `(bytes of text,
    /// phonemes)`. Whitespace between a name's words may be any run of it but a line break.
    fn longest(&self, text: &str, skip: Option<&FxHashSet<&'static str>>) -> Option<(usize, &'static str)> {
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
                && ends_word(&text[end..])
                && !skip.is_some_and(|s| s.contains(text[..end].nfc().collect::<String>().as_str()))
            {
                found = Some((end, p));
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
            // Every name starts with a capital (`names_start_with_a_capital`).
            let found = if starts_word && c.is_uppercase() { self.longest(&text[i..], skip) } else { None };
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
    #[test]
    fn names_start_with_a_capital() {
        let names = super::Names::new();
        assert!(names.phonemes.keys().all(|n| n.chars().next().is_some_and(char::is_uppercase)));
    }
}
