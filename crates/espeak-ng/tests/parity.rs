//! The engine against C espeak-ng 1.52's answers.
//!
//! `c_calls.jsonl` holds `espeak_TextToPhonemes` results from C in phonemizer's mode: corpus
//! words, adversarial texts and minimized regression cases for the voices whose data ships with
//! the crate, each from a C process forked after `espeak_SetVoiceByName` (`oracle.py pairs`), as
//! each engine call starts from that state. `{"reset": true}` lines are calls where C crashed.
//! Calls marked `ub` (C read memory out of bounds or uninitialised) run but aren't compared.
//!
//! `ESPEAK_NG_CALLS` adds more call files (separated by `:`), read with the data in
//! `ESPEAK_NG_DATA` when set, e.g. the full espeak-ng-data that `reference/` installs.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE, default_data_dir};
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
    /// C read out of bounds or uninitialised memory, so its clauses aren't a reference.
    #[serde(default)]
    ub: Vec<String>,
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
        let got = e.text_to_phonemes(&call.text, PHONEMES_IPA_TIE);
        if !call.ub.is_empty() {
            continue;
        }
        calls += 1;
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

/// C reads uninitialised stack memory after a clause when grouping digits, so the digits an
/// earlier text left there changed how a later text's number was read. The port zeroes those
/// buffers per clause (docs/divergences.md), so the later text reads the same either way.
#[test]
fn earlier_text_does_not_change_number_reading() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    let earlier = "6_970_053_455_318_812_34675\u{2009}060\u{2009}553\u{2009}8906811777883416022637791712286849618916638673633541247516847\u{2009}398\u{2009}090\u{2009}888\u{2009}190\u{439}";
    let later =
        "1247328568694660478413776674155675977384639814658857044049129367935696294763049432641823903136079361-10127";
    let run = |texts: &[&str]| {
        let mut e = Espeak::new(data.clone());
        e.set_voice_by_name("roa/fr").unwrap();
        texts.iter().map(|t| e.text_to_phonemes(t, PHONEMES_IPA_TIE)).last().unwrap()
    };
    assert_eq!(run(&[earlier, later]), run(&[later]));
}

/// In C, `\x01B` (speak punctuation) and `\x01…Y` (say-as) stay on for later calls, and a
/// character read ahead at the end of one call starts the next. Each call here starts fresh
/// (docs/divergences.md), so no earlier text changes a later call.
#[test]
fn earlier_text_does_not_change_later_calls() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    let later = "Hello, world. 42 cats.";
    let mut fresh = Espeak::new(data.clone());
    fresh.set_voice_by_name("gmw/en-US").unwrap();
    let expected = fresh.text_to_phonemes(later, PHONEMES_IPA_TIE);
    assert_eq!(expected, ["həlˈo͡ʊ", "wˈɜːld", "fˈɔː͡ɹɾi tˈuː kˈæts"]);
    let mut e = Espeak::new(data);
    e.set_voice_by_name("gmw/en-US").unwrap();
    for earlier in ["\\\x01B.,", "xx \x011B yy, zz.", "\x012Y abc, def.", "abc.", "[[h@l'oU", "\x01"] {
        e.text_to_phonemes(earlier, PHONEMES_IPA_TIE);
        assert_eq!(e.text_to_phonemes(later, PHONEMES_IPA_TIE), expected, "after {earlier:?}");
    }
}

#[test]
fn unknown_voice_is_an_error() {
    let data = EspeakData::load(default_data_dir()).unwrap();
    let mut e = Espeak::new(data);
    assert!(e.set_voice_by_name("no-such-voice").is_err());
    assert!(e.text_to_phonemes("hello", PHONEMES_IPA_TIE).is_empty());
}
