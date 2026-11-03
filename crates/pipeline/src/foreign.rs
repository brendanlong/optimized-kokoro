//! kokoro's `foreign.py` (brendanlong/kokoro at the commit Kokoro-FastAPI pins) for the English
//! pipelines: finds runs of other scripts so each is read by its own language's G2P, in the
//! English voice, and post-processes their phonemes.
//!
//! `foreign.py` looks for six scripts besides Han and kana; here every non-Latin script makes
//! runs, read by the espeak voice espeak-ng itself switches to for that alphabet
//! ([`kokoro_espeak::alphabet_language`]). The six keep `foreign.py`'s voices.

pub use crate::tables::Script;
use crate::tables::{NONSPACING_MARKS, SCRIPTS};
use kokoro_text::py;
use std::cmp::Ordering;

/// A language other than the pipeline's, which reads a run of its script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Chinese,
    Japanese,
    /// Read by espeak, with the voice for this language code.
    Espeak(&'static str),
    /// A script espeak has no voice for.
    Unvoiced(Script),
}

impl Language {
    /// The reference's language code (`zh`, `ja`, or the espeak voice's), or the script's name.
    pub fn code(self) -> &'static str {
        match self {
            Language::Chinese => "zh",
            Language::Japanese => "ja",
            Language::Espeak(code) => code,
            Language::Unvoiced(script) => script.name(),
        }
    }

    /// The espeak voice that reads this language: none for Chinese and Japanese, which have G2Ps
    /// of their own, or for a script espeak doesn't read.
    pub fn espeak(self) -> Option<&'static str> {
        match self {
            Language::Espeak(code) => Some(code),
            _ => None,
        }
    }

    /// The English name a run is read as when there's no G2P for it.
    pub fn name(self) -> &'static str {
        match self {
            Language::Chinese => "Chinese",
            Language::Japanese => "Japanese",
            Language::Espeak(code) => code,
            Language::Unvoiced(script) => script.name(),
        }
    }

    /// The language that reads a run of `script` starting with `first`, or `None` for Latin,
    /// which is the pipeline's own.
    fn of_run(script: Script, first: char) -> Option<Language> {
        Some(match script {
            Script::Latin => return None,
            Script::Arabic => Language::Espeak("ar"),
            Script::Cyrillic => Language::Espeak("ru"),
            Script::Devanagari => Language::Espeak("hi"),
            Script::Greek => Language::Espeak("el"),
            Script::Hangul => Language::Espeak("ko"),
            Script::Hebrew => Language::Espeak("he"),
            _ => kokoro_espeak::alphabet_language(first).map_or(Language::Unvoiced(script), Language::Espeak),
        })
    }
}

/// Every espeak voice a run may go to.
pub fn espeak_voices() -> Vec<&'static str> {
    let mut voices: Vec<&'static str> = SCRIPTS
        .iter()
        .flat_map(|&(lo, hi, script)| (lo..=hi).filter_map(move |c| Language::of_run(script, c)?.espeak()))
        .collect();
    voices.sort_unstable();
    voices.dedup();
    voices
}

fn in_ranges(table: &[(char, char)], c: char) -> bool {
    table.binary_search_by(|&(lo, hi)| order(lo, hi, c)).is_ok()
}

fn order(lo: char, hi: char, c: char) -> Ordering {
    if hi < c {
        Ordering::Less
    } else if lo > c {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// `\p{Script}`, for every script but Common, Inherited and Unknown.
pub fn script(c: char) -> Option<Script> {
    SCRIPTS.binary_search_by(|&(lo, hi, _)| order(lo, hi, c)).ok().map(|i| SCRIPTS[i].2)
}

fn is_mark(c: char) -> bool {
    in_ranges(NONSPACING_MARKS, c)
}

fn is_kana(c: char) -> bool {
    matches!(script(c), Some(Script::Hiragana | Script::Katakana))
}

fn is_cjk(c: char) -> bool {
    c == 'ー' || matches!(script(c), Some(Script::Han | Script::Hiragana | Script::Katakana))
}

/// `[ '’\-]`: what may join two words of one run.
pub fn is_joiner(c: char) -> bool {
    matches!(c, ' ' | '\'' | '’' | '-')
}

/// `(?:\p{S}\p{Mn}*)+` (three or more letters for Greek) at `start`: the end of the word, if any.
fn word_end(text: &str, start: usize, s: Script) -> Option<usize> {
    let mut letters = 0;
    let mut end = start;
    for c in text[start..].chars() {
        if script(c) == Some(s) {
            letters += 1;
        } else if letters == 0 || !is_mark(c) {
            break;
        }
        end += c.len_utf8();
    }
    let needed = if s == Script::Greek { 3 } else { 1 };
    (letters >= needed).then_some(end)
}

/// A match of `RUN_REGEX`: a run of Han and kana (`None`) or of one script's words.
struct Run {
    start: usize,
    end: usize,
    script: Option<Script>,
}

/// `RUN_REGEX.finditer`: the alternatives start with disjoint sets of characters, so at each
/// position at most one can match, and each is greedy without backtracking past a word.
fn runs(text: &str) -> Vec<Run> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(c) = text[pos..].chars().next() {
        if is_cjk(c) {
            let end = text[pos..].find(|c: char| !(is_cjk(c) || c == '、')).map_or(text.len(), |n| pos + n);
            out.push(Run { start: pos, end, script: None });
            pos = end;
            continue;
        }
        let Some(s) = script(c).filter(|s| !matches!(s, Script::Han | Script::Hiragana | Script::Katakana)) else {
            pos += c.len_utf8();
            continue;
        };
        let Some(mut end) = word_end(text, pos, s) else {
            pos += c.len_utf8();
            continue;
        };
        loop {
            let after = text[end..].find(|c: char| !is_joiner(c)).map_or(text.len(), |n| end + n);
            match (after > end).then(|| word_end(text, after, s)).flatten() {
                Some(next) => end = next,
                None => break,
            }
        }
        out.push(Run { start: pos, end, script: Some(s) });
        pos = end;
    }
    out
}

/// misaki's `LINK_REGEX` spans, `[text](feature)`, which the English G2P reads itself.
fn links(text: &str) -> Vec<(usize, usize)> {
    static LINK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\[([^\]]+)\]\(([^\)]*)\)").unwrap());
    LINK.find_iter(text).map(|m| (m.start(), m.end())).collect()
}

/// `ForeignScripts.split` in an English pipeline: `(segment, language)` pairs covering `text`,
/// where `None` is English. `reads` says whether the Chinese G2P has a reading for a Han
/// character: the reference leaves those pypinyin can't read (some radicals, Japanese-only
/// kanji), and `ー` outside kana, to the English G2P.
pub fn split(text: &str, reads: impl Fn(char) -> bool) -> Vec<(&str, Option<Language>)> {
    let mut segments = Vec::new();
    let mut last = 0;
    let mut push = |start: usize, end: usize, language: Language| {
        if start > last {
            segments.push((&text[last..start], None));
        }
        segments.push((&text[start..end], Some(language)));
        last = end;
    };
    let links = links(text);
    let mut link = 0;
    for run in runs(text) {
        while link < links.len() && links[link].1 <= run.start {
            link += 1;
        }
        if links.get(link).is_some_and(|&(start, _)| start <= run.start) {
            continue;
        }
        let run_text = &text[run.start..run.end];
        let language = match run.script {
            None if run_text.chars().any(is_kana) => Language::Japanese,
            None => {
                let readable = |c: char| c == '、' || (script(c) == Some(Script::Han) && reads(c));
                let mut start = run.start;
                let mut chars = run_text.char_indices().peekable();
                while let Some((i, c)) = chars.next() {
                    let r = readable(c);
                    if chars.peek().is_none_or(|&(_, next)| readable(next) != r) {
                        let end = run.start + i + c.len_utf8();
                        if r {
                            push(start, end, Language::Chinese);
                        }
                        start = end;
                    }
                }
                continue;
            }
            Some(script) => match Language::of_run(script, run_text.chars().next().expect("runs aren't empty")) {
                Some(language) => language,
                None => continue,
            },
        };
        push(run.start, run.end, language);
    }
    if last < text.len() {
        segments.push((&text[last..], None));
    }
    segments
}

/// What goes to the language's G2P: Hebrew loses its vowel points, which espeak reads as
/// letter names.
pub fn prepare(segment: &str, language: Language) -> std::borrow::Cow<'_, str> {
    if language == Language::Espeak("he") && segment.chars().any(is_mark) {
        segment.chars().filter(|&c| !is_mark(c)).collect::<String>().into()
    } else {
        segment.into()
    }
}

/// The rest of `ForeignScripts.phonemize`: stripped, with sounds Kokoro has no phoneme for
/// mapped to the nearest one, and Mandarin tones and unaspirated stops made English.
pub fn finish(phonemes: &str, language: Language) -> String {
    let ps: String = py::strip(phonemes)
        .chars()
        .map(|c| match c {
            'ħ' => 'h',
            'ʕ' => 'ʔ',
            c => c,
        })
        .collect();
    if language == Language::Chinese { english_mandarin(&ps) } else { ps }
}

/// `MANDARIN_TONE.sub(r'ˈ\1', ps)` then `MANDARIN_UNASPIRATED` and `ʰ` removal: each syllable's
/// tone arrow becomes a stress mark before its vowels, and p/t/k without `ʰ` become b/d/ɡ.
fn english_mandarin(ps: &str) -> String {
    const VOWELS: &str = "aeiouyɛəɤɨʊɚɔmnŋ";
    let chars: Vec<char> = ps.chars().collect();
    let mut toned = String::with_capacity(ps.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        if VOWELS.contains(chars[i]) {
            let start = i;
            while i < chars.len() && VOWELS.contains(chars[i]) {
                i += 1;
            }
            let tone = i < chars.len() && "↓↗↘→".contains(chars[i]);
            if tone {
                toned.push('ˈ');
            }
            toned.extend(&chars[start..i]);
            i += usize::from(tone);
        } else {
            toned.push(chars[i]);
            i += 1;
        }
    }
    let chars: Vec<char> = toned.chars().collect();
    let mut out = String::with_capacity(toned.len());
    for (i, &c) in chars.iter().enumerate() {
        let aspirated = chars.get(i + 1) == Some(&'ʰ');
        match c {
            'p' if !aspirated => out.push('b'),
            't' if !aspirated => out.push('d'),
            'k' if !aspirated => out.push('ɡ'),
            'ʰ' => {}
            c => out.push(c),
        }
    }
    out
}

/// `CJK_PUNCTUATION`, applied to the text of pipelines that aren't Chinese or Japanese so CJK
/// sentences split into tokens and chunks at punctuation.
pub fn translate_cjk_punctuation(text: &str) -> std::borrow::Cow<'_, str> {
    fn replacement(c: char) -> Option<&'static str> {
        Some(match c {
            '。' | '．' => ". ",
            '，' => ", ",
            '！' => "! ",
            '？' => "? ",
            '：' => ": ",
            '；' => "; ",
            '「' | '『' => "“",
            '」' | '』' => "”",
            '（' => " (",
            '）' => ") ",
            '　' | '・' => " ",
            _ => return None,
        })
    }
    if !text.chars().any(|c| replacement(c).is_some()) {
        return text.into();
    }
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match replacement(c) {
            Some(r) => out.push_str(r),
            None => out.push(c),
        }
    }
    out.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Language::*;

    fn shape(text: &str) -> Vec<(&str, Option<&'static str>)> {
        split(text, |c| c != '兙').into_iter().map(|(s, l)| (s, l.map(Language::code))).collect()
    }

    #[test]
    fn runs_of_other_scripts_are_split_out() {
        assert_eq!(shape("She said Καλημέρα."), [("She said ", None), ("Καλημέρα", Some("el")), (".", None)]);
        assert_eq!(shape("Привет"), [("Привет", Some("ru"))]);
        assert_eq!(
            shape("He said مرحبا to علي."),
            [("He said ", None), ("مرحبا", Some("ar")), (" to ", None), ("علي", Some("ar")), (".", None)]
        );
        assert_eq!(shape("Dijo (Привет) y"), [("Dijo (", None), ("Привет", Some("ru")), (") y", None)]);
    }

    #[test]
    fn words_joined_by_spaces_apostrophes_and_hyphens_stay_one_run() {
        assert_eq!(shape("x Москва - река’с y"), [("x ", None), ("Москва - река’с", Some("ru")), (" y", None)]);
        assert_eq!(shape("Москва  "), [("Москва", Some("ru")), ("  ", None)]);
    }

    #[test]
    fn short_greek_is_left_to_english() {
        assert_eq!(shape("The β-catenin level rose by Δ, about π percent."), [("The β-catenin level rose by Δ, about π percent.", None)]);
        assert_eq!(shape("αβ T cells and ΔΔCt values."), [("αβ T cells and ΔΔCt values.", None)]);
        assert_eq!(shape("ΔΔ λόγος"), [("ΔΔ ", None), ("λόγος", Some("el"))]);
    }

    #[test]
    fn marks_belong_to_the_letter_before() {
        assert_eq!(shape("שָׁלוֹם"), [("שָׁלוֹם", Some("he"))]);
        assert_eq!(prepare("שָׁלוֹם", Espeak("he")), "שלום");
        assert_eq!(shape("\u{301}Привет"), [("\u{301}", None), ("Привет", Some("ru"))]);
        assert_eq!(shape("ट्रांसपोर्टरों"), [("ट्रांसपोर्टरों", Some("hi"))]);
        assert_eq!(shape("a\u{483}b"), [("a\u{483}b", None)]);
        assert_eq!(shape("Ελλ\u{301}ηνικά"), [("Ελλ\u{301}ηνικά", Some("el"))]);
    }

    #[test]
    fn other_scripts_go_to_the_voice_espeak_switches_to() {
        assert_eq!(shape("Georgian გამარჯობა is"), [("Georgian ", None), ("გამარჯობა", Some("ka")), (" is", None)]);
        assert_eq!(shape("Armenian Բարեւ"), [("Armenian ", None), ("Բարեւ", Some("hy"))]);
        assert_eq!(shape("வணக்கம்"), [("வணக்கம்", Some("ta"))]);
        assert_eq!(shape("ਸਤਿ ਸ੍ਰੀ"), [("ਸਤਿ ਸ੍ਰੀ", Some("pa"))]);
    }

    #[test]
    fn scripts_espeak_has_no_voice_for_are_unvoiced() {
        assert_eq!(shape("Thai สวัสดี ครับ."), [("Thai ", None), ("สวัสดี ครับ", Some("Thai")), (".", None)]);
        assert_eq!(shape("ሰላም"), [("ሰላም", Some("Ethiopic"))]);
        assert_eq!(shape("ᠮᠣᠩᠭᠣᠯ"), [("ᠮᠣᠩᠭᠣᠯ", Some("Mongolian"))]);
        assert_eq!(split("ᐊᑎᒍ", |_| true), [("ᐊᑎᒍ", Some(Unvoiced(Script::CanadianAboriginal)))]);
        assert_eq!(Unvoiced(Script::CanadianAboriginal).name(), "Canadian Aboriginal");
    }

    #[test]
    fn cjk_runs_are_chinese_unless_they_have_kana() {
        assert_eq!(shape("他说 我们走吧、明天见"), [("他说", Some("zh")), (" ", None), ("我们走吧、明天见", Some("zh"))]);
        assert_eq!(shape("東京へ行く"), [("東京へ行く", Some("ja"))]);
        assert_eq!(shape("ーabc"), [("ーabc", None)]);
        assert_eq!(shape("我兙你"), [("我", Some("zh")), ("兙", None), ("你", Some("zh"))]);
        assert_eq!(shape("ー東ーー京"), [("ー", None), ("東", Some("zh")), ("ーー", None), ("京", Some("zh"))]);
        assert_eq!(shape("、东"), [("、", None), ("东", Some("zh"))]);
    }

    #[test]
    fn links_are_the_english_g2ps() {
        assert_eq!(shape("Go to [东京](/tˈOkjO/) or [Ελλάδα](/ɛlˈɑdə/) now."), [("Go to [东京](/tˈOkjO/) or [Ελλάδα](/ɛlˈɑdə/) now.", None)]);
        assert_eq!(shape("[a](/b/) Привет"), [("[a](/b/) ", None), ("Привет", Some("ru"))]);
    }

    #[test]
    fn nearest_phonemes_and_mandarin() {
        assert_eq!(finish(" maɾħaba ʕali ", Espeak("ar")), "maɾhaba ʔali");
        assert_eq!(finish("pei↓ʨiŋ→", Chinese), "bˈeiʨˈiŋ");
        assert_eq!(finish("tʰa→ n↗", Chinese), "tˈa ˈn");
        assert_eq!(finish("pei↓", Espeak("ru")), "pei↓");
    }

    #[test]
    fn cjk_punctuation() {
        assert_eq!(translate_cjk_punctuation("他说：“我们走吧，明天见。”"), "他说: “我们走吧, 明天见. ”");
        assert_eq!(translate_cjk_punctuation("plain"), "plain");
    }
}
