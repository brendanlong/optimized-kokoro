//! Time per call of each wrapper over the cases in an `espeak_dump.py` file:
//! `cargo run --release -p kokoro-espeak --example bench -- tests/espeak.jsonl`.

use kokoro_espeak::{EspeakFallback, EspeakG2P};
use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("usage: bench CASES.jsonl");
    let mut cases: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let key = (v["wrapper"].as_str().unwrap().to_string(), v["language"].as_str().unwrap().to_string());
        cases.entry(key).or_default().push(v["text"].as_str().unwrap().to_string());
    }
    for ((wrapper, language), texts) in cases {
        let run: Box<dyn Fn(&str)> = if wrapper == "fallback" {
            let w = EspeakFallback::new(language == "en-gb").unwrap();
            Box::new(move |t| {
                w.phonemize(t).unwrap();
            })
        } else {
            let w = EspeakG2P::new(&language).unwrap();
            Box::new(move |t| {
                w.phonemize(t).unwrap();
            })
        };
        run(&texts[0]);
        let start = Instant::now();
        for t in &texts {
            run(t);
        }
        let elapsed = start.elapsed();
        let words: usize = texts.iter().map(|t| t.split_whitespace().count()).sum();
        println!(
            "{wrapper:<8} {language:<6} {:>6} calls {:>8.1} µs/call {:>7.1} µs/word",
            texts.len(),
            elapsed.as_secs_f64() * 1e6 / texts.len() as f64,
            elapsed.as_secs_f64() * 1e6 / words.max(1) as f64
        );
    }
}
