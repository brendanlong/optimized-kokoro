//! Adversarial inputs through the whole pipeline: making the text 4× longer should make
//! `chunks` about 4× slower, not 16×. Run in release for timings:
//! `cargo test --release -p kokoro-pipeline --test scaling -- --nocapture`.

use kokoro_pipeline::{Lang, NormalizationOptions, Pipeline};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

type Shape = (&'static str, Box<dyn Fn(usize) -> String>);

fn pipeline() -> &'static Pipeline {
    static PIPELINE: OnceLock<Pipeline> = OnceLock::new();
    PIPELINE.get_or_init(|| Pipeline::new(Lang::AmericanEnglish).unwrap())
}

fn time(text: &str) -> Duration {
    let options = NormalizationOptions::default();
    (0..3)
        .map(|_| {
            let start = Instant::now();
            pipeline().chunks(text, &options).for_each(drop);
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// Times `make(n)` and `make(4n)`, growing `n` until the small input takes long enough to measure
/// and is long enough that both are past the chunk limit, where a sentence is phonemized a second
/// time to cut it at clauses.
fn check(name: &str, make: impl Fn(usize) -> String) -> Option<String> {
    let mut n = 16;
    let mut small = time(&make(n));
    while (small < Duration::from_millis(3) || make(n).len() < 1500) && n < 1 << 16 {
        n *= 2;
        small = time(&make(n));
    }
    let big = time(&make(4 * n));
    let ratio = big.as_secs_f64() / small.as_secs_f64();
    let bytes = make(n).len();
    println!("{name:34} {bytes:>7} bytes {small:>9.2?}  ×4 {big:>9.2?}  ratio {ratio:.1}");
    (ratio > 8.0).then(|| format!("{name}: {small:?} for {bytes} bytes but {big:?} for 4× that"))
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn grows_linearly() {
    pipeline();
    let shapes: Vec<Shape> = vec![
        ("sentences", Box::new(|n| "The cat sat on the mat. ".repeat(n))),
        ("one sentence, no punctuation", Box::new(|n| format!("{}end.", "word ".repeat(n)))),
        ("one sentence of clauses", Box::new(|n| format!("{}end.", "word, ".repeat(n)))),
        ("long clauses", Box::new(|n| format!("{}end.", "one two three four five six seven eight; ".repeat(n)))),
        ("one long word", Box::new(|n| "a".repeat(n))),
        ("unknown words", Box::new(|n| (0..n).map(|i| format!("qz{i:x}v ")).collect())),
        ("hyphenated run", Box::new(|n| format!("{}qx", "qx-".repeat(n)))),
        ("dotted letters", Box::new(|n| "a.b.".repeat(n))),
        ("numbers", Box::new(|n| "1,234 and 5.6 ".repeat(n))),
        ("Russian run", Box::new(|n| "Привет ".repeat(n))),
        ("Russian sentences", Box::new(|n| "Привет. ".repeat(n))),
        ("alternating scripts", Box::new(|n| "a Привет ".repeat(n))),
        ("alternating, no spaces", Box::new(|n| "aБ".repeat(n))),
        ("Russian joined by hyphens", Box::new(|n| "Б-".repeat(n))),
        ("short Greek", Box::new(|n| "αβ ".repeat(n))),
        ("Greek words", Box::new(|n| "λόγος ".repeat(n))),
        ("Greek pair with marks", Box::new(|n| format!("ΑΒ{}", "\u{301}".repeat(n)))),
        ("Latin with marks", Box::new(|n| format!("a{}", "\u{301}".repeat(n)))),
        ("Hebrew with points", Box::new(|n| "שָׁלוֹם ".repeat(n))),
        ("Arabic", Box::new(|n| "مرحبا ".repeat(n))),
        ("Hindi", Box::new(|n| "नमस्ते ".repeat(n))),
        ("Korean", Box::new(|n| "안녕하세요 ".repeat(n))),
        ("links and runs", Box::new(|n| "[a](/b/) Привет ".repeat(n))),
        ("unclosed links", Box::new(|n| "[Привет](".repeat(n))),
        ("Han run (placeholder)", Box::new(|n| "東京".repeat(n))),
        ("Han sentences (placeholder)", Box::new(|n| "東. ".repeat(n))),
        ("Han words (one placeholder)", Box::new(|n| "東 ".repeat(n))),
        ("Thai words (one placeholder)", Box::new(|n| "สวัสดี ".repeat(n))),
        ("Thai with zero-width spaces", Box::new(|n| "สวัสดี\u{200b}".repeat(n))),
        ("Latin and Thai alternating", Box::new(|n| "aส".repeat(n))),
        ("Georgian", Box::new(|n| "გამარჯობა ".repeat(n))),
        ("long-vowel marks", Box::new(|n| "ー".repeat(n))),
        ("CJK full stops", Box::new(|n| "a。".repeat(n))),
        ("CJK commas", Box::new(|n| format!("{}a", "word，".repeat(n)))),
        ("pause tags", Box::new(|n| "[pause:0.1s] a ".repeat(n))),
        ("blank lines", Box::new(|n| "a\n\n".repeat(n))),
    ];
    let slow: Vec<String> = shapes.iter().filter_map(|(name, make)| check(name, make)).collect();
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}
