//! Cross-request state check: probe outputs must not change after arbitrary other inputs.
use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::pipeline::{Lang, Pipeline};
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
const PROBES: &[&str] = &[
    "Hello world.",
    "The zxqv flurble went to Москва.",
    "Ναι, I said 1,234 times at 12:30 pm.",
    "[Kokoro](/kˈOkəɹO/) reads qwertyuiop and NASA.",
    "He read the book; she will read it.",
    "Ünïcödé çafé naïve résumé.",
    "Schadenfreude, über, déjà vu, and tsunami.",
    "I live in Lyon, near Paris; je ne sais quoi.",
];

fn run(p: &KokoroPipelines, lang: Lang, text: &str) -> String {
    p.chunks(lang, text, kokoro_server::pipeline::DEFAULT_MAX_TOKENS, kokoro_server::pipeline::DEFAULT_MAX_TOKENS)
        .map(|c| match c {
            Ok(c) => format!("{}|{:?}", c.phonemes, c.pause),
            Err(e) => format!("ERR {e}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() {
    let iters: usize = std::env::args().nth(1).map_or(2000, |s| s.parse().unwrap());
    let p = KokoroPipelines::load().unwrap();
    let langs = [Lang::AmericanEnglish, Lang::BritishEnglish];
    let base: Vec<Vec<String>> = langs.iter().map(|&l| PROBES.iter().map(|t| run(&p, l, t)).collect()).collect();
    let mut x: u64 = 0x1234_5678_9abc_def1;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut diffs = 0;
    for i in 0..iters {
        let n = 1 + (next() % 60) as usize;
        let text: String = (0..n).map(|_| ATOMS[(next() % ATOMS.len() as u64) as usize]).collect();
        let lang = langs[(next() % 2) as usize];
        let _ = run(&p, lang, &text);
        for (li, &l) in langs.iter().enumerate() {
            for (pi, probe) in PROBES.iter().enumerate() {
                let now = run(&p, l, probe);
                if now != base[li][pi] {
                    diffs += 1;
                    println!(
                        "iter {i}: after {text:?} ({lang:?}), probe {probe:?} in {l:?} changed:\n  was {:?}\n  now {now:?}",
                        base[li][pi]
                    );
                }
            }
        }
        if diffs > 5 {
            break;
        }
    }
    println!("done, {diffs} differences");
}
