//! Adversarial inputs: making the text 4× longer should make `phonemize` about 4× slower, not 16×.
//! Run in release for timings: `cargo test --release -p kokoro-g2p-zh --test scaling -- --nocapture`.

use kokoro_g2p_zh::ZhG2p;
use std::time::{Duration, Instant};

type Shape = (&'static str, Box<dyn Fn(usize) -> String>);

fn time(g2p: &ZhG2p, text: &str) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            let _ = std::hint::black_box(g2p.phonemize(text));
            start.elapsed()
        })
        .min()
        .unwrap()
}

fn check(g2p: &ZhG2p, name: &str, make: impl Fn(usize) -> String) -> Option<String> {
    let mut n = 16;
    let mut small = time(g2p, &make(n));
    while small < Duration::from_millis(2) && n < 1 << 18 {
        n *= 2;
        small = time(g2p, &make(n));
    }
    let big = time(g2p, &make(4 * n));
    let ratio = big.as_secs_f64() / small.as_secs_f64();
    let bytes = make(n).len();
    println!("{name:34} {bytes:>8} bytes {small:>9.2?}  ×4 {big:>9.2?}  ratio {ratio:.1}");
    (ratio > 8.0).then(|| format!("{name}: {small:?} for {bytes} bytes but {big:?} for 4× that"))
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn grows_linearly() {
    let g2p = ZhG2p::new();
    let shapes: Vec<Shape> = vec![
        ("sentences", Box::new(|n| "我们今天去北京，明天去上海。".repeat(n))),
        ("one Han run", Box::new(|n| "银行行长在长城上看见了很多人".repeat(n))),
        ("a repeated character (HMM)", Box::new(|n| "的".repeat(n))),
        ("a repeated word", Box::new(|n| "中国".repeat(n))),
        ("a phrase prefix (mmseg)", Box::new(|n| "一丁不".repeat(n))),
        ("single characters, no words", Box::new(|n| "鿫鿬鿭".repeat(n))),
        ("rare characters (HMM)", Box::new(|n| "㐀丂丄丅丆".repeat(n))),
        ("U+9FD6-U+9FFF", Box::new(|n| "鿫中".repeat(n))),
        ("digits", Box::new(|n| "1234567890".repeat(n))),
        ("numbers", Box::new(|n| "12，".repeat(n))),
        ("years", Box::new(|n| "12345年".repeat(n))),
        ("months in a long digit run", Box::new(|n| format!("{}月", "1".repeat(n)))),
        ("years in a long digit run", Box::new(|n| format!("{}年", "1".repeat(n)))),
        ("dates", Box::new(|n| "2024年3月5日".repeat(n))),
        ("decimals", Box::new(|n| "1.2.".repeat(n))),
        ("signs", Box::new(|n| "-1".repeat(n))),
        ("fractions", Box::new(|n| "1/2/".repeat(n))),
        ("fraction without a denominator", Box::new(|n| format!("{}/", "1".repeat(n)))),
        ("percent", Box::new(|n| "12%".repeat(n))),
        ("decimal without a percent", Box::new(|n| format!("{}.5", "1".repeat(n)))),
        ("degrees", Box::new(|n| "30℃".repeat(n))),
        ("CJK punctuation", Box::new(|n| "，《》".repeat(n))),
        ("Latin and Han", Box::new(|n| "a中".repeat(n))),
        ("full-width digits", Box::new(|n| "１２３".repeat(n))),
        ("other digits", Box::new(|n| "٣٣中".repeat(n))),
        ("spaces", Box::new(|n| "中 ".repeat(n))),
    ];
    let slow: Vec<String> = shapes.iter().filter_map(|(name, make)| check(&g2p, name, make)).collect();
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}
