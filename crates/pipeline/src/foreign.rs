//! kokoro's `foreign.py` (brendanlong/kokoro at the commit Kokoro-FastAPI pins) for the English
//! pipelines: finds runs of other scripts so each is read by its own language's G2P, in the
//! English voice, and post-processes their phonemes.
//!
//! `foreign.py` looks for six scripts besides Han and kana; here every non-Latin script makes
//! runs. The six keep `foreign.py`'s espeak voices and rules; any other script is read by the
//! language espeak-ng itself switches to for an alphabet of that script
//! ([`kokoro_espeak::alphabet_languages`]), if there is one. Runs of the new scripts also hold the
//! invisible format characters (`\p{Cf}`: zero-width space and joiners, soft hyphens) between
//! their letters.

pub use crate::tables::Script;
use crate::tables::{FORMAT_CHARS, NONSPACING_MARKS, SCRIPTS};
use kokoro_text::py;
use std::cmp::Ordering;
use std::sync::LazyLock;

/// A language other than the pipeline's, which reads a run of its script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Chinese,
    Japanese,
    /// Latin runs in a pipeline of another language (Chinese, Japanese), read by misaki's American
    /// English G2P.
    English,
    /// A run of another script, read by espeak with the voice for the script ([`espeak_voice`]).
    Script(Script),
    /// Letters of a script espeak has no voice for, or that its voice for the script doesn't read.
    Unread(Script),
}

impl Language {
    /// The reference's language code (`zh`, `ja`, or the espeak voice's), or the script's name if
    /// espeak has no voice for it.
    pub fn code(self) -> &'static str {
        match self {
            Language::Chinese => "zh",
            Language::Japanese => "ja",
            Language::English => "en",
            Language::Script(script) => espeak_voice(script).expect("a voiced script"),
            Language::Unread(script) => script.name(),
        }
    }

    /// The espeak voice that reads this language: none for Chinese and Japanese, which have G2Ps
    /// of their own, or for letters espeak doesn't read.
    pub fn espeak(self) -> Option<&'static str> {
        match self {
            Language::Script(script) => espeak_voice(script),
            _ => None,
        }
    }

    /// The English name a run is read as when there's no G2P for it.
    pub fn name(self) -> &'static str {
        match self {
            Language::Chinese => "Chinese",
            Language::Japanese => "Japanese",
            Language::English => "English",
            // Han characters pypinyin has no reading for.
            Language::Unread(Script::Han) => "Chinese",
            Language::Script(script) | Language::Unread(script) => script.name(),
        }
    }
}

/// `foreign.py`'s scripts, whose runs keep its rules exactly.
pub fn in_foreign_py(script: Script) -> bool {
    use Script::*;
    matches!(script, Han | Hiragana | Katakana | Latin | Arabic | Cyrillic | Devanagari | Greek | Hangul | Hebrew)
}

/// espeak-ng's alphabets that name a language, each with the script most of its letters are in.
static ALPHABETS: LazyLock<Vec<(char, char, String, Script)>> = LazyLock::new(|| {
    kokoro_espeak::alphabet_languages()
        .into_iter()
        .filter_map(|(lo, hi, language)| {
            let mut counts: Vec<(Script, usize)> = Vec::new();
            for s in (lo..=hi).filter_map(script) {
                match counts.iter_mut().find(|(x, _)| *x == s) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((s, 1)),
                }
            }
            let (dominant, _) = counts.into_iter().max_by_key(|&(_, n)| n)?;
            Some((lo, hi, language, dominant))
        })
        .collect()
});

/// The espeak voice that reads a run of `script`: `foreign.py`'s for its six scripts, otherwise
/// the language of the espeak-ng alphabet whose letters are mostly of that script. Coptic in the
/// Greek block, or Javanese in the range espeak gives Korean, aren't what those voices read.
pub fn espeak_voice(script: Script) -> Option<&'static str> {
    match script {
        Script::Arabic => Some("ar"),
        Script::Cyrillic => Some("ru"),
        Script::Devanagari => Some("hi"),
        Script::Greek => Some("el"),
        Script::Hangul => Some("ko"),
        Script::Hebrew => Some("he"),
        Script::Latin | Script::Han | Script::Hiragana | Script::Katakana => None,
        _ => ALPHABETS.iter().find(|a| a.3 == script).map(|a| a.2.as_str()),
    }
}

/// Whether espeak's voice for `script` reads `c`: espeak lowercases a word, then reads the letters
/// in the alphabet it switched to. Georgian capitals (Mtavruli) lowercase into it; Nuskhuri and
/// Armenian ligatures don't, and espeak would spell them out.
fn espeak_reads(script: Script, c: char) -> bool {
    c.to_lowercase().all(|c| ALPHABETS.iter().any(|&(lo, hi, _, s)| s == script && (lo..=hi).contains(&c)))
}

/// Every espeak voice a run may go to.
pub fn espeak_voices() -> Vec<&'static str> {
    let mut voices: Vec<&'static str> = SCRIPTS.iter().filter_map(|&(_, _, script)| espeak_voice(script)).collect();
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

fn is_format(c: char) -> bool {
    in_ranges(FORMAT_CHARS, c)
}

/// What may come between two runs of one language for them to be read as one: the joiners,
/// marks and format characters.
pub fn is_gap(c: char) -> bool {
    is_joiner(c) || is_mark(c) || is_format(c)
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
/// Outside `foreign.py`'s scripts, `\p{Cf}` counts as a mark.
fn word_end(text: &str, start: usize, s: Script) -> Option<usize> {
    let joins = |c: char| is_mark(c) || (!in_foreign_py(s) && is_format(c));
    let mut letters = 0;
    let mut end = start;
    for c in text[start..].chars() {
        if script(c) == Some(s) {
            letters += 1;
        } else if letters == 0 || !joins(c) {
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

/// The pipeline a text is split in: what `ForeignScripts` takes as its own language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Native {
    /// `lang` a and b: Latin runs are the pipeline's own.
    English,
    /// `lang` z: Han runs without kana are its own, Latin runs [`Language::English`].
    Chinese,
    /// `lang` j: every Han and kana run is its own, Latin runs [`Language::English`].
    Japanese,
    /// An espeak language (`lang` e, f, h, i and p) written in this script (`NATIVE_SCRIPTS`):
    /// runs of the script are its own, Latin runs (in Hindi) [`Language::English`], and Han and
    /// kana runs Chinese and Japanese as in the English pipelines.
    Espeak(Script),
}

/// `ForeignScripts.split` in an English pipeline: `(segment, language)` pairs covering `text`,
/// where `None` is English. `reads` says whether the Chinese G2P has a reading for a Han
/// character: the reference leaves those pypinyin can't read (some radicals, Japanese-only
/// kanji) to the English G2P, which here reads them as [`Language::Unread`]`(Han)`. `ー` outside
/// kana stays English.
pub fn split(text: &str, reads: impl Fn(char) -> bool) -> Vec<(&str, Option<Language>)> {
    split_in(text, Native::English, reads)
}

/// `ForeignScripts.split` in the Chinese pipeline: Han runs without kana are its own (`None`),
/// whatever pypinyin reads, and Latin runs are [`Language::English`].
pub fn split_chinese(text: &str) -> Vec<(&str, Option<Language>)> {
    split_in(text, Native::Chinese, |_| true)
}

/// `ForeignScripts.split` in the pipeline of `native`, where `None` is the pipeline's own
/// language; `reads` is as for [`split`], in the pipelines that split Han runs out.
pub fn split_in(text: &str, native: Native, reads: impl Fn(char) -> bool) -> Vec<(&str, Option<Language>)> {
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
            None if native == Native::Japanese => continue,
            Some(script) if native == Native::Espeak(script) => continue,
            Some(Script::Latin) if native != Native::English => Language::English,
            None if run_text.chars().any(is_kana) => Language::Japanese,
            None if native == Native::Chinese => continue,
            None => {
                let language = |c: char| match script(c) {
                    _ if c == '、' => Some(Language::Chinese),
                    Some(Script::Han) if reads(c) => Some(Language::Chinese),
                    Some(Script::Han) => Some(Language::Unread(Script::Han)),
                    _ => None,
                };
                let mut start = run.start;
                let mut chars = run_text.char_indices().peekable();
                while let Some((i, c)) = chars.next() {
                    let l = language(c);
                    if chars.peek().is_none_or(|&(_, next)| language(next) != l) {
                        let end = run.start + i + c.len_utf8();
                        if let Some(l) = l {
                            push(start, end, l);
                        }
                        start = end;
                    }
                }
                continue;
            }
            Some(Script::Latin) => continue,
            Some(script) if in_foreign_py(script) => Language::Script(script),
            Some(_) if !run_text.chars().any(char::is_alphanumeric) => continue,
            Some(script) if espeak_voice(script).is_none() => Language::Unread(script),
            Some(script) => {
                let language = |reads: bool| if reads { Language::Script(script) } else { Language::Unread(script) };
                let mut start = run.start;
                let mut current = None;
                for (i, c) in run_text.char_indices().filter(|&(_, c)| self::script(c) == Some(script)) {
                    let reads = espeak_reads(script, c);
                    if let Some(previous) = current
                        && previous != reads
                    {
                        push(start, run.start + i, language(previous));
                        start = run.start + i;
                    }
                    current = Some(reads);
                }
                push(start, run.end, language(current.unwrap_or(false)));
                continue;
            }
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
    if language == Language::Script(Script::Hebrew) && segment.chars().any(is_mark) {
        segment.chars().filter(|&c| !is_mark(c)).collect::<String>().into()
    } else {
        segment.into()
    }
}

/// The rest of `ForeignScripts.phonemize` in an English pipeline: stripped, with sounds Kokoro
/// has no phoneme for mapped to the nearest one, and Mandarin tones and unaspirated stops made
/// English.
pub fn finish(phonemes: &str, language: Language) -> String {
    finish_in(phonemes, language, true)
}

/// [`finish`] in an English pipeline or another: outside English, Mandarin tones still become
/// stress marks, but its stops stay as they are (`if self.language == 'en'`).
pub fn finish_in(phonemes: &str, language: Language, english: bool) -> String {
    let ps: String = py::strip(phonemes)
        .chars()
        .map(|c| match c {
            'ħ' => 'h',
            'ʕ' => 'ʔ',
            c => c,
        })
        .collect();
    if language == Language::Chinese { mandarin(&ps, english) } else { ps }
}

/// `MANDARIN_TONE.sub(r'ˈ\1', ps)`, then in English `MANDARIN_UNASPIRATED` and `ʰ` removal:
/// each syllable's tone arrow becomes a stress mark before its vowels, and p/t/k without `ʰ`
/// become b/d/ɡ.
fn mandarin(ps: &str, english: bool) -> String {
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
    if !english {
        return toned;
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
    use Language::Chinese;

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
        assert_eq!(
            shape("The β-catenin level rose by Δ, about π percent."),
            [("The β-catenin level rose by Δ, about π percent.", None)]
        );
        assert_eq!(shape("αβ T cells and ΔΔCt values."), [("αβ T cells and ΔΔCt values.", None)]);
        assert_eq!(shape("ΔΔ λόγος"), [("ΔΔ ", None), ("λόγος", Some("el"))]);
    }

    #[test]
    fn marks_belong_to_the_letter_before() {
        assert_eq!(shape("שָׁלוֹם"), [("שָׁלוֹם", Some("he"))]);
        assert_eq!(prepare("שָׁלוֹם", Language::Script(Script::Hebrew)), "שלום");
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
    fn the_runs_script_decides_its_voice() {
        assert_eq!(shape("ᲒᲐᲛᲐᲠᲯᲝᲑᲐ"), [("ᲒᲐᲛᲐᲠᲯᲝᲑᲐ", Some("ka"))]);
        assert_eq!(shape("Ა გამარჯობა"), [("Ა გამარჯობა", Some("ka"))]);
        assert_eq!(shape("ⴂⴀⴋⴀ"), [("ⴂⴀⴋⴀ", Some("Georgian"))]);
        assert_eq!(shape("ﬓԲարեւ ﬔ"), [("ﬓ", Some("Armenian")), ("Բարեւ ", Some("hy")), ("ﬔ", Some("Armenian"))]);
        for (text, script) in
            [("ꦗꦮ", "Javanese"), ("ꨌꩌ", "Cham"), ("ꮳꮃꭹ", "Cherokee"), ("ꯃꯤꯇꯩ", "Meetei Mayek"), ("ϢϤϦ", "Coptic")]
        {
            assert_eq!(shape(text), [(text, Some(script))]);
        }
    }

    #[test]
    fn format_characters_stay_inside_runs_of_new_scripts() {
        assert_eq!(shape("สวัสดี\u{200b}ครับ\u{200b}ผม"), [("สวัสดี\u{200b}ครับ\u{200b}ผม", Some("Thai"))]);
        assert_eq!(shape("สวัส\u{ad}ดี"), [("สวัส\u{ad}ดี", Some("Thai"))]);
        assert_eq!(shape("ශ්\u{200d}රී"), [("ශ්\u{200d}රී", Some("si"))]);
        assert_eq!(shape("می\u{200c}خواهم"), [("می", Some("ar")), ("\u{200c}", None), ("خواهم", Some("ar"))]);
    }

    #[test]
    fn scripts_espeak_has_no_voice_for_are_unvoiced() {
        assert_eq!(shape("Thai สวัสดี ครับ."), [("Thai ", None), ("สวัสดี ครับ", Some("Thai")), (".", None)]);
        assert_eq!(shape("ሰላም"), [("ሰላም", Some("Ethiopic"))]);
        assert_eq!(shape("፩፪፫"), [("፩፪፫", Some("Ethiopic"))]);
        assert_eq!(shape("a ꫰ ๏"), [("a ꫰ ๏", None)]);
        assert_eq!(shape("ᠮᠣᠩᠭᠣᠯ"), [("ᠮᠣᠩᠭᠣᠯ", Some("Mongolian"))]);
        assert_eq!(split("ᐊᑎᒍ", |_| true), [("ᐊᑎᒍ", Some(Language::Unread(Script::CanadianAboriginal)))]);
        assert_eq!(Language::Unread(Script::CanadianAboriginal).name(), "Canadian Aboriginal");
    }

    #[test]
    fn cjk_runs_are_chinese_unless_they_have_kana() {
        assert_eq!(
            shape("他说 我们走吧、明天见"),
            [("他说", Some("zh")), (" ", None), ("我们走吧、明天见", Some("zh"))]
        );
        assert_eq!(shape("東京へ行く"), [("東京へ行く", Some("ja"))]);
        assert_eq!(shape("ーabc"), [("ーabc", None)]);
        assert_eq!(shape("我兙你"), [("我", Some("zh")), ("兙", Some("Han")), ("你", Some("zh"))]);
        assert_eq!(Language::Unread(Script::Han).name(), "Chinese");
        assert_eq!(shape("ー東ーー京"), [("ー", None), ("東", Some("zh")), ("ーー", None), ("京", Some("zh"))]);
        assert_eq!(shape("、东"), [("、", None), ("东", Some("zh"))]);
    }

    #[test]
    fn espeak_languages_keep_their_own_script() {
        let shape = |text, script| -> Vec<(&str, Option<&str>)> {
            split_in(text, Native::Espeak(script), |c| c != '兙')
                .into_iter()
                .map(|(s, l)| (s, l.map(Language::code)))
                .collect()
        };
        assert_eq!(
            shape("Dijo नमस्ते y 北京 o 兙.", Script::Latin),
            [
                ("Dijo ", None),
                ("नमस्ते", Some("hi")),
                (" y ", None),
                ("北京", Some("zh")),
                (" o ", None),
                ("兙", Some("Han")),
                (".", None)
            ]
        );
        assert_eq!(
            shape("वह New York गया", Script::Devanagari),
            [("वह ", None), ("New York", Some("en")), (" गया", None)]
        );
        assert_eq!(
            shape("Привет मित्र カナ", Script::Devanagari),
            [("Привет", Some("ru")), (" मित्र ", None), ("カナ", Some("ja"))]
        );
    }

    #[test]
    fn links_are_the_english_g2ps() {
        assert_eq!(
            shape("Go to [东京](/tˈOkjO/) or [Ελλάδα](/ɛlˈɑdə/) now."),
            [("Go to [东京](/tˈOkjO/) or [Ελλάδα](/ɛlˈɑdə/) now.", None)]
        );
        assert_eq!(shape("[a](/b/) Привет"), [("[a](/b/) ", None), ("Привет", Some("ru"))]);
    }

    #[test]
    fn nearest_phonemes_and_mandarin() {
        assert_eq!(finish(" maɾħaba ʕali ", Language::Script(Script::Arabic)), "maɾhaba ʔali");
        assert_eq!(finish("pei↓ʨiŋ→", Chinese), "bˈeiʨˈiŋ");
        assert_eq!(finish("tʰa→ n↗", Chinese), "tˈa ˈn");
        assert_eq!(finish("pei↓", Language::Script(Script::Cyrillic)), "pei↓");
        assert_eq!(finish_in("pei↓ʨʰiŋ→", Chinese, false), "pˈeiʨʰˈiŋ");
    }

    #[test]
    fn cjk_punctuation() {
        assert_eq!(translate_cjk_punctuation("他说：“我们走吧，明天见。”"), "他说: “我们走吧, 明天见. ”");
        assert_eq!(translate_cjk_punctuation("plain"), "plain");
    }
}
