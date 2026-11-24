//! The espeak pipelines (`lang` e, f, h, i and p: kokoro's `g2p_phonemes` with misaki's
//! `EspeakG2P`) end to end against the reference server: for every record of those languages
//! `reference/dump.py` wrote (the committed hard cases and `tests/espeak_mixed.jsonl`, plus
//! `reference/out/*.jsonl` or `$KOKORO_REFERENCE_OUT`), the chunks' text and phonemes must equal
//! the recorded `smart_split` output. `tests/espeak_mixed.jsonl` is `reference/espeak_mixed_cases.py
//! 1 100`: each language's Wikipedia sentences mixed with English words, Devanagari, Han, kana, the
//! other scripts, CJK punctuation, parentheses and links. `dump.py` records no ids or word spans;
//! `tests/requests.rs` compares those for these languages (`request_cases.py ... efhip`).
//!
//! Records the intentional differences in `docs/divergences.md` change are counted apart, per
//! language:
//! - runs of scripts `foreign.py` leaves to the pipeline's G2P, which now go to espeak's voice for
//!   the script or read as "<script> text";
//! - Han characters pypinyin can't read, which the reference leaves to the pipeline's espeak and
//!   the port reads as "Chinese text";
//! - espeak's language-switch flags, which misaki's English fallback leaves in Latin runs of
//!   Hindi text;
//! - espeak calls whose output in the reference depends on memory an earlier call left
//!   (`crates/espeak/tests/divergences.jsonl`).
//!
//! Japanese runs need UniDic (`$KOKORO_UNIDIC`); without it, records with them aren't compared.
//! `PARITY_SHOW=n` prints up to n mismatches (default 20), and `SHOW_SKIPPED=1` the records not
//! compared that differ.

mod common;

use kokoro_pipeline::foreign::{Script, in_foreign_py, script};
use kokoro_pipeline::{Lang, Pipeline};
use kokoro_reference::Record;
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

/// Letters of a script `foreign.py` doesn't split out, other than Latin and CJK.
fn uses_new_script(record: &Record) -> bool {
    record.text.chars().filter_map(script).any(|s| !in_foreign_py(s))
}

fn reads_unreadable_han(record: &Record) -> bool {
    record
        .calls
        .iter()
        .flat_map(|c| &c.segments)
        .any(|(s, l)| l.is_none() && s.chars().any(|c| script(c) == Some(Script::Han)))
}

fn uses_japanese(record: &Record) -> bool {
    record.calls.iter().flat_map(|c| &c.segments).any(|(_, l)| l.as_deref() == Some("ja"))
}

fn has_language_flag(record: &Record) -> bool {
    record.chunks.iter().any(|c| kokoro_reference::without_language_flags(&c.phonemes).is_some())
}

/// Texts whose reference espeak output depends on uninitialised memory.
fn espeak_divergences() -> HashSet<String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../espeak/tests/divergences.jsonl");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["text"].as_str().unwrap().to_owned())
        .collect()
}

fn reads_divergent_espeak(record: &Record, divergent: &HashSet<String>) -> bool {
    record.calls.iter().flat_map(|c| &c.segments).any(|(s, _)| divergent.contains(s.trim()))
}

#[derive(Default)]
struct Count {
    records: usize,
    compared: usize,
    matched: usize,
    /// Matched but for regrouped short clauses (docs/divergences.md).
    regrouped: usize,
    /// Matched but for the spacing where runs of different G2Ps join (docs/divergences.md).
    respaced: usize,
    /// Matched but for a name read in its own language.
    named: usize,
    chunks: usize,
    chars: usize,
    skipped: BTreeMap<&'static str, Vec<String>>,
}

/// Whether each chunk differs only in names, after the port's spacing where runs join.
fn names_only(expected: &[(String, String)], actual: &[(String, String)], language: &str) -> bool {
    expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(e, a)| {
            e == a
                || e.0 == a.0
                    && kokoro_reference::divergence::differs_only_in_names(
                        &names_in(&e.0, language),
                        &kokoro_reference::divergence::respaced(&e.1, &a.1),
                        &a.1,
                    )
        })
}

/// The phonemes of the names the G2P links in `text` of `language` (espeak's code, `en` for English).
fn names_in(text: &str, language: &str) -> Vec<String> {
    static G2P: std::sync::LazyLock<kokoro_g2p_en::G2p> = std::sync::LazyLock::new(|| kokoro_g2p_en::G2p::new(false));
    G2P.names_in(text, language)
}

#[test]
fn espeak_reference_records() {
    let mut records = kokoro_reference::all();
    records.extend(kokoro_reference::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/espeak_mixed.jsonl")));
    let langs = [Lang::Spanish, Lang::French, Lang::Hindi, Lang::Italian, Lang::BrazilianPortuguese];
    records.retain(|r| langs.iter().any(|l| l.code() == r.lang));
    let pipelines: Vec<Pipeline> = langs.iter().map(|&l| Pipeline::new(l).unwrap()).collect();
    let japanese = kokoro_pipeline::japanese_available();
    let divergent = espeak_divergences();
    let show: usize = std::env::var("PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let mut counts: BTreeMap<&str, Count> = BTreeMap::new();
    let mut mismatches = Vec::new();
    let start = Instant::now();
    let mut voices = common::Voices::default();
    for record in &records {
        let pipeline = pipelines.iter().find(|p| p.lang().code() == record.lang).unwrap();
        let count = counts.entry(pipeline.lang().code()).or_default();
        count.records += 1;
        count.chars += record.text.chars().count();
        let actual: Vec<(String, String)> = pipeline
            .chunks(&record.text)
            .filter_map(|c| {
                let c = c.unwrap_or_else(|e| panic!("{}: {e}", record.id));
                c.pause.is_none().then_some((c.text, kokoro_reference::ours(&c.phonemes)))
            })
            .collect();
        count.chunks += actual.len();
        let expected: Vec<(String, String)> =
            record.chunks.iter().map(|c| (c.text.clone(), kokoro_reference::canonical(&c.phonemes))).collect();
        assert!(record.error.is_none(), "{}: the reference failed: {:?}", record.id, record.error);
        let reason = if uses_new_script(record) {
            Some("other scripts")
        } else if reads_unreadable_han(record) {
            Some("Han without a reading")
        } else if has_language_flag(record) {
            Some("espeak language flags")
        } else if kokoro_reference::divergence::has_link_read_as_text(&record.lang, &record.text) {
            Some("links, which the reference read as text")
        } else if voices.change_letters(pipeline.lang(), &record.text) {
            Some("letters espeak lacks or marks it composes")
        } else if reads_divergent_espeak(record, &divergent) {
            Some("espeak reads uninitialised memory")
        } else if uses_japanese(record) && !japanese {
            Some("Japanese without UniDic")
        } else {
            None
        };
        if let Some(reason) = reason {
            count.skipped.entry(reason).or_default().push(record.id.clone());
            if actual != expected && std::env::var_os("SHOW_SKIPPED").is_some() {
                println!("{} ({reason}): {:?}\n  expected {expected:?}\n  actual   {actual:?}", record.id, record.text);
            }
            continue;
        }
        count.compared += 1;
        if actual == expected {
            count.matched += 1;
        } else if names_only(
            &expected,
            &actual,
            pipeline.lang().espeak_voice().and_then(|v| v.get(..2)).unwrap_or("en"),
        ) {
            // A name from names.json, read in its own language (docs/divergences.md, "misaki English: names"),
            // which may move where a long sentence is cut and how the runs beside it are spaced.
            count.matched += 1;
            count.named += 1;
        } else if kokoro_reference::divergence::chunks_differ_only_in_run_spacing(&expected, &actual) {
            count.matched += 1;
            count.respaced += 1;
        } else if kokoro_reference::clause_regrouping(
            &expected,
            &actual,
            kokoro_text::MIN_CLAUSE_CHUNK,
            Some(&record.sentences),
        )
        .is_some()
        {
            count.matched += 1;
            count.regrouped += 1;
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
    for (lang, c) in &counts {
        println!(
            "{lang}: {}/{} compared records match ({} with short clauses regrouped, {} spaced as runs join, {} with names), of {} ({} chunks, {} characters); not compared: {:?}",
            c.matched,
            c.compared,
            c.regrouped,
            c.respaced,
            c.named,
            c.records,
            c.chunks,
            c.chars,
            c.skipped
                .iter()
                .map(|(reason, ids)| format!("{} {reason} (e.g. {:?})", ids.len(), &ids[..ids.len().min(3)]))
                .collect::<Vec<_>>()
        );
    }
    println!("{} records in {elapsed:.2?}", records.len());
    for m in mismatches.iter().take(show) {
        println!("{m}");
    }
    assert!(mismatches.is_empty(), "{} records differ from the reference", mismatches.len());
}
