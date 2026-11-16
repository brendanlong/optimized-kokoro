//! Random texts through the server's text checks and chunking, looking for panics. Args: seed iterations.
use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::pack::SOFT_MAX_PHONEMES;
use kokoro_server::pipeline::{DEFAULT_MAX_TOKENS, Lang};
use kokoro_server::stream;
use std::sync::Arc;

#[path = "common/atoms.rs"]
mod atoms;
use atoms::ATOMS;

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
                        let _ = kokoro_text::check_pause_budget(&text);
                        let _ = kokoro_text::check_speakable(&text, &kokoro_text::Options::default());
                        for c in
                            stream::packed_chunks(&*p, lang, &text, DEFAULT_MAX_TOKENS, SOFT_MAX_PHONEMES, &|| false)
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
