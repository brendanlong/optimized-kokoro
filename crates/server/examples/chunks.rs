//! The chunks the server renders for each line of stdin, for listening tests: American English,
//! cut at clauses above LIMIT phonemes and packed to SOFT (`pack`), as `stream::produce_units`
//! does. With SOFT 0, the chunks as Kokoro-FastAPI makes them (no packing, clause groups up to
//! LIMIT).
//!
//! `cargo run --release -p kokoro-server --example chunks -- LIMIT SOFT < texts.txt`, where a line
//! is a JSON string. Each output line is `{"chunks": [{"text", "ids", "split": [before, after],
//! "units": [{"text", "ids", "after"}]}]}`: `units` are the pipeline's chunks (sentences and
//! clause groups) packed into it, in order, `after` whether the sentence goes on after it.

use kokoro_server::kokoro::KokoroPipelines;
use kokoro_server::pack::pack;
use kokoro_server::pipeline::{Chunk, Lang, Pipeline};
use std::io::{BufRead, Write};

fn main() {
    let arg = |i: usize| std::env::args().nth(i).and_then(|a| a.parse::<usize>().ok()).expect("LIMIT SOFT");
    let (limit, soft) = (arg(1), arg(2));
    let pipelines = KokoroPipelines::load().unwrap();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let text: String = serde_json::from_str(&line.unwrap()).unwrap();
        let groups = if soft == 0 { limit } else { soft };
        let units: Vec<Chunk> = pipelines
            .chunks(Lang::AmericanEnglish, &text, limit, groups)
            .map(Result::unwrap)
            .filter(|c| c.pause.is_none() && !c.ids.is_empty())
            .collect();
        let packed: Vec<Chunk> = if soft == 0 {
            units.clone()
        } else {
            pack(units.iter().cloned().map(Ok), soft, limit).map(Result::unwrap).collect()
        };
        let mut next = units.iter();
        let chunks: Vec<_> = packed
            .iter()
            .map(|c| {
                let mut parts = Vec::new();
                let mut ids = 0;
                while ids < c.ids.len() {
                    let unit = next.next().expect("units cover the chunks");
                    ids += unit.ids.len() + usize::from(ids > 0);
                    parts.push(serde_json::json!({ "text": unit.text, "phonemes": unit.phonemes, "ids": unit.ids, "after": unit.split.after }));
                }
                assert_eq!(ids, c.ids.len());
                serde_json::json!({ "text": c.text, "ids": c.ids, "split": [c.split.before, c.split.after], "units": parts })
            })
            .collect();
        writeln!(out, "{}", serde_json::json!({ "chunks": chunks })).unwrap();
    }
}
