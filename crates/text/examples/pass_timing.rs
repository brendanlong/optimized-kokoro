//! Times one normalizer pass (or the whole `smart_split`) on a repeated unit at 100 KB, for
//! comparison with the Python reference on the same input.
//!
//! stdin: lines of `pass<TAB>["prefix", "unit", "suffix"]`, with pass names from
//! `normalize::english_passes` or `smart_split`.

use kokoro_text::{Limits, Options, normalize, smart_split};
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let options = Options { remove_emoji: true, ..Options::default() };
    let passes = normalize::english_passes(&options);
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let (stage, shape) = line.split_once('\t').unwrap();
        let [prefix, unit, suffix]: [String; 3] = serde_json::from_str(shape).unwrap();
        let mut text = prefix.clone();
        while text.len() < 100_000 {
            text.push_str(&unit);
        }
        text.push_str(&suffix);
        let start = Instant::now();
        if stage == "smart_split" {
            smart_split(&text, "a", &options, &Limits::default(), 450, |t: &str| t.to_owned()).for_each(drop);
        } else {
            let (_, pass) = passes.iter().find(|(name, _)| *name == stage).expect("a pass name");
            drop(pass(&text));
        }
        println!("{stage}\t{:.1}ms\t{shape}", start.elapsed().as_secs_f64() * 1e3);
    }
}
