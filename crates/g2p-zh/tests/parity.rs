//! misaki's `ZHG2P()` on the committed cases (`reference/zh_dump.py --fixture`) and, when present,
//! on `reference/out/zh-g2p/*.jsonl` (or `$KOKORO_REFERENCE_OUT/zh-g2p`).

use kokoro_g2p_zh::ZhG2p;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Case {
    text: String,
    phonemes: Option<String>,
    error: Option<String>,
}

fn cases() -> Vec<(String, Case)> {
    let mut files = vec![(
        "tests/cases.jsonl".to_owned(),
        include_str!("cases.jsonl").to_owned(),
    )];
    let out = std::env::var_os("KOKORO_REFERENCE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/out"))
        .join("zh-g2p");
    if let Ok(entries) = std::fs::read_dir(&out) {
        let mut paths: Vec<_> = entries.map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        paths.sort();
        for p in paths {
            files.push((p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
        }
    }
    files
        .iter()
        .flat_map(|(name, data)| {
            data.lines().filter(|l| !l.trim().is_empty()).map(move |l| (name.clone(), serde_json::from_str(l).unwrap()))
        })
        .collect()
}

#[test]
fn matches_misaki() {
    let g2p = ZhG2p::new();
    let cases = cases();
    let mut failures = Vec::new();
    for (file, case) in &cases {
        let got = g2p.phonemize(&case.text);
        let ok = match (&got, &case.phonemes, &case.error) {
            (Ok(ps), Some(expected), _) => ps == expected,
            (Err(_), None, Some(_)) => true,
            _ => false,
        };
        if !ok {
            failures.push(format!("{file}: {:?}\n  expected {:?} {:?}\n  got      {:?}", case.text, case.phonemes, case.error, got));
        }
    }
    eprintln!("{} of {} cases match", cases.len() - failures.len(), cases.len());
    for f in failures.iter().take(20) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}
