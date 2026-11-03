//! Times `Pipeline::chunks` end to end.
//!
//! `cargo run --release -p kokoro-pipeline --example bench -- [FILE|-] [REPEAT]`: with a file, times
//! it as one request (`-` reads stdin). `--dump DUMP...` times each American English record of
//! those reference dumps as a request; with no arguments, every one `kokoro_reference::all` finds.
//! `reference/pipeline_bench.py` times the reference the same way. `KOKORO_BENCH_LANG=z` (or `b`)
//! times another pipeline, and takes that language's records.

use kokoro_pipeline::{Lang, NormalizationOptions, Pipeline};
use std::io::Read;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let start = Instant::now();
    let code = std::env::var("KOKORO_BENCH_LANG").unwrap_or_else(|_| "a".into());
    let lang = Lang::from_code(&code).expect("a pipeline language");
    let pipeline = Pipeline::new(lang).unwrap();
    let options = NormalizationOptions::default();
    pipeline.chunks("Warm up the lexicon and espeak data, Mr. Qzxv. 银行行长。", &options).for_each(drop);
    eprintln!("load {:.2?}", start.elapsed());
    let texts: Vec<String> = match args.first().map(String::as_str) {
        Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s).unwrap();
            vec![s]
        }
        Some("--dump") => args[1..]
            .iter()
            .flat_map(kokoro_reference::read)
            .filter(|r| r.lang == code)
            .map(|r| r.text)
            .collect(),
        Some(path) => vec![std::fs::read_to_string(path).unwrap()],
        None => kokoro_reference::all().into_iter().filter(|r| r.lang == code).map(|r| r.text).collect(),
    };
    let repeat: usize = match args.first().map(String::as_str) {
        Some("--dump") => 1,
        _ => args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1),
    };
    let (mut chunks, mut chars) = (0, 0);
    let start = Instant::now();
    for _ in 0..repeat {
        for text in &texts {
            chars += text.chars().count();
            chunks += pipeline.chunks(text, &options).filter(|c| c.as_ref().is_ok_and(|c| c.pause.is_none())).count();
        }
    }
    let elapsed = start.elapsed();
    let requests = texts.len() * repeat;
    println!(
        "{requests} requests, {chunks} chunks, {chars} characters: {:.1} µs per request, {:.1} µs per chunk, {:.2} µs per character",
        elapsed.as_secs_f64() * 1e6 / requests as f64,
        elapsed.as_secs_f64() * 1e6 / chunks as f64,
        elapsed.as_secs_f64() * 1e6 / chars as f64,
    );
}
