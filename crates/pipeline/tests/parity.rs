//! End to end against the reference server: for every English and Chinese record
//! `reference/dump.py` wrote (the committed hard cases, plus `reference/out/*.jsonl` or
//! `$KOKORO_REFERENCE_OUT`), the chunks' text and phonemes must equal the recorded `smart_split`
//! output.
//!
//! Records the intentional differences in `docs/divergences.md` change are counted apart:
//! - Japanese runs, read as "Japanese text" until its G2P is ported. Where each chunk is one
//!   reference G2P call, the chunks must equal the reference's with each run's tokens replaced by
//!   the placeholder's;
//! - runs of scripts `foreign.py` leaves to the English G2P, which now go to espeak's voice for
//!   the script or read as "<script> text";
//! - espeak's language-switch flags, which the reference's English fallback reads out.
//!
//! `PARITY_SHOW=n` prints up to n mismatches (default 20).

use kokoro_pipeline::foreign::{Language, Script, script};
use kokoro_pipeline::{ForeignG2p, Lang, NormalizationOptions, Pipeline};
use std::collections::HashMap;
use kokoro_reference::{Call, Record};
use std::time::Instant;

fn is_cjk(language: &Option<String>) -> bool {
    matches!(language.as_deref(), Some("ja"))
}

fn uses_cjk(record: &Record) -> bool {
    record.calls.iter().flat_map(|c| &c.segments).any(|(_, l)| is_cjk(l))
}

/// Letters of a script `foreign.py` doesn't split out, other than Latin and CJK.
fn uses_new_script(record: &Record) -> bool {
    use Script::*;
    record.text.chars().filter_map(script).any(|s| {
        !matches!(s, Latin | Han | Hiragana | Katakana | Arabic | Cyrillic | Devanagari | Greek | Hangul | Hebrew)
    })
}

/// An English record where the reference left a Han character pypinyin can't read to the English
/// G2P, which the port reads as "Chinese text".
fn reads_unreadable_han(record: &Record) -> bool {
    record.lang != "z"
        && record.calls.iter().flat_map(|c| &c.segments).any(|(s, l)| l.is_none() && s.chars().any(|c| script(c) == Some(Script::Han)))
}

fn has_language_flag(record: &Record) -> bool {
    record.chunks.iter().any(|c| kokoro_reference::without_language_flags(&c.phonemes).is_some())
}

struct Out {
    phonemes: String,
    whitespace: bool,
}

fn starts_word(phonemes: Option<&str>) -> bool {
    phonemes.and_then(|p| p.chars().next()).is_some_and(char::is_alphabetic)
}

/// The reference call's phonemes with each Japanese run (runs of one language with
/// only spaces between them counting as one) read as `placeholder(language)`, kept apart from a
/// word right against it.
fn with_placeholders(call: &Call, placeholder: impl Fn(&str) -> Vec<String>) -> String {
    let mut runs = call.foreign.iter().filter(|(_, l, _)| l == "ja").peekable();
    let mut out: Vec<Out> = Vec::new();
    let mut tokens = call.tokens.iter().peekable();
    while let Some(token) = tokens.next() {
        let starts_run = |t: &kokoro_reference::Token, run: Option<&&(String, String, String)>| {
            t.tag.is_empty() && run.is_some_and(|(segment, _, _)| *segment == t.text)
        };
        if !starts_run(token, runs.peek()) {
            let phonemes = token.phonemes.clone().unwrap_or_default();
            out.push(Out { phonemes, whitespace: !token.whitespace.is_empty() });
            continue;
        }
        let language = runs.peek().unwrap().1.clone();
        let mut first = Some(token);
        let mut whitespace = String::new();
        while let Some(start) = first.take() {
            let (_, _, phonemes) = runs.next().unwrap();
            let mut last = start;
            for _ in 1..phonemes.split_whitespace().count() {
                last = tokens.next().unwrap();
            }
            whitespace = last.whitespace.clone();
            if let Some(&next) = tokens.peek()
                && starts_run(next, runs.peek())
                && runs.peek().unwrap().1 == language
                && whitespace.chars().all(|c| c == ' ')
            {
                first = tokens.next();
            }
        }
        let next = tokens.peek().and_then(|t| t.phonemes.as_deref());
        if let Some(previous) = out.iter_mut().rev().find(|o| !o.phonemes.is_empty() || o.whitespace)
            && previous.phonemes.ends_with(char::is_alphabetic)
        {
            previous.whitespace = true;
        }
        let words = placeholder("Japanese");
        let n = words.len();
        for (i, phonemes) in words.into_iter().enumerate() {
            let whitespace = if i + 1 < n {
                true
            } else {
                !whitespace.is_empty() || starts_word(next)
            };
            out.push(Out { phonemes, whitespace });
        }
    }
    let joined: String = out.iter().map(|o| format!("{}{}", o.phonemes, if o.whitespace { " " } else { "" })).collect();
    joined.trim().to_owned()
}

/// The reference's Japanese G2P output for each run it read, standing in for the Japanese G2P
/// in the Chinese pipeline.
struct Recorded(HashMap<String, String>);

impl ForeignG2p for Recorded {
    fn phonemize(&self, text: &str) -> Result<String, String> {
        self.0.get(text).cloned().ok_or_else(|| format!("no recorded phonemes for {text:?}"))
    }
}

#[test]
fn reference_records() {
    let records: Vec<Record> = kokoro_reference::all().into_iter().filter(|r| Lang::from_code(&r.lang).is_some()).collect();
    let american = Pipeline::new(Lang::AmericanEnglish).unwrap();
    let british = Pipeline::new(Lang::BritishEnglish).unwrap();
    let chinese = Pipeline::new(Lang::Chinese).unwrap();
    let japanese: HashMap<String, String> = records
        .iter()
        .filter(|r| r.lang == "z")
        .flat_map(|r| &r.calls)
        .flat_map(|c| &c.foreign)
        .filter(|(_, l, _)| l == "ja")
        .map(|(segment, _, phonemes)| (segment.clone(), phonemes.clone()))
        .collect();
    let chinese_recorded = Pipeline::new(Lang::Chinese).unwrap().with_foreign_g2p(Language::Japanese, Box::new(Recorded(japanese)));
    let options = NormalizationOptions::default();
    let show: usize = std::env::var("PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let (mut matched, mut compared, mut chars) = (0, 0, 0);
    let (mut cjk, mut cjk_matched, mut new_script, mut flagged) = (0, 0, Vec::new(), Vec::new());
    let (mut zh, mut zh_matched, mut failing, mut failing_matched, mut unreadable) = (0, 0, 0, 0, Vec::new());
    let mut mismatches = Vec::new();
    let start = Instant::now();
    for record in &records {
        let chinese_with_japanese = record.lang == "z" && uses_cjk(record);
        let pipeline = match record.lang.as_str() {
            "b" => &british,
            // Read with the reference's own Japanese phonemes, so the chunks must match exactly.
            "z" if chinese_with_japanese => &chinese_recorded,
            "z" => &chinese,
            _ => &american,
        };
        // Chunks up to the first error: where the reference fails, its chunks stop there, and the
        // port's error stands in for the failed sentence.
        let mut actual = Vec::new();
        let mut error = None;
        for chunk in pipeline.chunks(&record.text, &options) {
            match chunk {
                Ok(c) if c.pause.is_none() && error.is_none() => actual.push((c.text, c.phonemes)),
                Ok(_) => {}
                Err(e) => {
                    error.get_or_insert(e);
                }
            }
        }
        chars += record.text.chars().count();
        if uses_new_script(record) {
            new_script.push(record.id.as_str());
            continue;
        }
        if has_language_flag(record) {
            flagged.push(record.id.as_str());
            continue;
        }
        if reads_unreadable_han(record) {
            unreadable.push(record.id.as_str());
            continue;
        }
        let expected: Vec<(String, String)> = if uses_cjk(record) && !chinese_with_japanese {
            cjk += 1;
            if record.calls.len() != record.chunks.len()
                || record.calls.iter().zip(&record.chunks).any(|(call, chunk)| call.input != chunk.text)
            {
                continue;
            }
            let placeholder = |name: &str| {
                pipeline.phonemize(&format!("{name} text")).unwrap().0.split(' ').map(str::to_owned).collect()
            };
            record.calls.iter().map(|c| (c.input.clone(), with_placeholders(c, placeholder))).collect()
        } else {
            compared += 1;
            zh += usize::from(record.lang == "z");
            failing += usize::from(record.error.is_some());
            record.chunks.iter().map(|c| (c.text.clone(), c.phonemes.clone())).collect()
        };
        if error.is_some() == record.error.is_some() && actual == expected {
            if uses_cjk(record) && !chinese_with_japanese {
                cjk_matched += 1;
            } else {
                matched += 1;
                zh_matched += usize::from(record.lang == "z");
                failing_matched += usize::from(record.error.is_some());
            }
        } else {
            let first = expected.iter().zip(&actual).position(|(e, a)| e != a).unwrap_or(expected.len().min(actual.len()));
            mismatches.push(format!(
                "{} ({} vs {} chunks, first difference at {first}; error {error:?}, reference {:?})\n  expected: {:?}\n  actual:   {:?}",
                record.id,
                expected.len(),
                actual.len(),
                record.error,
                expected.get(first),
                actual.get(first)
            ));
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{matched}/{compared} records match exactly ({:.3}%): {zh_matched}/{zh} of them Chinese (those with Japanese runs read with the reference's Japanese phonemes), {failing_matched}/{failing} up to where the reference fails; {cjk_matched}/{cjk} English records with Japanese runs match with placeholders",
        100.0 * matched as f64 / compared.max(1) as f64
    );
    println!(
        "not compared: {} with other scripts {new_script:?}, {} with espeak language flags {flagged:?}, {} with Han characters pypinyin can't read {unreadable:?}",
        new_script.len(),
        flagged.len(),
        unreadable.len()
    );
    println!("{} records, {chars} characters in {:.2?}", records.len(), elapsed);
    for m in mismatches.iter().take(show) {
        println!("{m}");
    }
    assert!(mismatches.is_empty(), "{} records differ from the reference", mismatches.len());
}
