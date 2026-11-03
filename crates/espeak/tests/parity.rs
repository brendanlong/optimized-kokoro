//! Parity with misaki's espeak wrappers over C espeak-ng 1.52, from `reference/espeak_dump.py` output:
//! the committed `tests/espeak.jsonl`, plus `espeak*.jsonl` in `reference/out/` (or
//! `$KOKORO_REFERENCE_OUT`) when present. `tests/divergences.jsonl` lists the cases where the
//! engine intentionally differs from C (docs/divergences.md), with the engine's output.

use kokoro_espeak::{EspeakFallback, EspeakG2P};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Deserialize)]
struct Case {
    wrapper: String,
    language: String,
    text: String,
    /// `(text, phonemes)` for each call phonemizer made to C espeak.
    espeak: Vec<(String, String)>,
    phonemes: Option<String>,
}

fn cases() -> Vec<Case> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::var_os("KOKORO_REFERENCE_OUT")
        .map_or_else(|| root.join("../../reference/out"), PathBuf::from);
    let mut paths = vec![root.join("tests/espeak.jsonl")];
    if let Ok(entries) = std::fs::read_dir(out) {
        let mut extra: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|x| x == "jsonl")
                    && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("espeak"))
            })
            .collect();
        extra.sort();
        paths.extend(extra);
    }
    paths
        .iter()
        .flat_map(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
                .collect::<Vec<Case>>()
        })
        .collect()
}

#[derive(Deserialize)]
struct Divergence {
    wrapper: String,
    language: String,
    text: String,
    phonemes: String,
}

/// The engine's expected output for each listed divergence, by (wrapper, language, text).
fn divergences() -> HashMap<(String, String, String), String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/divergences.jsonl");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| {
            let d: Divergence = serde_json::from_str(l).unwrap();
            ((d.wrapper, d.language, d.text), d.phonemes)
        })
        .collect()
}

enum Wrapper {
    Fallback(EspeakFallback),
    G2P(EspeakG2P),
}

impl Wrapper {
    fn new(case: &Case) -> Self {
        match case.wrapper.as_str() {
            "fallback" => Wrapper::Fallback(EspeakFallback::new(case.language == "en-gb").unwrap()),
            "g2p" => Wrapper::G2P(EspeakG2P::new(&case.language).unwrap()),
            other => panic!("unknown wrapper {other}"),
        }
    }

    fn run(&self, text: &str, espeak: Option<&mut dyn FnMut(&str) -> kokoro_espeak::Result<String>>) -> Option<String> {
        match (self, espeak) {
            (Wrapper::Fallback(w), Some(e)) => w.phonemize_with(text, e).unwrap(),
            (Wrapper::Fallback(w), None) => w.phonemize(text).unwrap(),
            (Wrapper::G2P(w), Some(e)) => Some(w.phonemize_with(text, e).unwrap()),
            (Wrapper::G2P(w), None) => Some(w.phonemize(text).unwrap()),
        }
    }
}

#[derive(Default)]
struct Tally {
    total: usize,
    matched: usize,
    diverged: usize,
    chunks: usize,
    chunks_matched: usize,
}

fn report(tallies: &BTreeMap<(String, String), Tally>) {
    println!("{:<10} {:<7} {:>17} {:>17} {:>9}", "wrapper", "lang", "outputs", "espeak calls", "diverged");
    for ((wrapper, language), t) in tallies {
        let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
        println!(
            "{wrapper:<10} {language:<7} {:>7}/{:<6} {:>5.1}% {:>7}/{:<6} {:>5.1}% {:>9}",
            t.matched,
            t.total,
            pct(t.matched, t.total),
            t.chunks_matched,
            t.chunks,
            pct(t.chunks_matched, t.chunks),
            t.diverged
        );
    }
}

/// The wrapper logic alone: given C espeak's recorded answers, the wrappers must ask espeak the
/// same questions and turn its answers into exactly misaki's output.
#[test]
fn wrappers_match_misaki_given_c_espeak() {
    let mut wrappers = HashMap::new();
    let mut failures = Vec::new();
    for case in cases() {
        let wrapper = wrappers
            .entry((case.wrapper.clone(), case.language.clone()))
            .or_insert_with(|| Wrapper::new(&case));
        let mut calls = case.espeak.iter();
        let mut asked = Vec::new();
        let mut espeak = |chunk: &str| {
            asked.push(chunk.to_string());
            Ok(calls.next().filter(|(text, _)| text == chunk).map_or_else(String::new, |(_, ps)| ps.clone()))
        };
        let got = wrapper.run(&case.text, Some(&mut espeak));
        let expected_calls: Vec<_> = case.espeak.iter().map(|(t, _)| t.clone()).collect();
        if got != case.phonemes || asked != expected_calls {
            failures.push(format!(
                "{} {} {:?}\n  misaki: {:?} asked {:?}\n  rust:   {:?} asked {:?}",
                case.wrapper, case.language, case.text, case.phonemes, expected_calls, got, asked
            ));
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

/// End to end on kokoro-espeak-ng. Prints exact-match rates per wrapper and language, for the
/// wrappers' output and for each raw espeak call, then any mismatches (`KOKORO_ESPEAK_SHOW` per
/// language). Run with `--nocapture` for the report. A listed divergence counts as matched when
/// the output is the listed one (the `diverged` column counts them).
#[test]
fn parity_with_c_espeak() {
    let show: usize = std::env::var("KOKORO_ESPEAK_SHOW").map_or(20, |s| s.parse().unwrap());
    let mut wrappers = HashMap::new();
    let mut tallies: BTreeMap<(String, String), Tally> = BTreeMap::new();
    let mut shown: HashMap<(String, String), usize> = HashMap::new();
    let mut seen = std::collections::HashSet::new();
    let divergences = divergences();
    for case in cases() {
        let key = (case.wrapper.clone(), case.language.clone());
        if !seen.insert((key.clone(), case.text.clone())) {
            continue;
        }
        let wrapper = &*wrappers.entry(key.clone()).or_insert_with(|| Wrapper::new(&case));
        let backend = match wrapper {
            Wrapper::Fallback(w) => w.backend(),
            Wrapper::G2P(w) => w.backend(),
        };
        let got = wrapper.run(&case.text, None);
        let tally = tallies.entry(key.clone()).or_default();
        tally.total += 1;
        tally.chunks += case.espeak.len();
        tally.chunks_matched += case.espeak.iter().filter(|(text, c)| backend.espeak(text).unwrap() == *c).count();
        let listed = divergences.get(&(case.wrapper.clone(), case.language.clone(), case.text.clone()));
        if listed.is_some() {
            tally.diverged += 1;
        }
        let expected = listed.cloned().or_else(|| case.phonemes.clone());
        if got == expected {
            tally.matched += 1;
        } else {
            let n = shown.entry(key).or_default();
            if *n < show {
                *n += 1;
                println!(
                    "{} {} {:?}\n  C:  {}\n  rs: {}",
                    case.wrapper,
                    case.language,
                    case.text,
                    expected.as_deref().unwrap_or("None"),
                    got.as_deref().unwrap_or("None")
                );
            }
        }
    }
    report(&tallies);
    assert!(tallies.values().all(|t| t.matched == t.total));
}
