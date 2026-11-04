//! Every pass must stay linear and fast on adversarial input: each is timed on text built by
//! repeating a short unit to n and 4n bytes. Python's regexes are quadratic on some of these;
//! the port must not be.
//!
//! The default tests cover hand-picked shapes. `cargo test --release -p kokoro-text --test scaling
//! -- --ignored` also sweeps every unit of one to three characters from an alphabet of the
//! characters the patterns react to.

use kokoro_text::{Limits, Options, check_pause_budget, check_speakable, join_lines, normalize, sentences};
use kokoro_text::{smart_split, strip_emoji};
use std::time::{Duration, Instant};

type Stage = (String, Box<dyn Fn(&str)>);

fn stages() -> Vec<Stage> {
    let all = Options { remove_emoji: true, ..Options::default() };
    let mut stages: Vec<Stage> = normalize::english_passes(&all)
        .into_iter()
        .map(|(name, pass)| (name.to_owned(), Box::new(move |t: &str| drop(pass(t))) as Box<dyn Fn(&str)>))
        .collect();
    let opts = all.clone();
    stages.push((
        "smart_split".into(),
        Box::new(move |t| smart_split(t, "a", &opts, &Limits::default(), 450, |x: &str| x.to_owned()).for_each(drop)),
    ));
    stages.push(("join_lines".into(), Box::new(|t| drop(join_lines(t)))));
    stages.push(("sentences".into(), Box::new(|t| drop(sentences(t)))));
    let opts = all.clone();
    stages.push((
        "checks".into(),
        Box::new(move |t| {
            let _ = check_speakable(t, &opts);
            let _ = check_pause_budget(t, &Limits::default());
        }),
    ));
    stages.push(("strip_emoji".into(), Box::new(|t| drop(strip_emoji(t)))));
    stages
}

fn build(prefix: &str, unit: &str, suffix: &str, n: usize) -> String {
    let mut text = String::from(prefix);
    while text.len() < n {
        text.push_str(unit);
    }
    text + suffix
}

fn timed(f: impl Fn()) -> Duration {
    let start = Instant::now();
    f();
    start.elapsed()
}

/// Times every stage at `n` and `4n` bytes; returns the shapes that grew superlinearly or blew
/// the budget of `per_100kb` per 100 KB.
fn check(shapes: &[(String, String, String)], stages: &[Stage], n: usize, per_100kb: Duration) -> Vec<String> {
    let mut slow = Vec::new();
    let mut worst: Vec<(Duration, String)> = vec![(Duration::ZERO, String::new()); stages.len()];
    for (prefix, unit, suffix) in shapes {
        let small = build(prefix, unit, suffix, n);
        let big = build(prefix, unit, suffix, 4 * n);
        for (k, (name, stage)) in stages.iter().enumerate() {
            // The whole pipeline runs every pass, so it gets their combined budget.
            let budget = per_100kb.mul_f64(4.0 * n as f64 / 100_000.0) * if name == "smart_split" { 4 } else { 1 };
            let a = timed(|| stage(&small));
            let b = timed(|| stage(&big));
            if b > worst[k].0 {
                worst[k] = (b, format!("{prefix:?} + {unit:?}* + {suffix:?}"));
            }
            let superlinear = b > Duration::from_millis(2) && b.as_secs_f64() > 8.0 * a.as_secs_f64();
            if superlinear || b > budget {
                // Retime once, so a scheduling hiccup isn't reported.
                let a = timed(|| stage(&small)).min(a);
                let b = timed(|| stage(&big)).min(b);
                if (b > Duration::from_millis(2) && b.as_secs_f64() > 8.0 * a.as_secs_f64()) || b > budget {
                    slow.push(format!(
                        "{name} on {prefix:?} + {unit:?}* + {suffix:?}: {a:?} at {n} B, {b:?} at {} B",
                        4 * n
                    ));
                }
            }
        }
    }
    for ((name, _), (time, shape)) in stages.iter().zip(worst) {
        println!("slowest {name:<16} {time:>10.1?} at {} B: {shape}", 4 * n);
    }
    slow
}

const UNITS: &[&str] = &[
    "a",
    "A",
    "1",
    "1.",
    "1,",
    "a.",
    "A.",
    "a@",
    "@a",
    "a.com",
    ".com",
    "a-",
    "-1",
    "1-",
    "$1",
    "$",
    "1 ",
    "1k",
    " k",
    "1 thousand",
    " thousand",
    "1:",
    "1:1",
    "12:30",
    "12:3",
    "1 1",
    "(555) ",
    "555-",
    "555 ",
    "+1 ",
    "AB1",
    "AB ",
    "A1",
    "A1 ",
    "A_ ",
    "-A1",
    "AB1C ",
    "A_",
    "A.B.",
    "a.b. ",
    "Dr.",
    "DR. ",
    "etc.",
    "yeah",
    "yea",
    "how're",
    "howre",
    "1S",
    "1.1",
    "1.1.",
    "...",
    "…",
    "…a",
    "  ",
    "\n",
    "\n\n",
    "\r\n",
    "[a](/",
    "[",
    "](/",
    "/)",
    "[a](/b/)",
    "[pause:1s]",
    "[pause:1",
    "[voice:a",
    "[voice:a ",
    "[voice: ",
    "[rate:1",
    "😀",
    "1️⃣",
    "x1",
    "1x",
    "1_",
    "_1",
    ".1",
    ",1",
    "1.a",
    "km",
    "1km",
    "1 km",
    "1,000",
    "1,000,",
    "1,00",
    "http://",
    "www.",
    "a.co",
    "a.b",
    "1.2.3.4",
    "192.168.0.1:",
    "a.com:1",
    "a.com/",
    "a.com/?",
    "user@a.com",
    "a.a@b",
    "a@b.",
    "a@b.cc",
    "a@b.cc1",
    "-",
    "--",
    " - ",
    "Ω",
    "°C",
    "1°",
    "1 °C",
    "١",
    "²",
    "é",
    "\u{a0}",
    "\x1c",
    "a.a",
    "a-a.com",
    "a.com.",
    "1.com",
    "1.1.1.1.",
    "1 hundred",
    "$1 hundred",
    "$1k",
    "1kk",
    "-$1",
    ".5",
    "0",
    "a1",
    "1a",
    "_",
    "A A",
    "AAAA ",
    "AAAA1",
    "I",
    "IV ",
    "Mr.",
    "MR. A",
    "e.g. ",
    "A. b",
    "12:30 pm",
    "12:30:45",
    "1:30a",
    "1 :",
    "1-1",
    "1,1",
    "1,111",
    // The number reader and the symbol pass (docs/divergences.md, "Numbers" and "Symbols").
    "1st",
    "(x-1)²",
    ")²",
    "(",
    "1,11",
    "1,11,",
    "1st ",
    "1990s ",
    "'80s ",
    "1/2",
    "1/2 ",
    "1/",
    "1½",
    "1 1/2 ",
    "10²",
    "x²",
    "H₂O",
    "1-1-",
    "2020-05-05 ",
    "1:1:",
    "~1",
    "±1",
    "#1",
    "1x1",
    "1x ",
    "1 * 1 ",
    "1 km/h ",
    "1mph",
    "B2B",
    "$0.01 ",
    "£1",
    "1¢",
    "−1",
    "1'1\"",
    "a·b",
    "∑",
    "→",
    "✓ ",
    "★",
    "👨\u{200d}👩\u{200d}👧",
    "\u{200d}",
    "🇺🇸",
    "🇺",
    "\u{fe0f}",
    "👍🏽",
    "①",
    "C# ",
    "#a",
    "a < b ",
    "<",
    "->",
    "(1",
    "1.1.1 ",
    "1e5",
    "1/2",
    "1 km/h",
    "1 KB",
    "1Ω",
    "1 s",
    "–",
    "、",
    "’",
    "“",
    "(s)",
    "'re",
];

fn shapes() -> Vec<(String, String, String)> {
    let mut shapes: Vec<_> = UNITS.iter().map(|u| (String::new(), u.to_string(), String::new())).collect();
    let wrapped: &[(&str, &str, &str)] = &[
        ("a@b.", "c", "1"),
        ("a@b.", "c", "_"),
        ("x", "1", ""),
        ("", "1", "x"),
        ("1", " ", "km"),
        ("1", " ", "kmx"),
        ("$1", "k", "x"),
        ("1", " thousand", "x"),
        ("$1", " thousand", "x"),
        ("a.com/", "x", ""),
        ("", "a", ".com"),
        ("(", "1", ""),
        ("[a](/", "b", ""),
        ("[", "a", "](/b"),
        ("[voice:a", " ", ""),
        ("[voice:", "a", ""),
        ("[rate:", "1", ""),
        ("[pause:", "1", ""),
        ("", "1", "s]"),
        ("", "a", "@b.com"),
        ("", "a.", "@b.com"),
        ("a@", "b.", "com"),
        ("a@", "b", ".com"),
        ("a@", "b.", "c1"),
        ("", "1.", "1x"),
        ("", "1,", "1x"),
        ("", "1", ".1.1x"),
        ("", "A", "1"),
        ("", "A ", "1"),
        ("", "A1", "a"),
        ("", "AB", "a"),
        ("", "1", ":30"),
        ("1", " ", ":30"),
        ("", "-", "1"),
        ("http://", "a", ".com"),
        ("http://", "a.", "com"),
        ("http://a.com", "/", ""),
        ("", "a-", ".com"),
        ("", "a.", "comx"),
        ("$", "1", "x"),
        ("$", "1", ".5x"),
        ("$1.", "1", "x"),
        ("[a](/", "b/)", ""),
        ("x\u{e000}", "1", "\u{e001}"),
        ("", "1", " km"),
        ("1.", "0", " km"),
        ("1,", "000", " km"),
        ("+", "1", " km"),
        ("1", "\u{a0}", "km"),
        ("x", "A", " A"),
        ("a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a@b.", "c", "1"),
        ("", "a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a@b.cc1", ""),
        ("", "1                                                  kmx ", ""),
    ];
    shapes.extend(wrapped.iter().map(|(p, u, s)| (p.to_string(), u.to_string(), s.to_string())));
    shapes
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn passes_scale_linearly() {
    // Matches longer than about 10 KB lose the regex crate's bounded backtracker for captures, a
    // constant factor slower; at these sizes both runs of a shape are on the same side of that step.
    let slow = check(&shapes(), &stages(), 25_000, Duration::from_millis(100));
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}

#[test]
#[ignore = "sweeps thousands of shapes; run with --release"]
fn every_short_unit_scales_linearly() {
    let alphabet = ["a", "A", "k", "m", "1", " ", ".", ",", ":", "-", "@", "$", "_", "/", "(", "[", "é", "\n"];
    let mut units: Vec<String> = alphabet.iter().map(|a| a.to_string()).collect();
    for _ in 0..2 {
        let longer: Vec<String> = units
            .iter()
            .filter(|u| u.chars().count() == units.last().unwrap().chars().count())
            .flat_map(|u| alphabet.iter().map(move |a| format!("{u}{a}")))
            .collect();
        units.extend(longer);
    }
    let shapes: Vec<_> = units.into_iter().map(|u| (String::new(), u, String::new())).collect();
    let slow = check(&shapes, &stages(), 4_000, Duration::from_millis(100));
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}
