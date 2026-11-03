//! Compares the engine with C espeak-ng's answers.
//!
//!     cargo run --release -p kokoro-espeak-ng --example compare -- CALLS.jsonl [DATA_DIR] [VOICE] [SHOW]
//!
//! CALLS.jsonl lines are `{"voice": "gmw/en-US", "text": "...", "clauses": ["...", ...]}`, the
//! clauses `espeak_TextToPhonemes` returned in phonemizer's mode (see `scratch/tools/oracle.py`).
//! Prints the exact-match rate per voice, per call and per word, then mismatches. Calls marked
//! `ub` (C read memory out of bounds or uninitialised; see `oracle.py --ub`) run but aren't scored;
//! the `ub` column counts them and `ub-same` how many still match. With `FRESH=1` each call gets
//! a new engine, as with `oracle.py fresh`.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Deserialize)]
struct Call {
    voice: String,
    text: String,
    #[serde(default)]
    clauses: Vec<String>,
    /// C crashed on this call; it restarts with fresh state.
    #[serde(default)]
    reset: bool,
    #[serde(default)]
    ub: Vec<String>,
}

#[derive(Default)]
struct Tally {
    calls: usize,
    calls_ok: usize,
    words: usize,
    words_ok: usize,
    mismatched_word_count: usize,
    ub: usize,
    ub_same: usize,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("calls.jsonl");
    let data_dir = args.get(2).cloned().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let only = args.get(3).filter(|s| !s.is_empty() && *s != "-").cloned();
    let show: usize = args.get(4).map_or(20, |s| s.parse().unwrap());
    let data = EspeakData::load(&data_dir).unwrap();
    let mut engines: HashMap<String, Espeak> = HashMap::new();
    let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
    let mut shown: HashMap<String, usize> = HashMap::new();
    let text = std::fs::read_to_string(path).unwrap();
    let fresh = std::env::var_os("FRESH").is_some();
    let start = std::time::Instant::now();
    for line in text.lines() {
        let call: Call = serde_json::from_str(line).unwrap();
        if only.as_ref().is_some_and(|v| *v != call.voice) {
            continue;
        }
        if fresh || call.reset {
            engines.remove(&call.voice);
        }
        if call.reset {
            continue;
        }
        let e = engines.entry(call.voice.clone()).or_insert_with(|| {
            let mut e = Espeak::new(data.clone());
            e.set_voice_by_name(&call.voice).unwrap();
            e
        });
        let got = e.text_to_phonemes(&call.text, PHONEMES_IPA_TIE);
        let t = tallies.entry(call.voice.clone()).or_default();
        if !call.ub.is_empty() {
            t.ub += 1;
            t.ub_same += usize::from(got == call.clauses);
            if got != call.clauses && std::env::var_os("SHOW_UB").is_some() {
                println!("ub {} {:?}\n  {:?}", call.voice, call.text, call.ub);
            }
            continue;
        }
        t.calls += 1;
        let exp_words: Vec<&str> = call.clauses.iter().flat_map(|c| c.split(' ')).collect();
        let got_words: Vec<&str> = got.iter().flat_map(|c| c.split(' ')).collect();
        t.words += exp_words.len();
        if exp_words.len() == got_words.len() {
            t.words_ok += exp_words.iter().zip(&got_words).filter(|(a, b)| a == b).count();
        } else {
            t.mismatched_word_count += 1;
        }
        if got == call.clauses {
            t.calls_ok += 1;
        } else {
            let n = shown.entry(call.voice.clone()).or_default();
            if *n < show {
                *n += 1;
                let (a, b) = (call.clauses.join("\n"), got.join("\n"));
                let a: Vec<char> = a.chars().collect();
                let b: Vec<char> = b.chars().collect();
                let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
                let from = common.saturating_sub(40);
                let tail = |v: &[char]| v[from..v.len().min(common + 60)].iter().collect::<String>();
                println!("{} {:?}\n  C:  …{:?}\n  rs: …{:?}", call.voice, call.text, tail(&a), tail(&b));
            }
        }
    }
    let elapsed = start.elapsed();
    println!("{:<12} {:>17} {:>19} {:>8} {:>6} {:>7}", "voice", "calls", "words", "badcount", "ub", "ub-same");
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    for (voice, t) in &tallies {
        println!(
            "{voice:<12} {:>7}/{:<7} {:>6.2}% {:>8}/{:<8} {:>6.2}% {:>6} {:>6} {:>7}",
            t.calls_ok,
            t.calls,
            pct(t.calls_ok, t.calls),
            t.words_ok,
            t.words,
            pct(t.words_ok, t.words),
            t.mismatched_word_count,
            t.ub,
            t.ub_same
        );
    }
    eprintln!("{:.2?}", elapsed);
}
