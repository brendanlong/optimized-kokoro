//! Script segmentation against kokoro's `ForeignScripts.split` on random mixed-script text
//! (`reference/foreign_cases.py`), with pypinyin's readings for the Han characters in it.

use kokoro_pipeline::foreign::{Language, split};
use serde::Deserialize;

#[derive(Deserialize)]
struct Case {
    text: String,
    segments: Vec<(String, Option<String>)>,
    unreadable: Vec<char>,
}

#[test]
fn matches_foreign_py() {
    let data = include_str!("foreign_cases.jsonl");
    let mut failures = Vec::new();
    let mut total = 0;
    for line in data.lines().filter(|l| !l.trim().is_empty()) {
        let case: Case = serde_json::from_str(line).unwrap();
        total += 1;
        let actual: Vec<(String, Option<String>)> = split(&case.text, |c| !case.unreadable.contains(&c))
            .into_iter()
            .map(|(s, l)| (s.to_owned(), l.map(Language::code).map(str::to_owned)))
            .collect();
        if actual != case.segments {
            failures.push(format!("{:?}\n  expected {:?}\n  actual   {:?}", case.text, case.segments, actual));
        }
    }
    for f in failures.iter().take(20) {
        println!("{f}");
    }
    assert!(failures.is_empty(), "{} of {total} cases differ", failures.len());
}
