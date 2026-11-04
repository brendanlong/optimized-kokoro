//! Times `text_to_phonemes` over the texts of one voice in a calls file (as for `compare`):
//! `bench CALLS.jsonl VOICE [DATA_DIR] [REPEATS]`. Prints the best of the repeats.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let voice = &args[2];
    let dir =
        args.get(3).cloned().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let repeats: usize = args.get(4).map_or(3, |s| s.parse().unwrap());
    let texts: Vec<String> = std::fs::read_to_string(&args[1])
        .unwrap()
        .lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (v["voice"] == voice.as_str()).then(|| v["text"].as_str().unwrap().to_string())
        })
        .collect();
    let load = Instant::now();
    let data = EspeakData::load(&dir).unwrap();
    let mut e = Espeak::new(data);
    e.set_voice_by_name(voice).unwrap();
    let setup = load.elapsed();
    let words: usize = texts.iter().map(|t| t.split_whitespace().count().max(1)).sum();
    let mut best = f64::MAX;
    let mut sink = 0;
    for _ in 0..repeats {
        let start = Instant::now();
        for t in &texts {
            sink += e.text_to_phonemes(t, PHONEMES_IPA_TIE).len();
        }
        best = best.min(start.elapsed().as_secs_f64());
    }
    println!(
        "{voice} {} calls {} words {:.3} s {:.2} us/call {:.2} us/word (setup {:.1} ms, sink {sink})",
        texts.len(),
        words,
        best,
        best * 1e6 / texts.len() as f64,
        best * 1e6 / words as f64,
        setup.as_secs_f64() * 1e3
    );
}
