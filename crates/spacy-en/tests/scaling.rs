//! Tokenizing adversarial strings must take time linear in their length: compares a string of
//! about 25 KB with one 4× longer, built the same way (`head`, `unit` repeated, `tail`).

use kokoro_spacy_en::Nlp;
use std::time::{Duration, Instant};

/// Prefix, suffix and infix characters, and other things the patterns look for.
const UNITS: &[&str] = &[
    "§", "%", "=", "—", "–", "+", "…", ",", ":", ";", "!", "?", "¿", "¡", "(", ")", "[", "]", "{", "}", "<", ">", "_",
    "#", "*", "&", "。", "'", "\"", "”", "“", "`", "‘", "´", "’", "‚", "„", "»", "«", "$", "£", "€", "¥", "US$", ".",
    "-", "/", "~", "^", "@", "😀", "°", "'s", "n't", "--", "...",
];

/// URL-ish strings, credentials, IP addresses, apostrophes, hyphens, dots, units.
const MIXED: &[(&str, &str, &str)] = &[
    ("", "a:", "@127.0.0.1"),
    ("", "a:", "@example.com"),
    ("", "a@", "b.com"),
    ("", "1.2.3.4@", ""),
    ("http://", "a.", "com"),
    ("www.", "a.", "com"),
    ("www.", "a-", ".com"),
    ("http://x.com/", "?", ""),
    ("http://", "a:", "@b.com/"),
    ("", "a.", "com"),
    ("", "a-", ""),
    ("", "1+", ""),
    ("", "a.b", ""),
    ("", "a,b", ""),
    ("", "a:b/", ""),
    ("", "a--b", ""),
    ("", "'a'", ""),
    ("", "U.S.", ""),
    ("", "e.g.", ""),
    ("", "1km", ""),
    ("", "1", "km"),
    ("", "1$", ""),
    ("1", "°F.", ""),
    ("", "a)", ""),
    ("", "(a", ""),
    ("(word", ")", ""),
    ("a", "....", "a"),
    ("", "a", ""),
];

fn shapes() -> Vec<(&'static str, &'static str, &'static str)> {
    let around = UNITS.iter().flat_map(|&u| [("", u, ""), ("", u, "word"), ("word", u, ""), ("(", u, "word)")]);
    around.chain(MIXED.iter().copied()).collect()
}

fn time(nlp: &Nlp, text: &str) -> Duration {
    (0..3)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(nlp.tokenize(text));
            start.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn tokenizing_is_linear() {
    let nlp = Nlp::new();
    let make = |(head, unit, tail): (&str, &str, &str), bytes: usize| {
        format!("{head}{}{tail}", unit.repeat(bytes / unit.len()))
    };
    let mut failures = Vec::new();
    let mut timings = Vec::new();
    for shape in shapes() {
        let (short, long) = (make(shape, 25_000), make(shape, 100_000));
        let (t_short, t_long) = (time(&nlp, &short), time(&nlp, &long));
        let name = format!("{:?} + {:?} * n + {:?}", shape.0, shape.1, shape.2);
        // Quadratic time would be 16×; allow for noise, and ignore anything too fast to measure.
        if t_long > Duration::from_millis(5) && t_long > t_short * 8 {
            failures.push(format!("{name}: {t_short:?} for {} bytes, {t_long:?} for {}", short.len(), long.len()));
        }
        timings.push((t_long, t_long.as_secs_f64() / t_short.as_secs_f64(), name));
    }
    timings.sort_by_key(|t| std::cmp::Reverse(t.0));
    for (t, ratio, name) in &timings[..5] {
        eprintln!("100 KB: {t:?} ({ratio:.1}× the 25 KB time) for {name}");
    }
    assert!(failures.is_empty(), "superlinear: {failures:#?}");
}
