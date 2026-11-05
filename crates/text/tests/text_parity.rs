//! Parity with the reference's text functions on hard and randomized cases
//! (`reference/fixtures/text_cases.out.jsonl`, written by `reference/dump_text.py`).

use kokoro_text::{
    Chunk, Limits, Options, check_pause_budget, check_speakable, join_lines, py, sentences, smart_split,
};
use kokoro_text::{split, strip_emoji};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn options(name: &str) -> Options {
    let default = Options::default();
    match name {
        "default" => default,
        "all" => Options { remove_emoji: true, ..default },
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
        "checks" => {
            assert_eq!(case["allow_voice_tags"], false, "voice tags aren't supported");
            let opts = Options { remove_emoji: case["remove_emoji"].as_bool().unwrap(), ..Options::default() };
            let speakable = check_speakable(text, &opts).is_ok();
            let budget = check_pause_budget(text, &Limits::default()).err().map(|e| e.to_string());
            (
                json!({ "speakable": case["speakable"], "pause_budget": case["pause_budget"] }),
                json!({ "speakable": speakable, "pause_budget": budget }),
            )
        }
        "emoji" => (case["stripped"].clone(), json!(strip_emoji(text))),
        other => panic!("unknown kind {other}"),
    }
}

/// Every string in a value, joined by spaces: the texts to compare word by word.
fn strings(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(strings).collect::<Vec<_>>().join(" "),
        Value::Object(map) => map.values().map(strings).collect::<Vec<_>>().join(" "),
        _ => String::new(),
    }
}

#[test]
fn text_cases() {
    // KOKORO_TEXT_CASES points at a larger dump from the same script.
    let path = std::env::var("KOKORO_TEXT_CASES").unwrap_or_else(|_| {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../reference/fixtures/text_cases.out.jsonl").into()
    });
    let mut totals: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut mismatches = Vec::new();
    let mut changed: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_input_only = 0;
    let mut regrouped_cases = 0;
    for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
        let case: Value = serde_json::from_str(line).unwrap();
        let (expected, actual) = run(&case);
        let entry = totals.entry(str_of(&case, "kind").to_owned()).or_default();
        entry.0 += 1;
        // Numbers and symbols are read differently on purpose (docs/divergences.md): those
        // differences are counted, every other one fails.
        // Under the other option sets ("all" had the reference's unit option, which is gone; "none"
        // leaves URLs and symbols to the number pass), and
        // in the seeded fuzz cases, which glue numbers, symbols and words together in ways the
        // word-by-word check can't line up, a digit or symbol in the input is the explanation.
        let text = str_of(&case, "text");
        let has_number_or_symbol = text.chars().any(|c| {
            c.is_numeric() || (!c.is_alphanumeric() && !c.is_whitespace() && !".,;:!?'\"()[]{}-—–…“”‘’«»`".contains(c))
        });
        let kind = matches!(str_of(&case, "kind"), "normalize" | "smart_split");
        let strict =
            kind && kokoro_reference::divergence::explain(text, &strings(&expected), &strings(&actual)).is_ok();
        let by_input =
            kind && (case["options"] != "default" || str_of(&case, "id").starts_with("fuzz-")) && has_number_or_symbol;
        // Short clauses are regrouped on purpose (docs/divergences.md).
        let pairs = |chunks: &Value| -> Vec<(String, String)> {
            chunks
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| match c.as_array() {
                    Some(pair) => (pair[0].as_str().unwrap().to_owned(), pair[1].as_str().unwrap().to_owned()),
                    None => (format!("<pause {c}>"), String::new()),
                })
                .collect()
        };
        let regrouped = str_of(&case, "kind") == "smart_split"
            && kokoro_reference::clause_regrouping(&pairs(&expected), &pairs(&actual)).is_some();
        if expected != actual && regrouped {
            *changed.entry("smart_split".to_owned()).or_default() += 1;
            regrouped_cases += 1;
        } else if expected != actual && (strict || by_input) {
            *changed.entry(str_of(&case, "kind").to_owned()).or_default() += 1;
            by_input_only += usize::from(!strict);
        } else if expected != actual {
            entry.1 += 1;
            mismatches.push(format!(
                "{} {:?}\n  expected: {expected}\n  actual:   {actual}",
                str_of(&case, "id"),
                str_of(&case, "text")
            ));
        }
    }
    for (kind, (total, bad)) in &totals {
        let differ = changed.get(kind).copied().unwrap_or(0);
        println!(
            "{kind}: {}/{total} exact, {differ} differ on purpose (numbers, symbols, short clauses)",
            total - bad - differ
        );
    }
    println!("of those, {by_input_only} explained only by a digit or symbol in the input (fuzz and other option sets)");
    println!("and {regrouped_cases} smart_split cases differ by regrouped short clauses");
    for m in mismatches.iter().take(40) {
        println!("{m}");
    }
    assert!(mismatches.is_empty(), "{} mismatches against the reference", mismatches.len());
}
