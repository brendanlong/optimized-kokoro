//! Each line of stdin through the pipeline, as one JSON line of chunks: for surveys, before/after
//! comparisons and rendering audio.
//!
//! `cargo run --release -p kokoro-pipeline --example phonemize [-- LANG] < texts.txt`, where a
//! line is a JSON string or plain text. Each output line is
//! `{"text", "chunks": [{"text", "phonemes", "ids"}], "error"}`; `LANG` is a pipeline code
//! (default `a`).

use kokoro_pipeline::{Lang, NormalizationOptions, Pipeline};
use std::io::{BufRead, Write};

fn main() {
    let code = std::env::args().nth(1).unwrap_or_else(|| "a".into());
    let pipeline = Pipeline::new(Lang::from_code(&code).expect("a pipeline language")).unwrap();
    let options = NormalizationOptions::default();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let text = serde_json::from_str::<String>(&line).unwrap_or(line);
        let mut chunks = Vec::new();
        let mut error = None;
        for chunk in pipeline.chunks(&text, &options) {
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
