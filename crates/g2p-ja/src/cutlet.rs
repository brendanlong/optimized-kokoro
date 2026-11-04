//! misaki's copy of cutlet (`misaki/cutlet.py`): text normalization, MeCab's words grouped by
//! `ja_words.txt`, and kana to phonemes.

use crate::dict::Dictionary;
use crate::mecab::{self, Morph};
use crate::num2kana;
use crate::tables::{DECIMALS, DIGITS, HEPBURN, KATA2HIRA};
use kokoro_text::py;
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;

/// MeCab's character categories in UniDic's `char.def`, as cutlet uses them.
const SYMBOL: u8 = 3;
const HIRAGANA: u8 = 6;
const KATAKANA: u8 = 7;
/// Not a MeCab category: a number read straight into phonemes ([`Part::Number`]).
const NUMBER: u8 = u8::MAX;

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let cp = c as u32;
    let i = table.partition_point(|&(_, hi)| hi < cp);
    table.get(i).is_some_and(|&(lo, _)| lo <= cp)
}

/// `c.isdecimal()`: Python `re`'s `\d`.
pub fn is_decimal(c: char) -> bool {
    in_ranges(DECIMALS, c)
}

/// The digit value of a character for which `c.isdigit()` holds.
pub fn digit_value(c: char) -> Option<u8> {
    let cp = c as u32;
    let i = DIGITS.partition_point(|&(_, hi, _)| hi < cp);
    DIGITS.get(i).filter(|&&(lo, _, _)| lo <= cp).map(|&(lo, _, v)| v + (cp - lo) as u8)
}

/// `str.isdigit()`
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| digit_value(c).is_some())
}

fn table(key: &str) -> Option<&'static str> {
    HEPBURN.binary_search_by(|(k, _)| k.cmp(&key)).ok().map(|i| HEPBURN[i].1)
}

fn table_char(c: char) -> Option<&'static str> {
    let mut buf = [0; 4];
    table(c.encode_utf8(&mut buf))
}

/// `jaconv.kata2hira`
pub fn kata2hira(s: &str) -> String {
    s.chars().map(|c| KATA2HIRA.binary_search_by(|(k, _)| k.cmp(&c)).map_or(c, |i| KATA2HIRA[i].1)).collect()
}

/// `misaki/data/ja_words.txt`, which cutlet joins MeCab's words by.
pub struct Words {
    set: HashSet<String>,
    /// The longest entry, in characters.
    longest: usize,
}

impl Words {
    pub fn bundled() -> Words {
        let set: HashSet<String> =
            include_str!("../data/ja_words.txt").lines().map(|l| py::strip(l).to_owned()).collect();
        let longest = set.iter().map(|w| w.chars().count()).max().unwrap_or(0);
        Words { set, longest }
    }
}

/// `Katakana_Phonetic_Extensions`: small katakana ㇰ–ㇿ as their full-size kana.
fn phonetic_extension(c: char) -> Option<char> {
    const FULL: [char; 16] =
        ['ク', 'シ', 'ス', 'ト', 'ヌ', 'ハ', 'ヒ', 'フ', 'ヘ', 'ホ', 'ム', 'ラ', 'リ', 'ル', 'レ', 'ロ'];
    matches!(c, '\u{31f0}'..='\u{31ff}').then(|| FULL[c as usize - 0x31f0])
}

/// A piece of normalized text: text for MeCab, or a number the reference can't read (where it
/// raises), already in phonemes, which MeCab never sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// The number's digits (ASCII) and its phonemes.
    Number(String, String),
}

/// `Cutlet._normalize_text`, with digits read in kana. The reference reads each run of digits
/// with `Convert`, which fails on numbers of more than nine digits and on digits other than
/// ASCII's after NFKC (misaki then crashes); those are read by [`num2kana::number_words`] instead,
/// straight into phonemes. NULs, past which MeCab reads nothing, are dropped.
pub fn normalize(text: &str) -> Vec<Part> {
    let chars: Vec<char> = text.chars().filter(|&c| c != '\0').collect();
    let mut wave = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if matches!(c, '〜' | '～') && chars.get(i + 1).is_some_and(|&n| is_decimal(n)) {
            wave.push_str("から");
        } else {
            wave.push(phonetic_extension(c).unwrap_or(c));
        }
    }
    let nfkc: String = wave.nfkc().collect();
    // mojimoji.zen_to_han(kana=False); its other mappings are characters NFKC has already
    // replaced, and han_to_zen(digit=False, ascii=False) only maps half-width kana, which NFKC
    // has made full-width.
    let mut parts = Vec::new();
    let mut out = String::with_capacity(nfkc.len() + 16);
    let mut segment = String::new();
    let mut digits = false;
    let flush = |segment: &mut String, digits: bool, out: &mut String, parts: &mut Vec<Part>| {
        if segment.is_empty() {
            return;
        }
        if digits || is_digits(segment) {
            let ascii: Vec<u8> = segment.chars().map(|c| b'0' + digit_value(c).expect("a digit")).collect();
            out.push(' ');
            if segment.is_ascii() && ascii.len() <= 9 {
                out.push_str(&num2kana::convert_kana(&ascii));
            } else {
                parts.push(Part::Text(std::mem::take(out)));
                let words: Vec<String> =
                    num2kana::number_words(&ascii).iter().map(|kana| kana_phonemes(kana)).collect();
                parts.push(Part::Number(String::from_utf8(ascii).expect("ASCII"), words.join(" ")));
            }
        } else {
            out.push_str(segment);
        }
        segment.clear();
    };
    for c in nfkc.chars() {
        let c = match c {
            '\u{2018}' => '`',
            '\u{2019}' => '\'',
            '\u{201d}' => '"',
            c => c,
        };
        let d = is_decimal(c);
        if d != digits {
            flush(&mut segment, digits, &mut out, &mut parts);
            digits = d;
        }
        segment.push(c);
    }
    flush(&mut segment, digits, &mut out, &mut parts);
    parts.push(Part::Text(out));
    parts.retain(|p| !matches!(p, Part::Text(t) if t.is_empty()));
    parts
}

/// A number word's kana in phonemes, as `_romaji_word` reads a dictionary word, with the long
/// vowels MeCab's readings (`pron`) would have: きゅう and ちょう as きゅー and ちょー.
fn kana_phonemes(kana: &str) -> String {
    let hira = kata2hira(kana).replace("ゅう", "ゅー").replace("ょう", "ょー");
    romaji_word(&Word { surface: hira.clone(), hira, char_type: HIRAGANA })
}

struct Word {
    surface: String,
    hira: String,
    char_type: u8,
}

fn add_dakuten(k: char) -> Option<char> {
    const PLAIN: &str = "かきくけこさしすせそたちつてとはひふへほ";
    const VOICED: &str = "がぎぐげござじずぜぞだぢづでどばびぶべぼ";
    PLAIN.chars().position(|c| c == k).and_then(|i| VOICED.chars().nth(i))
}

fn is_sutegana(c: char) -> bool {
    "ゃゅょぁぃぅぇぉ".contains(c)
}

/// `Cutlet._get_single_mapping`
fn single_mapping(pk: Option<char>, kk: char, nk: Option<char>, out: &mut String) {
    if "〃々ゝゞヽ".contains(kk) {
        if "ゝヽ".contains(kk) {
            if let Some(pk) = pk {
                out.push(pk);
            }
        } else if "ゞヾ".contains(kk)
            && let Some(v) = pk.and_then(add_dakuten)
        {
            out.push_str(table_char(v).expect("voiced kana are in the table"));
        }
        return;
    }
    let pair = |a: char, b: char| {
        let mut buf = [0u8; 8];
        let n = a.encode_utf8(&mut buf).len();
        let m = b.encode_utf8(&mut buf[n..]).len();
        table(std::str::from_utf8(&buf[..n + m]).expect("two chars"))
    };
    if let Some(pk) = pk
        && let Some(v) = pair(pk, kk)
    {
        out.push_str(v);
        return;
    }
    if let Some(nk) = nk {
        if pair(kk, nk).is_some() {
            return;
        }
        if is_sutegana(nk) {
            if kk == 'っ' {
                return;
            }
            // The reference raises KeyError when `kk` isn't in the table (`ーゃ`, say).
            if let Some(v) = table_char(kk) {
                let mut cs = v.chars();
                cs.next_back();
                out.push_str(cs.as_str());
            }
            out.push_str(table_char(nk).expect("small kana are in the table"));
            return;
        }
    }
    if is_sutegana(kk) {
        return;
    }
    match kk {
        'ー' => out.push('ː'),
        'っ' => out.push('ʔ'),
        'ん' => {
            let tnk = nk.and_then(table_char).filter(|t| !t.is_empty());
            let n = match tnk {
                Some(t) if t.starts_with(['m', 'p', 'b']) => 'm',
                Some(t) if t.starts_with(['k', 'ɡ']) => 'ŋ',
                Some(t) if t.starts_with(['ɲ', 'ʨ', 'ʥ']) => 'ɲ',
                Some(t) if t.starts_with(['n', 't', 'd', 'ɾ', 'z']) => 'n',
                _ => 'ɴ',
            };
            out.push(n);
        }
        _ => out.push_str(table_char(kk).unwrap_or("")),
    }
}

/// `Cutlet._romaji_word`. A surface of digits, which the reference asserts can't happen (it
/// does, for digits NFKC leaves alone, such as `❶`), is read as a number.
fn romaji_word(word: &Word) -> String {
    if word.char_type == NUMBER {
        return word.hira.clone();
    }
    let surface = word.surface.as_str();
    if is_digits(surface) {
        let ascii: Vec<u8> = surface.chars().map(|c| b'0' + digit_value(c).expect("a digit")).collect();
        return num2kana::number_words(&ascii).iter().map(|kana| kana_phonemes(kana)).collect::<Vec<_>>().join(" ");
    }
    if surface.is_ascii() {
        return surface.to_owned();
    }
    let (hira, char_type) = (word.hira.as_str(), word.char_type);
    if char_type == SYMBOL {
        return surface.chars().map(|c| table_char(c).map_or_else(|| c.to_string(), str::to_owned)).collect();
    } else if char_type != HIRAGANA {
        return String::new();
    }
    let chars: Vec<char> = hira.chars().collect();
    let mut out = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let pk = i.checked_sub(1).map(|j| chars[j]);
        single_mapping(pk, c, chars.get(i + 1).copied(), &mut out);
    }
    out
}

/// `surface in s` for Python strings: a substring test (true for an empty `surface`).
fn within(surface: &str, s: &str) -> bool {
    s.contains(surface)
}

/// `Cutlet._romaji_tokens`, joined as `__call__` joins them.
fn romaji(words: Vec<Word>, ja_words: &Words) -> String {
    // run_end[i]: the first word after i of another char_type (the reference's `z`).
    let mut run_end = vec![words.len(); words.len()];
    for i in (0..words.len().saturating_sub(1)).rev() {
        run_end[i] = if words[i + 1].char_type != words[i].char_type { i + 1 } else { run_end[i + 1] };
    }
    let mut groups: Vec<Word> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let z = run_end[i];
        // The longest join of words[i..j] (j > i, j <= z) in ja_words.txt; joins longer than its
        // longest entry can't be in it.
        let mut chars = 0;
        let mut top = i;
        while top < z {
            chars += words[top].surface.chars().count();
            if chars > ja_words.longest {
                break;
            }
            top += 1;
        }
        let mut found = None;
        let mut joined = String::new();
        let mut candidates = Vec::new();
        for w in &words[i..top] {
            joined.push_str(&w.surface);
            candidates.push(joined.len());
        }
        for j in (i + 1..=top).rev() {
            if ja_words.set.contains(&joined[..candidates[j - i - 1]]) {
                found = Some(j);
                break;
            }
        }
        let j = found.unwrap_or(i + 1);
        let group = &words[i..j];
        groups.push(Word {
            surface: group.iter().map(|w| w.surface.as_str()).collect(),
            hira: group.iter().map(|w| w.hira.as_str()).collect(),
            char_type: group[0].char_type,
        });
        i = j;
    }
    let mut out: Vec<(String, bool)> = Vec::with_capacity(groups.len());
    for word in &groups {
        let roma = romaji_word(word);
        let surface = word.surface.as_str();
        let space;
        if within(surface, "「『«") || within(&roma, "([") {
            if let Some(po) = out.last_mut() {
                po.1 = true;
            }
            space = false;
        } else if within(surface, "」』»") || within(&roma, "]).,?!:") {
            if let Some(po) = out.last_mut() {
                po.1 = false;
            }
            space = true;
        } else {
            space = roma != " ";
        }
        out.push((roma, space));
    }
    let mut s = String::new();
    for (roma, space) in out {
        s.extend(roma.chars().filter(|&c| c != 'っ'));
        if space {
            s.push(' ');
        }
    }
    s
}

/// `re.sub(r'\s+', ' ', s.strip())`, then `(` and `)` as `«` and `»`, then
/// `re.sub(r'(?<![!",.:;?»—…”]) (?=ʔ)|(?<=ʔ) (?!["«“])', '', ps)`.
fn finish(s: &str) -> String {
    let mut collapsed: Vec<char> = Vec::with_capacity(s.len());
    let mut in_space = false;
    for c in py::strip(s).chars() {
        if py::is_space(c) {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            in_space = false;
            collapsed.push(match c {
                '(' => '«',
                ')' => '»',
                c => c,
            });
        }
    }
    let mut out = String::with_capacity(s.len());
    for (i, &c) in collapsed.iter().enumerate() {
        if c == ' ' {
            let prev = i.checked_sub(1).map(|j| collapsed[j]);
            let next = collapsed.get(i + 1).copied();
            let before_stop = next == Some('ʔ') && !prev.is_some_and(|p| "!\",.:;?»—…”".contains(p));
            let after_stop = prev == Some('ʔ') && !next.is_some_and(|n| "\"«“".contains(n));
            if before_stop || after_stop {
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Longest run of one whitespace character MeCab is given.
const MAX_SPACES: usize = 4096;

/// Runs of more than [`MAX_SPACES`] of one character MeCab skips as whitespace, cut to that many.
/// MeCab looks at most 65,535 bytes ahead of a node, so past that it cuts the next word short,
/// or inside a character, which crashes fugashi; below it, the run's length changes nothing.
fn shorten_spaces(dict: &Dictionary, text: String) -> String {
    let space = dict.char_info(0x20);
    let is_space = |c: char| (c as u32) < 0x10000 && dict.char_info(c as u32).is_kind_of(space);
    let mut longest = 0;
    let mut run = 0;
    let mut last = None;
    for c in text.chars() {
        run = if Some(c) == last { run + 1 } else { 1 };
        last = Some(c);
        if is_space(c) {
            longest = longest.max(run);
        }
    }
    if longest <= MAX_SPACES {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut run = 0;
    let mut last = None;
    for c in text.chars() {
        run = if Some(c) == last { run + 1 } else { 1 };
        last = Some(c);
        if run <= MAX_SPACES || !is_space(c) {
            out.push(c);
        }
    }
    out
}

/// `Cutlet.__call__(text)[0]`, which is `JAG2P()(text)[0]`.
pub fn phonemize(dict: &Dictionary, ja_words: &Words, text: &str) -> Result<String, mecab::Error> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let mut words = Vec::new();
    for part in normalize(text) {
        match part {
            Part::Text(text) => {
                let text = shorten_spaces(dict, text);
                words.extend(mecab::analyze(dict, &text)?.into_iter().map(
                    |Morph { surface, reading, char_type, unknown }| Word {
                        surface: surface.to_owned(),
                        hira: reading.map_or_else(|| kata2hira(surface), str::to_owned),
                        char_type: if char_type == KATAKANA || !unknown { HIRAGANA } else { char_type },
                    },
                ));
            }
            Part::Number(digits, phonemes) => words.push(Word { surface: digits, hira: phonemes, char_type: NUMBER }),
        }
    }
    Ok(finish(&romaji(words, ja_words)))
}
