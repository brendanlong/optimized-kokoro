//! The wrappers take time linear in the length of a segment: 4× the clauses or punctuation marks
//! take well under 16× the time.

use kokoro_espeak::EspeakG2P;
use std::time::{Duration, Instant};

fn best_of(runs: usize, mut f: impl FnMut()) -> Duration {
    (0..runs)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn long_segments_take_linear_time() {
    let n = if cfg!(debug_assertions) { 4_000 } else { 20_000 };
    for (language, unit) in [("es", "a。"), ("es", "a. "), ("fr-fr", "a, "), ("it", "(a) ")] {
        let g2p = EspeakG2P::new(language).unwrap();
        let time = |count: usize| {
            let text = unit.repeat(count);
            best_of(2, || assert!(!g2p.phonemize(&text).unwrap().is_empty()))
        };
        let (t1, t4) = (time(n), time(4 * n));
        let ratio = t4.as_secs_f64() / t1.as_secs_f64();
        assert!(ratio < 8.0, "{language} {unit:?}: ×{n} {t1:?}, ×{} {t4:?} ({ratio:.1}×)", 4 * n);
    }
}

/// Punctuation handling alone, with a stand-in engine so that it dominates the time.
#[test]
fn many_marks_take_linear_time() {
    let n = if cfg!(debug_assertions) { 20_000 } else { 100_000 };
    let g2p = EspeakG2P::new("es").unwrap();
    for unit in ["a. ", "a, b; ", "«a» "] {
        let time = |count: usize| {
            let text = unit.repeat(count);
            best_of(2, || {
                let out = g2p.phonemize_with(&text, |chunk| Ok(format!("{chunk} "))).unwrap();
                assert!(out.len() >= text.len() / 2);
            })
        };
        let (t1, t4) = (time(n), time(4 * n));
        let ratio = t4.as_secs_f64() / t1.as_secs_f64();
        assert!(ratio < 8.0, "{unit:?}: ×{n} {t1:?}, ×{} {t4:?} ({ratio:.1}×)", 4 * n);
    }
}

/// Letters with no rules of their own, which espeak reads without their accent: C starts the
/// word again for each one, which took time quadratic in the length of the word (on the machine
/// this was measured on, 80-160 ms per KB of `ą` or `ı` against about 1 ms for prose). Words
/// can't be longer than a clause, so doubling the text doesn't show it; this bounds the time per
/// KB instead.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn accented_letters_take_time_like_prose() {
    let prose = "The quick brown fox jumps over the lazy dog, and then it runs away. ".repeat(150);
    for (language, text) in [
        ("en-us", "ı".repeat(10_000)),
        ("en-us", "ą".repeat(10_000)),
        ("en-us", "ő".repeat(10_000)),
        ("es", "ß".repeat(10_000)),
        ("fr-fr", "ı".repeat(10_000)),
        ("en-us", prose.clone()),
        ("es", prose),
    ] {
        let g2p = EspeakG2P::new(language).unwrap();
        let took = best_of(2, || assert!(!g2p.phonemize(&text).unwrap().is_empty()));
        let per_kb = took.as_secs_f64() * 1e3 / (text.len() as f64 / 1024.0);
        let start: String = text.chars().take(4).collect();
        assert!(per_kb < 40.0, "{language} {start:?}…: {per_kb:.1} ms per KB");
    }
}
