//! Records written by `reference/dump.py`: the deployed Python pipeline's output at each stage.
#![forbid(unsafe_code)]

pub mod divergence;

use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Record {
    pub id: String,
    /// Kokoro pipeline code: a, b, e, f, h, i, j, p or z.
    pub lang: String,
    pub text: String,
    /// After line joining and the language's normalizer, before sentence splitting.
    pub normalized: String,
    pub sentences: Vec<String>,
    pub chunks: Vec<Chunk>,
    pub calls: Vec<Call>,
    /// Set when the reference pipeline raised; `chunks` and `calls` then stop where it failed.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chunk {
    pub text: String,
    pub phonemes: String,
}

/// One G2P call: `phonemize(input, lang)` in the reference server.
#[derive(Debug, Clone, Deserialize)]
pub struct Call {
    pub input: String,
    /// `(segment, language)`; language is `None` for the pipeline's own.
    pub segments: Vec<(String, Option<String>)>,
    pub phonemes: String,
    /// English only: misaki's tokens after G2P.
    #[serde(default)]
    pub tokens: Vec<Token>,
    /// English only: spaCy's tokens, one list per misaki call.
    #[serde(default)]
    pub spacy: Vec<Vec<SpacyToken>>,
    /// English only: words the espeak fallback answered, with its phonemes.
    #[serde(default)]
    pub fallback: Vec<(String, Option<String>)>,
    /// Runs read by another language's G2P: `(segment, language, phonemes)`.
    #[serde(default)]
    pub foreign: Vec<(String, String, String)>,
    /// `spacy` and `fallback` for Latin runs in another pipeline, read by English G2P.
    #[serde(default)]
    pub foreign_spacy: Vec<Vec<SpacyToken>>,
    #[serde(default)]
    pub foreign_fallback: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpacyToken {
    pub text: String,
    pub tag: String,
    pub whitespace: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Token {
    pub text: String,
    pub tag: String,
    pub whitespace: String,
    pub phonemes: Option<String>,
}

pub fn read(path: impl AsRef<Path>) -> Vec<Record> {
    let path = path.as_ref();
    let file = File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    BufReader::new(file)
        .lines()
        .enumerate()
        .filter_map(|(n, line)| {
            let line = line.unwrap();
            (!line.trim().is_empty())
                .then(|| serde_json::from_str(&line).unwrap_or_else(|e| panic!("{}:{}: {e}", path.display(), n + 1)))
        })
        .collect()
}

/// The committed fixtures (hard cases, Chinese mixed with English from `zh_mixed_cases.py`, kana next to Han and joiners in English from `kana_mixed_cases.py`), plus every dump in `reference/out/` (or `$KOKORO_REFERENCE_OUT`)
/// when present, since the downloaded corpus isn't committed.
pub fn all() -> Vec<Record> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference");
    let explicit = std::env::var_os("KOKORO_REFERENCE_OUT").map(PathBuf::from);
    let out = explicit.clone().unwrap_or_else(|| root.join("out"));
    let mut paths = vec![
        root.join("fixtures/hard_cases.out.jsonl"),
        root.join("fixtures/zh_mixed.out.jsonl"),
        root.join("fixtures/kana_mixed.out.jsonl"),
    ];
    let entries = match std::fs::read_dir(&out) {
        Err(e) if explicit.is_some() || e.kind() != std::io::ErrorKind::NotFound => {
            panic!("{}: {e}", out.display())
        }
        entries => entries.ok(),
    };
    if let Some(entries) = entries {
        let mut extra: Vec<_> = entries
            .map(|e| e.unwrap_or_else(|e| panic!("{}: {e}", out.display())).path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "jsonl") && p.file_name().is_some_and(|n| n != "hard_cases.jsonl")
            })
            .collect();
        extra.sort();
        paths.extend(extra);
    }
    paths.iter().flat_map(read).collect()
}

/// `phonemes` without the language-switch flags misaki's English fallback leaves in, as they look
/// after its English mapping (`(hi)`, `(An-us)`, `(kɔ)`). The port drops them
/// (docs/divergences.md); `None` if there were none.
pub fn without_language_flags(phonemes: &str) -> Option<String> {
    static FLAG: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\([a-zAɔɹ]{2,3}(-[a-zAɔɹ]{2,3})?\)").unwrap());
    FLAG.is_match(phonemes).then(|| FLAG.replace_all(phonemes, "").into_owned())
}

/// Whether `ours` differs from the reference's chunks of a record only in how sentences cut at
/// clauses are grouped: the port never lets a clause too short to stand alone make a chunk of its
/// own (docs/divergences.md, "Text: short clauses"), so such a sentence's clauses may be packed
/// differently and read again in other groups. Chunks are (text, phonemes). Each sentence cut at
/// clauses is a run of chunks whose texts end at clause punctuation, then one that doesn't: every
/// other chunk must be equal, and a regrouped run must hold the same clauses, in order. Returns
/// how many runs were regrouped, or `None` if anything else differs.
pub fn clause_regrouping(reference: &[(String, String)], ours: &[(String, String)]) -> Option<usize> {
    fn runs(chunks: &[(String, String)]) -> Vec<&[(String, String)]> {
        let mut out = Vec::new();
        let mut start = 0;
        for (i, (text, _)) in chunks.iter().enumerate() {
            if !text.trim_end().ends_with([',', ';', ':', '，', '、', '；', '：']) || i + 1 == chunks.len() {
                out.push(&chunks[start..=i]);
                start = i + 1;
            }
        }
        out
    }
    let text = |run: &[(String, String)]| run.iter().map(|c| c.0.trim()).collect::<Vec<_>>().join(" ");
    let (a, b) = (runs(reference), runs(ours));
    if a.len() != b.len() {
        return None;
    }
    let mut regrouped = 0;
    for (a, b) in a.iter().zip(&b) {
        if a == b {
            continue;
        }
        let same_groups = a.len() == b.len() && a.iter().zip(*b).all(|(x, y)| x.0 == y.0);
        if same_groups || text(a) != text(b) {
            return None;
        }
        regrouped += 1;
    }
    Some(regrouped)
}

#[cfg(test)]
mod tests {
    #[test]
    fn language_flags() {
        assert_eq!(super::without_language_flags("hˈɪndi(hi)həlˈʌnt(An-us) (ˈɛɹ"), Some("hˈɪndihəlˈʌnt (ˈɛɹ".into()));
        assert_eq!(super::without_language_flags("(ʧˌInˈiz) (a)"), None);
    }

    #[test]
    fn clause_regrouping_only_explains_regrouped_clauses() {
        let c = |t: &str| (t.to_owned(), format!("<{t}>"));
        let reference = [c("One."), c("aaa, he,"), c("bbb."), c("Two.")];
        let regrouped = [c("One."), c("aaa,"), c("he, bbb."), c("Two.")];
        assert_eq!(super::clause_regrouping(&reference, &regrouped), Some(1));
        assert_eq!(super::clause_regrouping(&reference, &reference), Some(0));
        let merged = [c("One."), c("aaa, he, bbb."), c("Two.")];
        assert_eq!(super::clause_regrouping(&reference, &merged), Some(1));
        let other_word = [c("One."), c("aaa,"), c("she, bbb."), c("Two.")];
        assert_eq!(super::clause_regrouping(&reference, &other_word), None);
        let other_sentence = [c("Uno."), c("aaa,"), c("he, bbb."), c("Two.")];
        assert_eq!(super::clause_regrouping(&reference, &other_sentence), None);
        let phonemes = [c("One."), c("aaa, he,"), ("bbb.".into(), "x".into()), c("Two.")];
        assert_eq!(super::clause_regrouping(&reference, &phonemes), None);
    }

    #[test]
    fn fixtures_parse() {
        let records = super::all();
        assert!(records.iter().any(|r| r.lang == "a" && !r.calls[0].spacy.is_empty()));
        assert!(records.iter().any(|r| r.lang == "z"));
    }
}
