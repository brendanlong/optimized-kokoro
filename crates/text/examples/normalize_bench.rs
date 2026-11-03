//! Times normalization of a corpus file (`reference/dump.py` input or output JSONL), or of a flood.
//!
//! cargo run --release -p kokoro-text --example normalize_bench -- reference/corpus/gutenberg.jsonl

use kokoro_text::{Options, join_lines, split};
use std::time::Instant;

fn main() {
    let arg = std::env::args().nth(1).expect("a JSONL corpus, or a character to repeat");
    let texts: Vec<String> = if std::path::Path::new(&arg).exists() {
        std::fs::read_to_string(&arg)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()["text"].as_str().unwrap().to_owned())
            .collect()
    } else {
        vec![arg.repeat(100_000 / arg.len())]
    };
    let options = Options::default();
    let chars: usize = texts.iter().map(|t| t.chars().count()).sum();
    let start = Instant::now();
    for text in &texts {
        std::hint::black_box(split::normalize(&join_lines(text), "a", &options));
    }
    let elapsed = start.elapsed();
    println!("{} texts, {chars} chars in {elapsed:?} ({:.1} MB/s)", texts.len(), chars as f64 / elapsed.as_secs_f64() / 1e6);
}
