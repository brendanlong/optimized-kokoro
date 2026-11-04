//! Throws random text at every voice and reports any panic, with the input that caused it:
//! `fuzz [ITERATIONS] [SEED] [DATA_DIR]`. Build without `--release` so arithmetic overflow panics.
//! Also reports the slowest call, as a check that time stays bounded.

use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const PIECES: &[&str] = &[
    " ",
    " ",
    "  ",
    "\n",
    "\n\n",
    "\t",
    ".",
    ",",
    "!",
    "?",
    ";",
    ":",
    "-",
    "--",
    "'",
    "\"",
    "(",
    ")",
    "[",
    "]",
    "[[",
    "]]",
    "\u{1}",
    "\u{1}B1",
    "\u{1}V",
    "\u{1}Y",
    "\u{1}+10S",
    "[\u{2}",
    "_^_",
    "...",
    "…",
    "'s",
    "1",
    "12",
    "123",
    "1234",
    "12345678",
    "1,234",
    "1.234",
    "3.14",
    "0",
    "007",
    "XIV",
    "Mr.",
    "U.S.A.",
    "e.g.",
    "the",
    "a",
    "I",
    "one",
    "and",
    "café",
    "naïve",
    "Łódź",
    "Nguyễn",
    "ω",
    "λόγος",
    "мир",
    "שלום",
    "سلام",
    "नमस्ते",
    "한국어",
    "日本語",
    "😀",
    "\u{200b}",
    "\u{301}",
    "ß",
    "İ",
    "ı",
    "ǅ",
    "\u{fffd}",
    "\u{e000}",
    "\u{10ffff}",
    "%",
    "$",
    "€",
    "£",
    "@",
    "#",
    "&",
    "*",
    "/",
    "\\",
];

fn random_text(rng: &mut Rng) -> String {
    let mut s = String::new();
    let n = rng.below(60) + 1;
    for _ in 0..n {
        match rng.below(10) {
            0..=5 => s.push_str(PIECES[rng.below(PIECES.len() as u64) as usize]),
            6 => {
                let c = char::from_u32(rng.below(0x3000) as u32).unwrap_or('x');
                s.push(c);
            }
            7 => {
                let c = char::from_u32(rng.below(0x110000) as u32).unwrap_or('y');
                s.push(c);
            }
            8 => {
                for _ in 0..rng.below(40) {
                    s.push((b'a' + rng.below(26) as u8) as char);
                }
            }
            _ => {
                for _ in 0..rng.below(30) {
                    s.push((b'0' + rng.below(10) as u8) as char);
                }
            }
        }
    }
    if rng.below(20) == 0 {
        s = s.repeat(rng.below(40) as usize + 1);
    }
    s.replace('\0', "")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let iterations: usize = args.get(1).map_or(10_000, |s| s.parse().unwrap());
    let seed: u64 = args.get(2).map_or(1, |s| s.parse().unwrap());
    let dir =
        args.get(3).cloned().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/espeak-ng-data").to_string());
    let data = EspeakData::load(&dir).unwrap();
    let voices: Vec<String> = data.list_voices().iter().map(|v| v.identifier.clone()).collect();
    let mut engines: Vec<Option<Espeak>> = voices.iter().map(|_| None).collect();
    let mut rng = Rng(seed.wrapping_mul(0x9e3779b97f4a7c15) | 1);
    let mut slowest = (Duration::ZERO, String::new(), String::new());
    let mut panics = 0;
    for i in 0..iterations {
        let v = rng.below(voices.len() as u64) as usize;
        let text = random_text(&mut rng);
        let engine = engines[v].get_or_insert_with(|| {
            let mut e = Espeak::new(data.clone());
            e.set_voice_by_name(&voices[v]).unwrap();
            e
        });
        let start = Instant::now();
        let r = catch_unwind(AssertUnwindSafe(|| engine.text_to_phonemes(&text, PHONEMES_IPA_TIE)));
        let took = start.elapsed();
        if took > slowest.0 {
            slowest = (took, voices[v].clone(), text.clone());
        }
        if r.is_err() {
            panics += 1;
            println!("PANIC #{i} voice {} text {:?}", voices[v], text);
            engines[v] = None;
        }
    }
    println!(
        "{iterations} calls over {} voices, {panics} panics; slowest {:?} ({} {:?} bytes={})",
        voices.len(),
        slowest.0,
        slowest.1,
        slowest.2.chars().take(60).collect::<String>(),
        slowest.2.len()
    );
}
