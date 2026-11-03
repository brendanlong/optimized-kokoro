//! Load time, memory and time per text of the Japanese G2P:
//! `cargo run --release -p kokoro-g2p-ja --example bench -- TEXTS.jsonl [REPEAT]`
//! (`{"text": ...}` lines; `tools/bench.py` times the reference on the same file).
use kokoro_g2p_ja::{Dictionary, JaG2p, default_dictionary_path};
use std::sync::Arc;
use std::time::Instant;

fn rss_mb() -> f64 {
    status_mb("VmRSS:")
}

/// A `/proc/self/status` size in MB (`RssAnon:` is private memory, `RssFile:` mapped file pages,
/// which the kernel can drop and re-read).
fn status_mb(key: &str) -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix(key))
        .and_then(|v| v.trim().trim_end_matches(" kB").trim().parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let before = rss_mb();
    let start = Instant::now();
    let dict = Dictionary::load(default_dictionary_path().expect("KOKORO_UNIDIC")).expect("dictionary");
    let load = start.elapsed();
    let g2p = JaG2p::new(Arc::new(dict));
    println!("load {load:.2?}, RSS {:.0} MB -> {:.0} MB", before, rss_mb());
    for (part, bytes) in g2p.dictionary().sizes() {
        println!("  {part:18} {:7.1} MB mapped", bytes as f64 / 1e6);
    }
    let Some(path) = args.get(1) else { return };
    let repeat: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let texts: Vec<String> = std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v["text"].as_str().map(str::to_owned))
        .collect();
    let chars: usize = texts.iter().map(|t| t.chars().count()).sum();
    let mut best = f64::MAX;
    for _ in 0..repeat {
        let start = Instant::now();
        for t in &texts {
            std::hint::black_box(g2p.phonemize(t).unwrap());
        }
        best = best.min(start.elapsed().as_secs_f64());
    }
    println!(
        "{} texts, {chars} chars: {:.1} µs per text, {:.2} µs per char (best of {repeat})",
        texts.len(),
        best * 1e6 / texts.len() as f64,
        best * 1e6 / chars as f64
    );
    let normalized: Vec<String> = texts
        .iter()
        .flat_map(|t| kokoro_g2p_ja::cutlet::normalize(t))
        .filter_map(|p| match p {
            kokoro_g2p_ja::cutlet::Part::Text(t) => Some(t),
            _ => None,
        })
        .collect();
    let start = Instant::now();
    for t in &normalized {
        std::hint::black_box(g2p.morphs(t).unwrap());
    }
    println!(
        "  RSS after the texts: {:.0} MB ({:.0} MB private, {:.0} MB mapped file pages)",
        rss_mb(),
        status_mb("RssAnon:"),
        status_mb("RssFile:")
    );
    println!("  of which MeCab: {:.1} µs per text", start.elapsed().as_secs_f64() * 1e6 / texts.len() as f64);
}
