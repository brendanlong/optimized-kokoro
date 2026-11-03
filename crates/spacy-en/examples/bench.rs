//! Times tokenizing and tagging the English sentences in the reference dumps.
//!
//!     cargo run --release -p kokoro-spacy-en --example bench

use kokoro_spacy_en::Nlp;
use std::time::Instant;

fn main() {
    let start = Instant::now();
    let nlp = Nlp::new();
    eprintln!("load: {:.1} ms", start.elapsed().as_secs_f64() * 1e3);
    let sentences: Vec<String> = kokoro_reference::all()
        .into_iter()
        .filter(|r| r.lang == "a" || r.lang == "b")
        .flat_map(|r| r.sentences)
        .collect();
    let tokens: usize = sentences.iter().map(|s| nlp.tokenize(s).len()).sum();
    for (name, run) in [
        ("tokenize", &(|s: &str| nlp.tokenize(s).len()) as &dyn Fn(&str) -> usize),
        ("process", &|s: &str| nlp.process(s).len()),
    ] {
        let start = Instant::now();
        let n: usize = sentences.iter().map(|s| run(s)).sum();
        let secs = start.elapsed().as_secs_f64();
        assert_eq!(n, tokens);
        eprintln!(
            "{name}: {} sentences, {tokens} tokens in {secs:.2} s: {:.1} µs/sentence, {:.0} tokens/s",
            sentences.len(),
            secs * 1e6 / sentences.len() as f64,
            tokens as f64 / secs
        );
    }
}
