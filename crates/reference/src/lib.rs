//! Records written by `reference/dump.py`: the deployed Python pipeline's output at each stage.

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
            (!line.trim().is_empty()).then(|| {
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("{}:{}: {e}", path.display(), n + 1))
            })
        })
        .collect()
}

/// The committed hard-case fixtures, plus every dump in `reference/out/` (or `$KOKORO_REFERENCE_OUT`)
/// when present, since the downloaded corpus isn't committed.
pub fn all() -> Vec<Record> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference");
    let out = std::env::var_os("KOKORO_REFERENCE_OUT").map_or_else(|| root.join("out"), PathBuf::from);
    let mut paths = vec![root.join("fixtures/hard_cases.out.jsonl")];
    if let Ok(entries) = std::fs::read_dir(out) {
        let mut extra: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl") && p.file_name().is_some_and(|n| n != "hard_cases.jsonl"))
            .collect();
        extra.sort();
        paths.extend(extra);
    }
    paths.iter().flat_map(read).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixtures_parse() {
        let records = super::all();
        assert!(records.iter().any(|r| r.lang == "a" && !r.calls[0].spacy.is_empty()));
        assert!(records.iter().any(|r| r.lang == "z"));
    }
}
