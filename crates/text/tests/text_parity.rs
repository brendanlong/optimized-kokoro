//! Parity with the reference's text functions on hard and randomized cases
//! (`reference/fixtures/text_cases.out.jsonl`, written by `reference/dump_text.py`).

use kokoro_text::{Chunk, Limits, Options, check_pause_budget, check_speakable, join_lines, py, sentences, smart_split};
use kokoro_text::{split, split_by_voice, ssml, strip_emoji};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn options(name: &str) -> Options {
    let default = Options::default();
    match name {
        "default" => default,
        "all" => Options { unit_normalization: true, remove_emoji: true, ..default },
        "none" => Options {
            url_normalization: false,
            email_normalization: false,
            optional_pluralization_normalization: false,
            phone_normalization: false,
            caps_normalization: false,
            replace_remaining_symbols: false,
            ..default
        },
        "off" => Options { normalize: false, ..default },
        other => panic!("unknown option set {other}"),
    }
}

fn str_of<'a>(case: &'a Value, key: &str) -> &'a str {
    case[key].as_str().unwrap_or_else(|| panic!("{key} missing from {case}"))
}

/// The expected fields of a case, and what the port produced for them.
fn run(case: &Value) -> (Value, Value) {
    let text = str_of(case, "text");
    let lang = case["lang"].as_str().unwrap_or("a");
    let opts = options(case["options"].as_str().unwrap_or("default"));
    match str_of(case, "kind") {
        "normalize" => {
            let normalized = split::normalize(&join_lines(text), lang, &opts);
            let actual = json!({ "normalized": normalized, "sentences": sentences(&normalized) });
            (json!({ "normalized": case["normalized"], "sentences": case["sentences"] }), actual)
        }
        "smart_split" => {
            let g2p = |t: &str| if t.chars().any(py::is_alnum) { t.to_owned() } else { String::new() };
            let max_tokens = case["max_tokens"].as_u64().unwrap() as usize;
            let chunks: Vec<Value> = smart_split(text, lang, &opts, &Limits::default(), max_tokens, g2p)
                .map(|c| match c {
                    Chunk::Speech(p) => json!([p.text, p.phonemes]),
                    Chunk::Pause(s) => json!(s),
                })
                .collect();
            (case["chunks"].clone(), json!(chunks))
        }
        "voices" => {
            let segments: Vec<Value> =
                split_by_voice(text, str_of(case, "voice")).into_iter().map(|s| json!([s.voice, s.rate, s.text])).collect();
            (case["segments"].clone(), json!(segments))
        }
        "checks" => {
            let opts = Options { remove_emoji: case["remove_emoji"].as_bool().unwrap(), ..Options::default() };
            let speakable = check_speakable(text, case["allow_voice_tags"].as_bool().unwrap(), Some(&opts)).is_ok();
            let budget = check_pause_budget(text, &Limits::default()).err().map(|e| e.to_string());
            (
                json!({ "speakable": case["speakable"], "pause_budget": case["pause_budget"] }),
                json!({ "speakable": speakable, "pause_budget": budget }),
            )
        }
        "emoji" => (case["stripped"].clone(), json!(strip_emoji(text))),
        "ssml" => {
            let result = ssml::translate(
                text,
                str_of(case, "voice"),
                case["allow_voice_tags"].as_bool().unwrap(),
                Limits::default().max_pause_duration_s,
            );
            let expected = if case.get("error").is_some() { json!("error") } else { case["translated"].clone() };
            (expected, result.map_or_else(|_| json!("error"), |s| json!(s)))
        }
        other => panic!("unknown kind {other}"),
    }
}

#[test]
fn text_cases() {
    // KOKORO_TEXT_CASES points at a larger dump from the same script.
    let path = std::env::var("KOKORO_TEXT_CASES")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/fixtures/text_cases.out.jsonl").into());
    let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut mismatches = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line).unwrap();
        let (expected, actual) = run(&case);
        let entry = totals.entry(str_of(&case, "kind").to_owned()).or_default();
        entry.0 += 1;
        if expected != actual {
            entry.1 += 1;
            mismatches.push(format!(
                "{} {:?}\n  expected: {expected}\n  actual:   {actual}",
                str_of(&case, "id"),
                str_of(&case, "text")
            ));
        }
    }
    for (kind, (total, bad)) in &totals {
        println!("{kind}: {}/{total} exact", total - bad);
    }
    for m in mismatches.iter().take(40) {
        println!("{m}");
    }
    assert!(mismatches.is_empty(), "{} mismatches against the reference", mismatches.len());
}
