//! Compares with misaki's regex-based subtokenize, recorded by tools/gen_subtokenize.py.

#[test]
fn matches_misaki_regex() {
    let data = include_str!("data/subtokenize.jsonl");
    let mut failures = Vec::new();
    for line in data.lines() {
        let (word, expected): (String, Vec<String>) = serde_json::from_str(line).unwrap();
        let got = kokoro_g2p_en::subtokenize(&word);
        if got != expected {
            failures.push(format!("{word:?}: expected {expected:?}, got {got:?}"));
        }
    }
    assert!(failures.is_empty(), "{} differ:\n{}", failures.len(), failures[..failures.len().min(20)].join("\n"));
}
