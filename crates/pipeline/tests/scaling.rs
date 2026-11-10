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

fn chinese() -> &'static Pipeline {
    static PIPELINE: OnceLock<Pipeline> = OnceLock::new();
    PIPELINE.get_or_init(|| Pipeline::new(Lang::Chinese).unwrap())
}

fn time(pipeline: &Pipeline, text: &str) -> Duration {
    let options = NormalizationOptions::default();
    (0..3)
        .map(|_| {
            let start = Instant::now();
            pipeline.chunks(text, &options).for_each(drop);
            start.elapsed()
        })
        .min()
        .unwrap()
}

/// Times `make(n)` and `make(4n)`, growing `n` until the small input takes long enough to measure
/// and is long enough that both are past the chunk limit, where a sentence is phonemized a second
/// time to cut it at clauses.
fn check(pipeline: &Pipeline, name: &str, make: impl Fn(usize) -> String) -> Option<String> {
    let mut n = 16;
    let mut small = time(pipeline, &make(n));
    while (small < Duration::from_millis(3) || make(n).len() < 1500) && n < 1 << 16 {
        n *= 2;
        small = time(pipeline, &make(n));
    }
    let big = time(pipeline, &make(4 * n));
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
        ("numbers of every kind", Box::new(|n| "$1.50, 1st, 1990s, 3/4, 10 km, 2020-05-05, 1-2-3 ".repeat(n))),
        ("one long number", Box::new(|n| "7".repeat(n))),
        ("numbers glued to words", Box::new(|n| "a1b2".repeat(n))),
        ("symbols", Box::new(|n| "∑ → ✓ ★ ① ½ ".repeat(n))),
        ("emoji sequences", Box::new(|n| "👨\u{200d}👩\u{200d}👧🇺🇸👍🏽".repeat(n))),
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
        ("Han run", Box::new(|n| "東京".repeat(n))),
        ("Han sentences", Box::new(|n| "東. ".repeat(n))),
        ("Han words", Box::new(|n| "東 ".repeat(n))),
        ("unreadable Han (one placeholder)", Box::new(|n| "兙".repeat(n))),
        ("Han and unreadable Han", Box::new(|n| "東兙".repeat(n))),
        ("kana words (one placeholder)", Box::new(|n| "カナ ".repeat(n))),
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
    let slow: Vec<String> = shapes.iter().filter_map(|(name, make)| check(pipeline(), name, make)).collect();
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}

/// Letters espeak reads without their accent (see `accented_letters_take_time_like_prose` in
/// kokoro-espeak): words can't be longer than a clause, so doubling the text doesn't show the
/// time quadratic in the length of a word that these took (280 ms per KB of `ı` where this was
/// measured, against 3 ms for prose); this bounds the time per KB instead.
#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn accented_letters_take_time_like_prose() {
    let prose = "The quick brown fox jumps over the lazy dog, and then it runs away. ".repeat(150);
    for (lang, text) in [
        (Lang::AmericanEnglish, "ı".repeat(10_000)),
        (Lang::AmericanEnglish, "ą".repeat(10_000)),
        (Lang::AmericanEnglish, "ő".repeat(10_000)),
        (Lang::Spanish, "ß".repeat(10_000)),
        (Lang::AmericanEnglish, prose.clone()),
        (Lang::Spanish, prose),
    ] {
        let took = time(&Pipeline::new(lang).unwrap(), &text);
        let per_kb = took.as_secs_f64() * 1e3 / (text.len() as f64 / 1024.0);
        let start: String = text.chars().take(4).collect();
        assert!(per_kb < 60.0, "{} {start:?}…: {per_kb:.1} ms per KB", lang.code());
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn chinese_grows_linearly() {
    chinese();
    let shapes: Vec<Shape> = vec![
        ("sentences", Box::new(|n| "我们今天去北京。".repeat(n))),
        ("one sentence, no punctuation", Box::new(|n| format!("{}。", "银行行长".repeat(n)))),
        ("one sentence of clauses", Box::new(|n| format!("{}。", "银行，".repeat(n)))),
        ("a repeated character", Box::new(|n| "的".repeat(n))),
        ("numbers", Box::new(|n| "12，".repeat(n))),
        ("a long number", Box::new(|n| "1".repeat(n))),
        ("dates", Box::new(|n| "2024年3月5日。".repeat(n))),
        ("Latin words", Box::new(|n| "hello 你好 ".repeat(n))),
        ("Latin and Han alternating", Box::new(|n| "a中".repeat(n))),
        ("kana (placeholder)", Box::new(|n| "カナ中".repeat(n))),
        ("Russian and Han", Box::new(|n| "Привет中".repeat(n))),
        ("Thai and Han", Box::new(|n| "สวัสดี中".repeat(n))),
        ("CJK punctuation", Box::new(|n| "中《》（）".repeat(n))),
        ("links", Box::new(|n| "[中](/a/)".repeat(n))),
    ];
    let slow: Vec<String> = shapes.iter().filter_map(|(name, make)| check(chinese(), name, make)).collect();
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}

#[test]
#[cfg_attr(debug_assertions, ignore = "timing test; run with --release")]
fn espeak_languages_grow_linearly() {
    let mut slow = Vec::new();
    for (lang, word, sentence) in [
        (Lang::Spanish, "palabra", "El gato está en la casa. "),
        (Lang::French, "mot", "Le chat est dans la maison. "),
        (Lang::Hindi, "शब्द", "बिल्ली घर में है। "),
        (Lang::Italian, "parola", "Il gatto è nella casa. "),
        (Lang::BrazilianPortuguese, "palavra", "O gato está na casa. "),
    ] {
        let pipeline = Pipeline::new(lang).unwrap();
        let shapes: Vec<Shape> = vec![
            ("sentences", Box::new(move |n| sentence.repeat(n))),
            ("one sentence, no punctuation", Box::new(move |n| format!("{}fin.", format!("{word} ").repeat(n)))),
            ("one sentence of clauses", Box::new(move |n| format!("{}fin.", format!("{word}, ").repeat(n)))),
            ("one long word", Box::new(|n| "a".repeat(n))),
            ("one long native word", Box::new(move |n| word.repeat(n))),
            ("a long number", Box::new(|n| "1".repeat(n))),
            ("numbers", Box::new(|n| "1.234,5 y 6 ".repeat(n))),
            ("dotted letters", Box::new(|n| "a.b.".repeat(n))),
            ("parentheses", Box::new(move |n| format!("({word}) ").repeat(n))),
            ("angle quotes", Box::new(move |n| format!("«{word}» ").repeat(n))),
            ("marks", Box::new(|n| format!("a{}", "\u{301}".repeat(n)))),
            ("hyphens", Box::new(move |n| format!("{word}-").repeat(n))),
            ("CJK full stops", Box::new(move |n| format!("{word}。").repeat(n))),
            ("CJK commas", Box::new(move |n| format!("{word}，").repeat(n))),
            ("English words", Box::new(|n| "hello world ".repeat(n))),
            ("Latin and Devanagari alternating", Box::new(|n| "aश".repeat(n))),
            ("Russian run", Box::new(|n| "Привет ".repeat(n))),
            ("native and Russian alternating", Box::new(move |n| format!("{word}Б").repeat(n))),
            ("Greek words", Box::new(|n| "λόγος ".repeat(n))),
            ("short Greek", Box::new(|n| "αβ ".repeat(n))),
            ("Hebrew with points", Box::new(|n| "שָׁלוֹם ".repeat(n))),
            ("Han run", Box::new(|n| "東京".repeat(n))),
            ("Han and native alternating", Box::new(move |n| format!("{word}中").repeat(n))),
            ("unreadable Han (one placeholder)", Box::new(|n| "兙".repeat(n))),
            ("kana words", Box::new(|n| "カナ ".repeat(n))),
            ("Thai words (one placeholder)", Box::new(|n| "สวัสดี ".repeat(n))),
            ("native and Thai alternating", Box::new(move |n| format!("{word}ส").repeat(n))),
            ("zero-width spaces", Box::new(move |n| format!("{word}\u{200b}").repeat(n))),
            ("links", Box::new(|n| "[a](/b/) ".repeat(n))),
            ("unclosed links", Box::new(|n| "[a](".repeat(n))),
            ("blank lines", Box::new(move |n| format!("{word}\n\n").repeat(n))),
        ];
        for (name, make) in &shapes {
            slow.extend(check(&pipeline, &format!("{} {name}", lang.code()), make));
        }
    }
    assert!(slow.is_empty(), "{}", slow.join("\n"));
}
