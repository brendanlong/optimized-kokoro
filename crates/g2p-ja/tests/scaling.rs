//! Adversarial inputs to the Japanese G2P: making the text 4× longer should make it about 4×
//! slower, not 16×. MeCab scans a whole run of one character category at every position (and
//! cutlet joins every following word to look it up in `ja_words.txt`), so both are quadratic or
//! worse there in the reference. Run in release for timings:
//! `cargo test --release -p kokoro-g2p-ja --test scaling -- --nocapture`.

use kokoro_g2p_ja::{JaG2p, default_dictionary};
use std::time::{Duration, Instant};

fn time(g2p: &JaG2p, text: &str) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            g2p.phonemize(text).unwrap();
            start.elapsed()
        })
        .min()
        .unwrap()
}

fn check(g2p: &JaG2p, name: &str, make: &dyn Fn(usize) -> String) -> Option<String> {
    let mut n = 16;
    let mut small = time(g2p, &make(n));
    while small < Duration::from_millis(5) && n < 1 << 20 {
        n *= 2;
        small = time(g2p, &make(n));
    }
    let big = time(g2p, &make(4 * n));
    let ratio = big.as_secs_f64() / small.as_secs_f64();
    let chars = make(n).chars().count();
    println!("{name:28} {chars:>8} chars {small:>9.2?}  ×4 {big:>9.2?}  ratio {ratio:.1}");
    (ratio > 8.0).then(|| format!("{name}: {small:?} for {chars} chars but {big:?} for 4× that"))
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn grows_linearly() {
    let g2p = match default_dictionary() {
        Ok(d) => JaG2p::new(d),
        Err(e) => return kokoro_g2p_ja::skip_without_dictionary(e),
    };
    type Shape = (&'static str, Box<dyn Fn(usize) -> String>);
    let shapes: Vec<Shape> = vec![
        ("sentences", Box::new(|n| "東京へ行きました。".repeat(n))),
        ("one long sentence", Box::new(|n| "私は東京で新しい本を読み、".repeat(n))),
        ("katakana run", Box::new(|n| "ア".repeat(n))),
        ("unknown katakana", Box::new(|n| "ヸヹ".repeat(n))),
        ("hiragana run", Box::new(|n| "ぁ".repeat(n))),
        ("long vowel marks", Box::new(|n| "ー".repeat(n))),
        ("latin letters", Box::new(|n| "x".repeat(n))),
        ("digits", Box::new(|n| "7".repeat(n))),
        ("grouped numbers", Box::new(|n| "1,234,".repeat(n))),
        ("non-BMP kanji", Box::new(|n| "𠀋".repeat(n))),
        ("rare kanji", Box::new(|n| "鬱齟齬".repeat(n))),
        ("symbols", Box::new(|n| "☆".repeat(n))),
        ("spaces", Box::new(|n| format!("あ{}い", " ".repeat(n)))),
        ("ideographic spaces", Box::new(|n| "あ\u{3000}".repeat(n))),
        ("small kana pairs", Box::new(|n| "きゃっ".repeat(n))),
        ("iteration marks", Box::new(|n| "々ゝゞ".repeat(n))),
        ("punctuation", Box::new(|n| "「」（）、。".repeat(n))),
        ("mixed", Box::new(|n| "Tokyo東京12ア x　ー😀".repeat(n))),
    ];
    let failures: Vec<String> = shapes.iter().filter_map(|(name, make)| check(&g2p, name, make)).collect();
    assert!(failures.is_empty(), "{failures:#?}");
}
