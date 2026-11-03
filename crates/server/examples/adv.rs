//! Times the server's text path (plan checks + every chunk) for JSON lines on stdin:
//! {"name": ..., "text": ..., "lang": "a"|"b"}. Prints name, bytes, chars, check ms, first-chunk ms,
//! total ms, chunks, slowest chunk ms, total ids.
use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::pipeline::{Lang, Pipeline};
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let p = KokoroPipelines::load().unwrap();
    let reps: usize = std::env::var("REPS").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    for line in std::io::stdin().lock().lines() {
        let v: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let text = v["text"].as_str().unwrap();
        let name = v["name"].as_str().unwrap_or("");
        let lang = if v["lang"].as_str() == Some("b") { Lang::BritishEnglish } else { Lang::AmericanEnglish };
        let mut best: Option<(f64, f64, f64, usize, f64, usize, usize)> = None;
        for _ in 0..reps {
            let t0 = Instant::now();
            let ok = kokoro_text::check_pause_budget(text, &kokoro_text::Limits::default()).is_ok()
                && kokoro_text::check_speakable(text, &kokoro_text::Options::default()).is_ok();
            let check = t0.elapsed().as_secs_f64() * 1e3;
            let t1 = Instant::now();
            let mut first = None;
            let mut n = 0;
            let mut slowest: f64 = 0.0;
            let mut ids = 0;
            let mut errs = 0;
            let mut last = Instant::now();
            for c in p.chunks(lang, text) {
                let now = Instant::now();
                slowest = slowest.max((now - last).as_secs_f64() * 1e3);
                last = now;
                first.get_or_insert((now - t1).as_secs_f64() * 1e3);
                match c {
                    Ok(c) => ids += c.ids.len(),
                    Err(_) => errs += 1,
                }
                n += 1;
            }
            let total = t1.elapsed().as_secs_f64() * 1e3;
            let _ = ok;
            let r = (check, first.unwrap_or(0.0), total, n, slowest, ids, errs);
            if best.is_none_or(|b| r.2 + r.0 < b.2 + b.0) {
                best = Some(r);
            }
        }
        let (check, first, total, n, slowest, ids, errs) = best.unwrap();
        println!(
            "{name}\t{}\t{}\t{check:.1}\t{first:.1}\t{total:.1}\t{n}\t{slowest:.1}\t{ids}\t{errs}",
            text.len(),
            text.chars().count()
        );
    }
}
