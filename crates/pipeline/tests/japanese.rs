//! The Japanese pipeline (`lang` j) end to end against the reference server: for every j record
//! `reference/dump.py` wrote (the committed hard cases and `tests/japanese.jsonl`, plus
//! `reference/out/*.jsonl` or `$KOKORO_REFERENCE_OUT`), the chunks' text and phonemes must equal
//! the recorded `smart_split` output. `tests/japanese.jsonl` is `crates/g2p-ja/tools/gen_mixed.py 3`
//! on Gutenberg and Aozora Bunko text (both public domain): Japanese with English, numbers and
//! other scripts, and English with Japanese runs (which `tests/parity.rs` reads).
//!
//! Where the reference raised (misaki asserts on numbers of more than nine digits), its chunks stop
//! at the sentence that failed; ours must start with them, and go on to read the rest
//! (`docs/divergences.md`). Records with letters of scripts `foreign.py` doesn't split out, which
//! the Japanese pipeline now reads as the English pipelines do, aren't compared.
//!
//! Needs UniDic (`crates/g2p-ja/tools/fetch_unidic.sh`). `PARITY_SHOW=n` prints up to n mismatches.

use kokoro_pipeline::foreign::{Script, script};
use kokoro_pipeline::{Lang, NormalizationOptions, Pipeline};
use std::time::Instant;

fn uses_new_script(text: &str) -> bool {
    use Script::*;
    text.chars().filter_map(script).any(|s| {
        !matches!(s, Latin | Han | Hiragana | Katakana | Arabic | Cyrillic | Devanagari | Greek | Hangul | Hebrew)
    })
}

#[test]
fn japanese_reference_records() {
    let mut records = kokoro_reference::all();
    records.extend(kokoro_reference::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/japanese.jsonl")));
    records.retain(|r| r.lang == "j");
    let pipeline = Pipeline::new(Lang::Japanese).unwrap();
    let options = NormalizationOptions::default();
    if let Some(Err(e)) = pipeline.chunks("日本", &options).next() {
        return kokoro_pipeline::skip_without_dictionary(e);
    }
    let show: usize = std::env::var("PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let (mut matched, mut compared, mut fixed, mut skipped, mut chars, mut chunks) = (0, 0, 0, 0, 0, 0);
    let mut mismatches = Vec::new();
    let start = Instant::now();
    for record in &records {
        chars += record.text.chars().count();
        let actual: Vec<(String, String)> = pipeline
            .chunks(&record.text, &options)
            .filter_map(|c| {
                let c = c.unwrap_or_else(|e| panic!("{}: {e}", record.id));
                c.pause.is_none().then_some((c.text, c.phonemes))
            })
            .collect();
        chunks += actual.len();
        let expected: Vec<(String, String)> =
            record.chunks.iter().map(|c| (c.text.clone(), c.phonemes.clone())).collect();
        if uses_new_script(&record.text) {
            skipped += 1;
            if actual != expected && std::env::var_os("SHOW_SKIPPED").is_some() {
                println!("{}: {:?}\n  expected {expected:?}\n  actual   {actual:?}", record.id, record.text);
            }
            continue;
        }
        let ok = if let Some(error) = &record.error {
            assert!(error.starts_with("AssertionError"), "{}: {error}", record.id);
            fixed += 1;
            actual.len() > expected.len() && actual[..expected.len()] == expected[..]
        } else {
            compared += 1;
            actual == expected
        };
        if ok {
            matched += 1;
        } else {
            let first =
                expected.iter().zip(&actual).position(|(e, a)| e != a).unwrap_or(expected.len().min(actual.len()));
            mismatches.push(format!(
                "{} ({} vs {} chunks, first difference at {first})\n  expected: {:?}\n  actual:   {:?}",
                record.id,
                expected.len(),
                actual.len(),
                expected.get(first),
                actual.get(first)
            ));
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{matched}/{} Japanese records match ({compared} exactly, {fixed} where the reference raised); {skipped} with other scripts not compared",
        compared + fixed
    );
    println!("{} records, {chars} characters, {chunks} chunks in {elapsed:.2?}", records.len());
    for m in mismatches.iter().take(show) {
        println!("{m}");
    }
    assert!(mismatches.is_empty(), "{} records differ from the reference", mismatches.len());
}
