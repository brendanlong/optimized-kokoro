//! Times `ZhG2p::phonemize` on each sentence of a corpus (`{"text": ...}` lines):
//! `cargo run --release -p kokoro-g2p-zh --example bench -- reference/corpus/wiki-zh.jsonl`.
//! `reference/zh_bench.py` times misaki's `ZHG2P` on the same sentences.

use kokoro_g2p_zh::ZhG2p;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("a corpus file");
    let start = Instant::now();
    let g2p = ZhG2p::new();
    eprintln!("load {:.2?}", start.elapsed());
    let mut sentences = Vec::new();
    for line in std::fs::read_to_string(&path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        let text = record["text"].as_str().unwrap();
        // The sentences `reference/zh_bench.py` cuts: up to and including 。！？
        let mut rest = text;
        while !rest.is_empty() {
            let end = rest.find(['。', '！', '？']).map_or(rest.len(), |i| i + 3);
            sentences.push(rest[..end].to_owned());
            rest = &rest[end..];
        }
    }
    let repeat = 5;
    let mut chars = 0;
    let start = Instant::now();
    for _ in 0..repeat {
        for s in &sentences {
            chars += s.chars().count();
            let _ = std::hint::black_box(g2p.phonemize(s));
        }
    }
    let elapsed = start.elapsed().as_secs_f64() * 1e6;
    let n = sentences.len() * repeat;
    println!("{} sentences: {:.1} µs per sentence, {:.3} µs per character", sentences.len(), elapsed / n as f64, elapsed / chars as f64);
}
