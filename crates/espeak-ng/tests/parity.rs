//! The engine against C espeak-ng 1.52's answers.
//!
//! `c_calls.jsonl` holds `espeak_TextToPhonemes` results from C in phonemizer's mode: corpus
//! words, adversarial texts and minimized regression cases for the voices whose data ships with
//! the crate. Calls replay in file order with one engine per voice, since C carries state from
//! one call to the next; `{"reset": true}` lines start that voice afresh.
//!
//! `ESPEAK_NG_CALLS` adds more call files (separated by `:`), read with the data in
//! `ESPEAK_NG_DATA` when set, e.g. the full espeak-ng-data that `reference/` installs.

use kokoro_espeak_ng::{default_data_dir, Espeak, EspeakData, PHONEMES_IPA_TIE};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Deserialize)]
struct Call {
    voice: String,
    text: String,
    #[serde(default)]
    clauses: Vec<String>,
    #[serde(default)]
    reset: bool,
}

fn replay(data: &Arc<EspeakData>, path: &Path) -> (usize, Vec<String>) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut engines: HashMap<String, Espeak> = HashMap::new();
    let mut calls = 0;
    let mut failures = Vec::new();
    for line in text.lines() {
        let call: Call = serde_json::from_str(line).unwrap();
        if call.reset {
            engines.remove(&call.voice);
            continue;
        }
        let e = engines.entry(call.voice.clone()).or_insert_with(|| {
            let mut e = Espeak::new(data.clone());
            e.set_voice_by_name(&call.voice).unwrap();
            e
        });
        calls += 1;
        let got = e.text_to_phonemes(&call.text, PHONEMES_IPA_TIE);
        if got != call.clauses {
            failures.push(format!("{} {:?}\n  C:    {:?}\n  Rust: {:?}", call.voice, call.text, call.clauses, got));
        }
    }
    (calls, failures)
}

fn check(data: &Arc<EspeakData>, path: &Path) {
    let (calls, failures) = replay(data, path);
    assert!(calls > 0, "{}: no calls", path.display());
    assert!(
        failures.is_empty(),
        "{}: {} of {calls} calls differ from C:\n{}",
        path.display(),
        failures.len(),
        failures.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn matches_c_espeak() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    check(&data, &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/c_calls.jsonl"));
}

#[test]
fn matches_c_espeak_on_extra_call_files() {
    let Some(files) = std::env::var_os("ESPEAK_NG_CALLS") else {
        return;
    };
    let dir = std::env::var_os("ESPEAK_NG_DATA").map_or_else(default_data_dir, Into::into);
    let data = EspeakData::load(dir).unwrap();
    for path in std::env::split_paths(&files) {
        check(&data, &path);
    }
}

#[test]
fn engines_share_data_across_threads() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    let handles: Vec<_> = ["gmw/en-US", "roa/fr", "zle/ru", "ko"]
        .into_iter()
        .map(|voice| {
            let data = data.clone();
            std::thread::spawn(move || {
                let mut e = Espeak::new(data);
                e.set_voice_by_name(voice).unwrap();
                e.text_to_phonemes("123, abc.", PHONEMES_IPA_TIE)
            })
        })
        .collect();
    for h in handles {
        let clauses = h.join().unwrap();
        assert_eq!(clauses.len(), 2);
        assert!(clauses.iter().all(|c| !c.is_empty()));
    }
}

#[test]
fn unknown_voice_is_an_error() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    let mut e = Espeak::new(data);
    assert!(e.set_voice_by_name("no-such-voice").is_err());
    assert!(e.text_to_phonemes("hello", PHONEMES_IPA_TIE).is_empty());
}
