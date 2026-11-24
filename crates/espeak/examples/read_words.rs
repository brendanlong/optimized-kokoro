//! Each line of stdin read by espeak in one language, as phonemizer gives misaki espeak's output
//! (`^` joins the two parts of an affricate or diphthong), one JSON line per line of input:
//! `[phonemes, switched]`, where `switched` says espeak read some of it in another language (a word
//! its rules take for English). For tools/names, which reads names in their own language and maps
//! the sounds to Kokoro's itself.
//!
//! `ESPEAK_DATA=DIR cargo run --release -p kokoro-espeak --example read_words -- LANGUAGE < words`,
//! where DIR is espeak-ng data with LANGUAGE's dictionary.

use kokoro_espeak::EspeakData;
use kokoro_espeak::phonemizer::{Backend, phonemize};
use std::io::{BufRead, Write};

fn main() {
    let language = std::env::args().nth(1).expect("usage: read_words LANGUAGE < words");
    let dir = std::env::var("ESPEAK_DATA").expect("ESPEAK_DATA: espeak-ng data with the language's dictionary");
    let mut backend = Backend::new(&language, EspeakData::load(dir).unwrap()).unwrap();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let mut switched = false;
        let raw = phonemize(&line.unwrap(), |chunk| {
            let ps = backend.espeak(chunk);
            switched |= ps.contains('(');
            ps
        })
        .unwrap_or_default();
        writeln!(out, "{}", serde_json::json!([raw.trim(), switched])).unwrap();
    }
}
