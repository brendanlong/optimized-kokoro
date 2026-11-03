//! Orders UniDic's context ids by how often analysis looks up their connection costs, so the
//! matrix's hot cells share pages:
//!
//!     cargo run --release -p kokoro-g2p-ja --example context_order -- count DICT TEXTS.jsonl... > data/context_order.txt
//!     cargo run --release -p kokoro-g2p-ja --example context_order -- measure DICT TEXTS.jsonl...
//!
//! `count` needs a dictionary built with UniDic's own ids (`build_dict DICDIR OUT none`). `measure`
//! reports how many 4 KiB pages of the matrix the texts read, in all and per text.
use kokoro_g2p_ja::Dictionary;
use kokoro_g2p_ja::cutlet::{Part, normalize};
use std::collections::HashSet;

fn texts(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .flat_map(|p| std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{p}: {e}")).lines().map(str::to_owned).collect::<Vec<_>>())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l).ok()?["text"].as_str().map(str::to_owned))
        .collect()
}

fn each_lookup(dict: &Dictionary, text: &str, f: &mut dyn FnMut(u16, u16)) {
    for part in normalize(text) {
        if let Part::Text(t) = part {
            kokoro_g2p_ja::mecab::analyze_observed(dict, &t, Some(&mut *f)).unwrap();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dict = Dictionary::load(&args[2]).unwrap();
    let texts = texts(&args[3..]);
    match args[1].as_str() {
        "count" => {
            let (mut right, mut left) = (vec![0u64; 1 << 16], vec![0u64; 1 << 16]);
            for t in &texts {
                each_lookup(&dict, t, &mut |r, l| {
                    right[r as usize] += 1;
                    left[l as usize] += 1;
                });
            }
            println!("# Context ids by how often analysis of {} texts looked them up (examples/context_order.rs).", texts.len());
            for (name, counts) in [("right", right), ("left", left)] {
                let mut ids: Vec<u16> = (1..=u16::MAX).filter(|&i| counts[i as usize] > 0).collect();
                ids.sort_by_key(|&i| (std::cmp::Reverse(counts[i as usize]), i));
                let ids: Vec<String> = ids.iter().map(u16::to_string).collect();
                println!("{name} {}", ids.join(" "));
            }
        }
        "measure" => {
            let mut all = HashSet::new();
            let mut per: Vec<usize> = texts
                .iter()
                .map(|t| {
                    let mut pages = HashSet::new();
                    each_lookup(&dict, t, &mut |r, l| {
                        pages.insert(dict.connection_offset(r, l) / 4096);
                    });
                    all.extend(pages.iter().copied());
                    pages.len()
                })
                .collect();
            per.sort_unstable();
            let mb = |pages: usize| pages as f64 * 4096.0 / 1e6;
            println!(
                "{} texts: {:.1} MB of matrix pages read in all; per text median {:.2} MB, p99 {:.2} MB, max {:.2} MB",
                texts.len(),
                mb(all.len()),
                mb(per[per.len() / 2]),
                mb(per[per.len() * 99 / 100]),
                mb(per[per.len() - 1])
            );
        }
        _ => panic!("count or measure"),
    }
}
