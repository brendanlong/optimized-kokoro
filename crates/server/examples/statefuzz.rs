//! Cross-request state check: probe outputs must not change after arbitrary other inputs.
use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::pack::SOFT_MAX_PHONEMES;
use kokoro_server::pipeline::{DEFAULT_MAX_TOKENS, Lang};
use kokoro_server::stream;
#[path = "common/atoms.rs"]
mod atoms;
use atoms::ATOMS;
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
    stream::packed_chunks(p, lang, text, DEFAULT_MAX_TOKENS, SOFT_MAX_PHONEMES, &|| false)
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
