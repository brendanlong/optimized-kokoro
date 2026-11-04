//! Parity with the reference pipeline dumps (`reference/dump.py`): normalized text, sentences, and
//! chunks from `smart_split` with a G2P that answers from the recorded calls.

use kokoro_reference::Record;
use kokoro_text::{Chunk, DEFAULT_MAX_TOKENS, Limits, Options, join_lines, sentences, smart_split, split};
use std::cell::Cell;
use std::collections::HashMap;

struct Stage {
    name: &'static str,
    total: usize,
    mismatches: Vec<String>,
}

impl Stage {
    fn new(name: &'static str) -> Self {
        Self { name, total: 0, mismatches: Vec::new() }
    }

    fn check<T: PartialEq + std::fmt::Debug>(&mut self, id: &str, expected: &T, actual: &T) {
        self.total += 1;
        if expected != actual {
            self.mismatches.push(format!("{id}\n  expected: {expected:?}\n  actual:   {actual:?}"));
        }
    }

    fn report(&self) -> bool {
        let matched = self.total - self.mismatches.len();
        println!(
            "{}: {matched}/{} exact ({:.3}%)",
            self.name,
            self.total,
            100.0 * matched as f64 / self.total.max(1) as f64
        );
        for m in self.mismatches.iter().take(20) {
            println!("{m}");
        }
        self.mismatches.is_empty()
    }
}

/// `smart_split` with a G2P that answers from the record's calls. When the reference's G2P raised
/// (`record.error`), the first text it has no answer for is the one that raised, and the output
/// stops before the sentence that asked for it, as the reference's did.
fn chunks(record: &Record, calls: &mut Vec<String>) -> Vec<(String, String)> {
    let answers: HashMap<&str, &str> = record.calls.iter().map(|c| (c.input.as_str(), c.phonemes.as_str())).collect();
    let failed = Cell::new(false);
    let g2p = |text: &str| {
        if failed.get() {
            return String::new();
        }
        match answers.get(text) {
            Some(p) => {
                calls.push(text.to_owned());
                p.to_string()
            }
            None if record.error.is_some() => {
                failed.set(true);
                String::new()
            }
            None => {
                calls.push(text.to_owned());
                format!("<no recorded call for {text:?}>")
            }
        }
    };
    let options = Options::default();
    let mut out = Vec::new();
    for chunk in smart_split(&record.text, &record.lang, &options, &Limits::default(), DEFAULT_MAX_TOKENS, g2p) {
        if failed.get() {
            break;
        }
        if let Chunk::Speech(p) = chunk {
            out.push((p.text, p.phonemes));
        }
    }
    out
}

#[test]
fn reference_dumps() {
    let records = kokoro_reference::all();
    let mut normalized = Stage::new("normalized");
    let mut sentence_stage = Stage::new("sentences");
    let mut chunk_stage = Stage::new("chunks");
    let mut call_stage = Stage::new("G2P calls");
    let options = Options::default();
    let mut changed = 0;
    for record in &records {
        let actual = split::normalize(&join_lines(&record.text), &record.lang, &options);
        if actual != record.normalized {
            // Numbers and symbols are read differently on purpose (docs/divergences.md); every
            // other difference fails. Such records' chunks and calls follow from the new text.
            match kokoro_reference::divergence::explain(&record.text, &record.normalized, &actual) {
                Ok(_) => changed += 1,
                Err(why) => normalized.check(&record.id, &record.normalized, &format!("{actual} ({why})")),
            }
            continue;
        }
        normalized.check(&record.id, &record.normalized, &actual);
        let actual: Vec<String> = sentences(&record.normalized)
            .iter()
            .map(|s| kokoro_text::py::strip(s).to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        sentence_stage.check(&record.id, &record.sentences, &actual);
        let mut calls = Vec::new();
        let actual = chunks(record, &mut calls);
        let expected: Vec<(String, String)> =
            record.chunks.iter().map(|c| (c.text.clone(), c.phonemes.clone())).collect();
        chunk_stage.check(&record.id, &expected, &actual);
        let expected: Vec<String> = record.calls.iter().map(|c| c.input.clone()).collect();
        call_stage.check(&record.id, &expected, &calls);
    }
    println!("{} records; {changed} normalized differently by the number reader and symbol pass", records.len());
    let ok = [&normalized, &sentence_stage, &chunk_stage, &call_stage].map(Stage::report);
    assert!(ok.iter().all(|&ok| ok), "mismatches against the reference");
}
