//! End to end against the reference server: for every English record `reference/dump.py` wrote
//! (the committed hard cases, plus `reference/out/*.jsonl` or `$KOKORO_REFERENCE_OUT`), the chunks'
//! text and phonemes must equal the recorded `smart_split` output.
//!
//! Records whose text has Chinese or Japanese runs are counted apart: there's no G2P for them yet,
//! so the pipeline reports [`Error::Unsupported`] for those sentences. `PARITY_SHOW=n` prints up to
//! n mismatches (default 20).

use kokoro_pipeline::{Error, Lang, NormalizationOptions, Pipeline};
use kokoro_reference::Record;
use std::time::Instant;

fn uses_cjk(record: &Record) -> bool {
    record.calls.iter().flat_map(|c| &c.segments).any(|(_, l)| matches!(l.as_deref(), Some("zh" | "ja")))
}

#[test]
fn reference_records() {
    let records: Vec<Record> = kokoro_reference::all().into_iter().filter(|r| Lang::from_code(&r.lang).is_some()).collect();
    let american = Pipeline::new(Lang::AmericanEnglish).unwrap();
    let british = Pipeline::new(Lang::BritishEnglish).unwrap();
    let options = NormalizationOptions::default();
    let show: usize = std::env::var("PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    let (mut matched, mut compared, mut cjk, mut cjk_unsupported, mut chars) = (0, 0, 0, 0, 0);
    let mut mismatches = Vec::new();
    let start = Instant::now();
    for record in &records {
        let pipeline = if record.lang == "b" { &british } else { &american };
        let mut actual = Vec::new();
        let mut errors = Vec::new();
        for chunk in pipeline.chunks(&record.text, &options) {
            match chunk {
                Ok(c) if c.pause.is_none() => actual.push((c.text, c.phonemes)),
                Ok(_) => {}
                Err(e) => errors.push(e),
            }
        }
        chars += record.text.chars().count();
        let unsupported = errors.iter().any(|e| matches!(e, Error::Unsupported { .. }));
        if uses_cjk(record) || unsupported {
            cjk += 1;
            cjk_unsupported += usize::from(unsupported);
            continue;
        }
        compared += 1;
        let expected: Vec<(String, String)> = record.chunks.iter().map(|c| (c.text.clone(), c.phonemes.clone())).collect();
        if errors.is_empty() && actual == expected {
            matched += 1;
        } else {
            let first = expected.iter().zip(&actual).position(|(e, a)| e != a).unwrap_or(expected.len().min(actual.len()));
            mismatches.push(format!(
                "{} ({} vs {} chunks, first difference at {first}; errors {errors:?})\n  expected: {:?}\n  actual:   {:?}",
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
        "{matched}/{compared} English records match exactly ({:.3}%); {cjk} more have Chinese or Japanese runs ({cjk_unsupported} gave Error::Unsupported)",
        100.0 * matched as f64 / compared.max(1) as f64
    );
    println!("{} records, {chars} characters in {:.2?}", records.len(), elapsed);
    for m in mismatches.iter().take(show) {
        println!("{m}");
    }
    assert!(mismatches.is_empty(), "{} records differ from the reference", mismatches.len());
}
