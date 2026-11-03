//! Time per G2P call over the recorded English calls (`cargo bench -p kokoro-g2p-en`), excluding spaCy
//! and espeak, whose recorded answers are replayed. Set `KOKORO_REFERENCE_OUT` to include corpus dumps.

use kokoro_g2p_en::{Fallback, G2p, Preprocessed, TaggedToken};
use std::collections::HashMap;
use std::hint::black_box;
use std::time::Instant;

struct Answers(HashMap<String, Option<String>>);

impl Fallback for Answers {
    fn phonemize(&mut self, word: &str) -> Option<String> {
        self.0.get(word).cloned().flatten()
    }
}

fn main() {
    let start = Instant::now();
    let us = G2p::new(false);
    println!("load (US lexicon): {:.1} ms", start.elapsed().as_secs_f64() * 1e3);
    let start = Instant::now();
    let gb = G2p::new(true);
    println!("load (GB lexicon): {:.1} ms", start.elapsed().as_secs_f64() * 1e3);

    let mut cases = Vec::new();
    let mut answers = HashMap::new();
    for record in kokoro_reference::all().into_iter().filter(|r| r.lang == "a" || r.lang == "b") {
        for call in record.calls {
            let [(segment, None)] = call.segments.as_slice() else { continue };
            let pre = Preprocessed::new(segment);
            let tagged: Vec<TaggedToken> = call.spacy[0]
                .iter()
                .map(|t| TaggedToken { text: t.text.clone(), tag: t.tag.clone(), whitespace: t.whitespace.clone() })
                .collect();
            answers.extend(call.fallback);
            cases.push((record.lang == "b", pre, tagged));
        }
    }
    let mut fallback = Answers(answers);
    let tokens: usize = cases.iter().map(|(_, _, t)| t.len()).sum();
    let rounds = (200_000 / cases.len().max(1)).clamp(1, 50);
    let mut best = f64::INFINITY;
    for _ in 0..rounds {
        let start = Instant::now();
        for (british, pre, tagged) in &cases {
            let g2p = if *british { &gb } else { &us };
            black_box(g2p.phonemize_tagged(pre, tagged.clone(), Some(&mut fallback)).unwrap());
        }
        best = best.min(start.elapsed().as_secs_f64());
    }
    println!(
        "{} calls, {} spaCy tokens: {:.1} µs per call, {:.2} µs per token (best of {rounds})",
        cases.len(),
        tokens,
        best * 1e6 / cases.len() as f64,
        best * 1e6 / tokens as f64
    );
}
