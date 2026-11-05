//! Each line of stdin through the pipeline, as one JSON line of chunks: for surveys, before/after
//! comparisons and rendering audio.
//!
//! `cargo run --release -p kokoro-pipeline --example phonemize [-- LANG [MAX_TOKENS]] < texts.txt`,
//! where a line is a JSON string or plain text. Each output line is
//! `{"text", "chunks": [{"text", "phonemes", "ids"}], "error"}`; `LANG` is a pipeline code
//! (default `a`), and sentences of more than `MAX_TOKENS` phonemes are cut at clauses (default
//! 450, the reference's).

use kokoro_pipeline::{Lang, NormalizationOptions, Pipeline};
use std::io::{BufRead, Write};

fn main() {
    let code = std::env::args().nth(1).unwrap_or_else(|| "a".into());
    let max_tokens = std::env::args().nth(2).map_or(kokoro_pipeline::DEFAULT_MAX_TOKENS, |n| n.parse().unwrap());
    let pipeline = Pipeline::new(Lang::from_code(&code).expect("a pipeline language")).unwrap();
    let options = NormalizationOptions::default();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let text = serde_json::from_str::<String>(&line).unwrap_or(line);
        let mut chunks = Vec::new();
        let mut error = None;
        for chunk in pipeline.chunks_within(&text, &options, max_tokens) {
            match chunk {
                Ok(c) if c.pause.is_none() => chunks.push(serde_json::json!({
                    "text": c.text, "phonemes": c.phonemes, "ids": c.ids,
                })),
                Ok(_) => {}
                Err(e) => error = Some(e.to_string()),
            }
        }
        let record = serde_json::json!({ "text": text, "chunks": chunks, "error": error });
        writeln!(out, "{record}").unwrap();
    }
}
