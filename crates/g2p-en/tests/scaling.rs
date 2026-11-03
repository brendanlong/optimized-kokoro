//! Adversarial inputs: making the input 4× longer should make G2P about 4× slower, not 16×.
//! Run in release (`cargo test --release -p kokoro-g2p-en --test scaling -- --nocapture`) for timings.

use kokoro_g2p_en::{Fallback, G2p, TaggedToken, Tagger};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Splits on spaces, like spaCy does for plain words; digits are `CD`, `!`/`.` runs `.`, the rest `NN`.
struct Spaces;

impl Tagger for Spaces {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken> {
        let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
        let n = words.len();
        words
            .into_iter()
            .enumerate()
            .map(|(i, w)| TaggedToken {
                text: w.to_string(),
                tag: if w.starts_with(|c: char| c.is_ascii_digit() || c == '$') {
                    "CD"
                } else if w.chars().all(|c| c == '!' || c == '.') {
                    "."
                } else {
                    "NN"
                }
                .to_string(),
                whitespace: if i + 1 < n { " ".to_string() } else { String::new() },
            })
            .collect()
    }
}

struct Echo;

impl Fallback for Echo {
    fn phonemize(&mut self, word: &str) -> Option<String> {
        Some(word.chars().take(8).collect())
    }
}

fn g2p() -> &'static G2p {
    static G2P: OnceLock<G2p> = OnceLock::new();
    G2P.get_or_init(|| G2p::new(false))
}

fn time(text: &str, fallback: bool) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            let mut echo = Echo;
            let _ = g2p().phonemize(text, &mut Spaces, if fallback { Some(&mut echo) } else { None });
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// Times `make(n)` and `make(4n)`, growing `n` until the small input takes long enough to measure.
fn check(name: &str, fallback: bool, make: impl Fn(usize) -> String) {
    let mut n = 64;
    let mut small = time(&make(n), fallback);
    while small < Duration::from_millis(2) && n < 1 << 16 {
        n *= 2;
        small = time(&make(n), fallback);
    }
    let big = time(&make(4 * n), fallback);
    let ratio = big.as_secs_f64() / small.as_secs_f64();
    println!("{name:32} {:>7} bytes {:>9.2?}  ×4 {:>9.2?}  ratio {ratio:.1}", make(n).len(), small, big);
    assert!(ratio < 9.0, "{name}: {small:?} for {} bytes but {big:?} for 4× that", make(n).len());
}

#[test]
fn grows_linearly() {
    g2p();
    check("hyphenated run, fallback", true, |n| format!("{}qx", "qx-".repeat(n)));
    check("hyphenated run, no fallback", false, |n| format!("{}qx", "qx-".repeat(n)));
    check("one long phoneme link", true, |n| format!("[{}](/a/)", "q ".repeat(n)));
    check("a link per word", true, |n| "[a](1) ".repeat(n));
    check("multi-word links", true, |n| "[a b](/x/) ".repeat(n));
    check("number-flag link", true, |n| format!("[{}](#{}#)", "1 ".repeat(n), "an&".repeat(n)));
    check("Greek link", true, |n| format!("[{}](/x/)Α", "ΟΣ ".repeat(n)));
    check("long lowercase word", true, |n| "a".repeat(n));
    check("long capitals", true, |n| "A".repeat(n));
    check("camel case", true, |n| "aB".repeat(n));
    check("dotted letters", true, |n| "a.".repeat(n));
    check("separated digits", true, |n| "1,,".repeat(n));
    check("digits", true, |n| "1".repeat(n.min(300)) + &".5".repeat(n / 2));
    check("apostrophes", true, |n| "a'".repeat(n));
    check("quotes", true, |n| "'".repeat(n));
    check("hyphens", true, |n| format!("a{}b", "-".repeat(n)));
    check("x2y aliases", true, |n| "x2".repeat(n));
    check("punctuation", true, |n| "!".repeat(n));
    check("plain words", true, |n| "The cat sat on the mat. ".repeat(n));
    check("numbers and money", true, |n| "$1 2nd 1990s 3.5 ".repeat(n));
    check("unknown words", true, |n| "qzx ".repeat(n));
}

/// Without a fallback, every subtoken of a run is looked up in up to `MAX_SPAN` spans. Spans that
/// can't have a reading (non-letter, non-ASCII characters such as combining marks) must cost no
/// more than ordinary ones: they once took 4× as long, 3.7 s for 128 K marks.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn unreadable_runs_cost_no_more_than_readable_ones() {
    g2p();
    check("combining marks, no fallback", false, |n| format!("a{}", "\u{301}".repeat(n)));
    check("emoji run, no fallback", false, |n| "😀".repeat(n));
    let n = 8000;
    let marks = time(&format!("a{}", "\u{301}".repeat(n)), false);
    let hyphens = time(&format!("{}qx", "qx-".repeat(n / 2)), false);
    println!("{n} subtokens: marks {marks:?}, hyphenated run {hyphens:?}");
    assert!(marks < hyphens * 3, "{marks:?} for {n} marks but {hyphens:?} for {n} hyphenated subtokens");
}
