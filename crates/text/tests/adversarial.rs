//! Floods of the characters each pattern scans for: normalization must stay roughly linear.

use kokoro_text::{Limits, Options, join_lines, sentences, smart_split, split};
use std::time::{Duration, Instant};

const SIZE: usize = 100_000;

fn floods() -> Vec<(&'static str, String)> {
    vec![
        ("letters", "a".repeat(SIZE)),
        ("caps", "A".repeat(SIZE)),
        ("caps words", "ABCD ".repeat(SIZE / 5)),
        ("digits", "1".repeat(SIZE)),
        ("glued digits", format!("x{}", "1".repeat(SIZE))),
        ("digits glued after", format!("{}x", "1".repeat(SIZE))),
        ("glued version", format!("x{}.1.1", "1".repeat(SIZE))),
        ("glued unit", format!("x{} km", "1".repeat(SIZE))),
        ("glued caps", format!("x{} A", "A".repeat(SIZE))),
        ("caps chains", "AB1C ".repeat(SIZE / 5)),
        ("alternating", "1x".repeat(SIZE / 2)),
        ("digit groups", "1,".repeat(SIZE / 2)),
        ("dotted digits", "1.".repeat(SIZE / 2)),
        ("dotted letters", "a.".repeat(SIZE / 2)),
        ("domain", format!("{}com", "a.".repeat(SIZE / 2))),
        ("email locals", "a@".repeat(SIZE / 2)),
        ("hyphens", "-".repeat(SIZE)),
        ("times", "1:".repeat(SIZE / 2)),
        ("money", "$1".repeat(SIZE / 2)),
        ("multipliers", format!("1{}", " thousand".repeat(SIZE / 9))),
        ("spaces", " ".repeat(SIZE)),
        ("newlines", "\n".repeat(SIZE)),
        ("brackets", "[".repeat(SIZE)),
        ("annotations", "[a](/".repeat(SIZE / 5)),
        ("phones", "(555) ".repeat(SIZE / 6)),
        ("units", "1 km".repeat(SIZE / 4)),
        ("emoji", "😀".repeat(SIZE / 4)),
    ]
}

#[test]
fn floods_stay_fast() {
    let options = Options { unit_normalization: true, remove_emoji: true, ..Options::default() };
    let budget = if cfg!(debug_assertions) { Duration::from_secs(30) } else { Duration::from_secs(2) };
    for (name, text) in floods() {
        let start = Instant::now();
        let normalized = split::normalize(&join_lines(&text), "a", &options);
        let _ = sentences(&normalized);
        let chunks = smart_split(&text, "a", &options, &Limits::default(), 450, |t: &str| t.to_owned()).count();
        let elapsed = start.elapsed();
        println!("{name}: {elapsed:?} ({chunks} chunks)");
        assert!(elapsed < budget, "{name} took {elapsed:?}");
    }
}
