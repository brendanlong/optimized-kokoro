//! Random texts through the server's text path, looking for panics. Args: seed iterations.
use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::pipeline::{Lang, Pipeline};
use std::sync::Arc;

const ATOMS: &[&str] = &[
    "a",
    "A",
    "Z",
    "i",
    "I",
    "1",
    "9",
    "0",
    ".",
    ",",
    ":",
    ";",
    "-",
    "--",
    "/",
    "$",
    "£",
    "€",
    "¥",
    "%",
    "@",
    "#",
    "&",
    "'",
    "’",
    "\"",
    "“",
    "”",
    "(",
    ")",
    "[",
    "]",
    "](/",
    "/)",
    "[a](/ə/)",
    "[x](/",
    "](",
    " ",
    "  ",
    "\n",
    "\r\n",
    "\n\n",
    "\t",
    "…",
    "!",
    "?",
    "é",
    "\u{301}",
    "Б",
    "α",
    "東",
    "😀",
    "ー",
    "。",
    "，",
    "、",
    "http://",
    "https://",
    "www.",
    ".com",
    ".org",
    "km",
    "kg",
    "pm",
    "am",
    "th",
    "st",
    "nd",
    "Mr.",
    "MR.",
    "Mrs.",
    "DR.",
    "Dr.",
    "etc.",
    "U.S.",
    "NASA",
    "they're",
    "you're",
    "yeah",
    "(s)",
    " hundred",
    " billion",
    "k",
    "m",
    "b",
    "e",
    "x",
    "[pause:0.1s]",
    "[pause:2s]",
    "\u{200b}",
    "\u{a0}",
    "ﬁ",
    "ａ",
    "Ⅻ",
    "½",
    "²",
    "₹",
    "+1 ",
    "555",
    "1,234",
    "3.14",
    "1.2.3",
    "12:30",
    "-5",
    ".5",
    "1e5",
    "0x1F",
    "ß",
    "İ",
    "ı",
    "ǅ",
    "\u{0}",
    "\u{feff}",
    "\u{202e}",
    "ש",
    "ا",
    "न",
    "한",
    "ก",
    "カ",
    "ά",
    "ω",
    "Ω",
    "μ",
    "°",
    "℃",
    "№",
    "™",
    "©",
    "®",
    "§",
    "¶",
    "•",
    "·",
    "—",
    "–",
    "‒",
    "~",
    "^",
    "*",
    "_",
    "=",
    "+",
    "<",
    ">",
    "|",
    "\\",
    "`",
    "{",
    "}",
    "1st",
    "2nd",
    "21st",
    "1990s",
    "'90s",
    "$1.5",
    "5%",
    "1/2",
    "3/4",
    "x2",
    "2x",
    "a.m.",
    "p.m.",
    "e.g.",
    "i.e.",
    "vs.",
    "St.",
    "Jr.",
    "Ph.D.",
    "O'Neil",
    "rock'n'roll",
    "don't",
    "can't",
    "I'm",
    "it's",
    "'s",
    "s'",
    "Mc",
    "IV",
    "XIV",
    "§1",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let seed: u64 = args.next().map_or(1, |s| s.parse().unwrap());
    let iters: usize = args.next().map_or(10000, |s| s.parse().unwrap());
    let p = Arc::new(KokoroPipelines::load().unwrap());
    std::panic::set_hook(Box::new(|info| {
        eprintln!("PANIC: {info}");
    }));
    let threads: Vec<_> = (0..20u64)
        .map(|t| {
            let p = p.clone();
            std::thread::spawn(move || {
                let mut x = seed.wrapping_mul(0x9E3779B97F4A7C15) ^ (t + 1).wrapping_mul(0xBF58476D1CE4E5B9);
                let mut next = move || {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x
                };
                let mut panics = 0;
                for _ in 0..iters {
                    let n = 1 + (next() % 40) as usize;
                    let text: String = (0..n).map(|_| ATOMS[(next() % ATOMS.len() as u64) as usize]).collect();
                    let lang = if next() % 2 == 0 { Lang::AmericanEnglish } else { Lang::BritishEnglish };
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _ = kokoro_text::check_pause_budget(&text, &kokoro_text::Limits::default());
                        let _ = kokoro_text::check_speakable(&text, &kokoro_text::Options::default());
                        for c in p
                            .chunks(
                                lang,
                                &text,
                                kokoro_server::pipeline::DEFAULT_MAX_TOKENS,
                                kokoro_server::pipeline::DEFAULT_MAX_TOKENS,
                            )
                            .flatten()
                        {
                            assert!(c.pause.is_some() || c.ids.iter().all(|i| (1..178).contains(i)), "bad id");
                        }
                    }));
                    if r.is_err() {
                        panics += 1;
                        eprintln!("PANIC INPUT ({lang:?}): {text:?}");
                        if panics > 5 {
                            break;
                        }
                    }
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    eprintln!("done");
}
