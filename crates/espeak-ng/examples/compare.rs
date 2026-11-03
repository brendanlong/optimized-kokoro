//! Compares the engine with C espeak-ng's answers.
//!
//!     cargo run --release -p kokoro-espeak-ng --example compare -- CALLS.jsonl [DATA_DIR] [VOICE] [SHOW]
//!
//! CALLS.jsonl lines are `{"voice": "gmw/en-US", "text": "...", "clauses": ["...", ...]}`, the
//! clauses `espeak_TextToPhonemes` returned in phonemizer's mode (see `scratch/tools/oracle.py`).
//! Prints the exact-match rate per voice, per call and per word, then mismatches.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize)]
struct Call {
    voice: String,
    text: String,
    clauses: Vec<String>,
}

#[derive(Default)]
struct Tally {
    calls: usize,
    calls_ok: usize,
    words: usize,
    words_ok: usize,
    mismatched_word_count: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("calls.jsonl");
    let data_dir = args.get(2).cloned().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let only = args.get(3).filter(|s| !s.is_empty() && *s != "-").cloned();
    let show: usize = args.get(4).map_or(20, |s| s.parse().unwrap());
    let data = EspeakData::load(&data_dir).unwrap();
    let mut engines: HashMap<String, Espeak> = HashMap::new();
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut shown: HashMap<String, usize> = HashMap::new();
    let text = std::fs::read_to_string(path).unwrap();
    let start = std::time::Instant::now();
    for line in text.lines() {
        let call: Call = serde_json::from_str(line).unwrap();
        if only.as_ref().is_some_and(|v| *v != call.voice) {
            continue;
        }
        let e = engines.entry(call.voice.clone()).or_insert_with(|| {
            let mut e = Espeak::new(data.clone());
            e.set_voice_by_name(&call.voice).unwrap();
            e
        });
        let got = e.text_to_phonemes(&call.text, PHONEMES_IPA_TIE);
        let t = tallies.entry(call.voice.clone()).or_default();
        t.calls += 1;
        let exp_words: Vec<&str> = call.clauses.iter().flat_map(|c| c.split(' ')).collect();
        let got_words: Vec<&str> = got.iter().flat_map(|c| c.split(' ')).collect();
        t.words += exp_words.len();
        if exp_words.len() == got_words.len() {
            t.words_ok += exp_words.iter().zip(&got_words).filter(|(a, b)| a == b).count();
        } else {
            t.mismatched_word_count += 1;
        }
        if got == call.clauses {
            t.calls_ok += 1;
        } else {
            let n = shown.entry(call.voice.clone()).or_default();
            if *n < show {
                *n += 1;
                println!("{} {:?}\n  C:  {:?}\n  rs: {:?}", call.voice, call.text, call.clauses, got);
            }
        }
    }
    let elapsed = start.elapsed();
    println!("{:<12} {:>17} {:>19} {:>8}", "voice", "calls", "words", "badcount");
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    for (voice, t) in &tallies {
        println!(
            "{voice:<12} {:>7}/{:<7} {:>6.2}% {:>8}/{:<8} {:>6.2}% {:>6}",
            t.calls_ok,
            t.calls,
            pct(t.calls_ok, t.calls),
            t.words_ok,
            t.words,
            pct(t.words_ok, t.words),
            t.mismatched_word_count
        );
    }
    eprintln!("{:.2?}", elapsed);
}
