//! Against the reference: MeCab's words (fugashi on UniDic 3.1.0) and `JAG2P()(text)[0]` for the
//! texts in `tests/fixtures/` and, when set, every `*.mecab.jsonl` and `*.jag2p.jsonl` in
//! `$KOKORO_JA_REFERENCE` (written by `tools/reference.py`).
//!
//! Needs the dictionary (`tools/fetch_unidic.sh`); without it the tests say so and pass.
//! Records where misaki raises are checked against the readings `docs/divergences.md` lists.
//! `PARITY_SHOW=n` prints up to n mismatches (default 10).

use kokoro_g2p_ja::{JaG2p, default_dictionary};
use serde_json::Value;
use std::path::PathBuf;
use std::time::Instant;

fn g2p() -> Option<JaG2p> {
    match default_dictionary() {
        Ok(d) => Some(JaG2p::new(d)),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn files(suffix: &str) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")];
    if let Some(d) = std::env::var_os("KOKORO_JA_REFERENCE") {
        dirs.push(d.into());
    }
    let mut out = Vec::new();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        let mut found: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.to_string_lossy().ends_with(suffix))
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

fn records(path: &PathBuf) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn show() -> usize {
    std::env::var("PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(10)
}

#[test]
fn mecab_words_match_fugashi() {
    let Some(g2p) = g2p() else { return };
    let (mut total, mut bad, mut crashed) = (0, 0, 0);
    let start = Instant::now();
    for path in files(".mecab.jsonl") {
        for r in records(&path) {
            let text = r["text"].as_str().unwrap();
            if text.contains(&" ".repeat(60000)) {
                // After 65,535 bytes of whitespace MeCab cuts a word inside a character, which
                // fugashi fails to decode, or a node's 16-bit length overflows and the lattice
                // loses its words. JAG2P never gives MeCab such a run (cutlet::shorten_spaces).
                g2p.morphs(text).unwrap();
                crashed += 1;
                continue;
            }
            assert!(r["error"].is_null(), "{text:?}: {}", r["error"]);
            total += 1;
            let ours: Vec<Value> = g2p
                .morphs(text)
                .unwrap()
                .into_iter()
                .map(|m| {
                    let reading = m.reading.map_or_else(|| kokoro_g2p_ja::cutlet::kata2hira(m.surface), str::to_owned);
                    serde_json::json!([m.surface, reading, m.char_type, m.unknown])
                })
                .collect();
            if ours != r["morphs"].as_array().unwrap().as_slice() {
                bad += 1;
                if bad <= show() {
                    let theirs = r["morphs"].as_array().unwrap();
                    let i = ours.iter().zip(theirs).position(|(a, b)| a != b).unwrap_or(ours.len().min(theirs.len()));
                    eprintln!(
                        "{}: {text:?}\n  ours   {:?}\n  theirs {:?}",
                        path.display(),
                        &ours[i..ours.len().min(i + 4)],
                        &theirs[i..theirs.len().min(i + 4)]
                    );
                }
            }
        }
    }
    eprintln!("mecab: {}/{total} match, {crashed} where MeCab overflows ({:.1?})", total - bad, start.elapsed());
    assert_eq!(bad, 0);
}

/// Where misaki raises, the reading `docs/divergences.md` gives instead.
fn divergent(error: &str) -> bool {
    // `Convert` answers a number of more than nine digits with "Number length too long, choose
    // less than 10 digits", whose "10" cutlet then asserts isn't a digit surface; a digit other
    // than ASCII's after NFKC makes `Convert` raise KeyError.
    error.starts_with("AssertionError") || error.starts_with("KeyError")
}

#[test]
fn jag2p_matches_misaki() {
    let Some(g2p) = g2p() else { return };
    let (mut total, mut bad, mut errors) = (0, 0, 0);
    let start = Instant::now();
    for path in files(".jag2p.jsonl") {
        for r in records(&path) {
            total += 1;
            let text = r["text"].as_str().unwrap();
            let ours = g2p.phonemize(text).unwrap();
            if let Some(error) = r["error"].as_str() {
                errors += 1;
                assert!(divergent(error), "{text:?}: unexpected reference error {error}");
                continue;
            }
            let theirs = r["phonemes"].as_str().unwrap();
            if ours != theirs {
                bad += 1;
                if bad <= show() {
                    eprintln!("{}: {text:?}\n  ours   {ours:?}\n  theirs {theirs:?}", path.display());
                }
            }
        }
    }
    eprintln!(
        "jag2p: {}/{} match, {errors} where misaki raises ({:.1?})",
        total - errors - bad,
        total - errors,
        start.elapsed()
    );
    assert_eq!(bad, 0);
}

/// `*.fixed.jsonl`: JAG2P patched with the fixes `docs/divergences.md` lists
/// (`tools/reference.py fixed`), so the records where misaki raises are checked too.
#[test]
fn jag2p_matches_misaki_with_the_listed_fixes() {
    let Some(g2p) = g2p() else { return };
    let (mut total, mut bad) = (0, 0);
    for path in files(".fixed.jsonl") {
        for r in records(&path) {
            total += 1;
            let text = r["text"].as_str().unwrap();
            let ours = g2p.phonemize(text).unwrap();
            let theirs = r["phonemes"].as_str().unwrap_or_else(|| panic!("{text:?}: {}", r["error"]));
            if ours != theirs {
                bad += 1;
                if bad <= show() {
                    eprintln!("{}: {text:?}\n  ours   {ours:?}\n  theirs {theirs:?}", path.display());
                }
            }
        }
    }
    eprintln!("fixed: {}/{total} match", total - bad);
    assert_eq!(bad, 0);
}
