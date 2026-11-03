//! Whole requests against the reference server, including what the corpus parity test doesn't
//! check: pauses, model ids, dropped phonemes and word spans. Cases come from
//! `reference/request_cases.py` (corpus words mixed with foreign runs, phoneme links, punctuation,
//! pause tags, long clause chains, and character fuzz) and `reference/request_dump.py`:
//! `tests/requests.jsonl` (the first 300 of seed 1, then links with blank phonemes at a chunk's
//! start and end, before a pause and at a clause cut), `tests/requests_espeak.jsonl` (the first 80
//! of `request_cases.py 2 1500 efhip`: the same kinds of text through the espeak pipelines), plus
//! `reference/out/requests/*.jsonl` (or under `$KOKORO_REFERENCE_OUT`).
//!
//! When the reference fails a request, the port yields `Err` for that sentence and carries on
//! (docs/divergences.md), so only the chunks before the failure are compared, and the port must
//! report an error, except where the failure is a listed divergence the port reads instead.
//!
//! Where the reference's English fallback left espeak's language-switch flags in the phonemes, the
//! port drops them, which moves every id and word span after them: those requests are counted apart
//! and compared on text and phonemes, without the flags, unless the shorter phonemes packed the
//! clauses into different chunks. Requests with letters of scripts `foreign.py` leaves to the
//! English G2P, which the port reads with espeak or as "<script> text", aren't compared.
//!
//! In [`STATE_DEPENDENT`] requests, C espeak-ng read an English word in French text (`read`,
//! `learned`, `underground`) differently because of an earlier call in the same process, which
//! the port doesn't let happen (docs/divergences.md, "espeak-ng: state between calls"). They are
//! counted apart, and must still be chunked the same, with as many words in each chunk.

use kokoro_pipeline::foreign::{Script, script};
use kokoro_pipeline::{Lang, NormalizationOptions, Pipeline};
use kokoro_reference::without_language_flags;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Case {
    id: String,
    lang: String,
    text: String,
    chunks: Vec<Expected>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
enum Expected {
    Pause { pause: f64 },
    Error { error: String },
    Speech { text: String, phonemes: String, ids: Vec<u8>, dropped: usize, words: Vec<(String, usize, usize)> },
}

fn cases() -> Vec<Case> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::var_os("KOKORO_REFERENCE_OUT").map_or_else(|| root.join("../../reference/out"), PathBuf::from);
    let mut paths = vec![root.join("tests/requests.jsonl"), root.join("tests/requests_espeak.jsonl")];
    if let Ok(entries) = std::fs::read_dir(out.join("requests")) {
        let mut extra: Vec<_> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        extra.sort();
        paths.extend(extra);
    }
    paths
        .iter()
        .flat_map(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
                .collect::<Vec<Case>>()
        })
        .collect()
}

/// Requests of `request_cases.py 2 1500 efhip` whose reference phonemes depend on espeak state an
/// earlier call left. Checked with `reference/espeak-ng-oracle/oracle.py pairs`: C from fresh state
/// gives the port's phonemes, and with `--history`, after the phonemizer piece before it (`was
/// featured`, then `read`), the reference's.
const STATE_DEPENDENT: [&str; 8] = [
    "efhip-2-51",
    "efhip-2-145",
    "efhip-2-286",
    "efhip-2-875",
    "efhip-2-1195",
    "efhip-2-1206",
    "efhip-2-1210",
    "efhip-2-1454",
];

fn uses_new_script(text: &str) -> bool {
    use Script::*;
    text.chars().filter_map(script).any(|s| {
        !matches!(s, Latin | Han | Hiragana | Katakana | Arabic | Cyrillic | Devanagari | Greek | Hangul | Hebrew)
    })
}

#[test]
fn requests_match_the_reference() {
    let pipelines: Vec<Pipeline> = Lang::ALL.into_iter().map(|l| Pipeline::new(l).unwrap()).collect();
    let options = NormalizationOptions::default();
    let cases = cases();
    let mut failures = Vec::new();
    let (mut flagged, mut rechunked, mut new_script, mut stateful) = (0, 0, 0, 0);
    for case in &cases {
        if uses_new_script(&case.text) {
            new_script += 1;
            continue;
        }
        let pipeline = pipelines.iter().find(|p| p.lang().code() == case.lang).expect("a pipeline language");
        let mut actual = Vec::new();
        for chunk in pipeline.chunks(&case.text, &options) {
            actual.push(match chunk {
                Ok(c) => match c.pause {
                    Some(pause) => Expected::Pause { pause },
                    None => Expected::Speech {
                        text: c.text,
                        phonemes: c.phonemes,
                        ids: c.ids,
                        dropped: c.dropped,
                        words: c.words.into_iter().map(|w| (w.text, w.ids.start, w.ids.end)).collect(),
                    },
                },
                Err(e) => Expected::Error { error: e.to_string() },
            });
        }
        let mut expected: Vec<&Expected> = case.chunks.iter().collect();
        let error = match expected.last() {
            Some(Expected::Error { error }) => Some(error.clone()),
            _ => None,
        };
        if error.is_some() {
            expected.pop();
        }
        if STATE_DEPENDENT.contains(&case.id.as_str()) {
            let shape = |c: &Expected| match c {
                Expected::Speech { text, phonemes, .. } => Some((text.clone(), phonemes.split_whitespace().count())),
                _ => None,
            };
            let shapes = |chunks: &mut dyn Iterator<Item = &Expected>| chunks.map(shape).collect::<Vec<_>>();
            assert_eq!(shapes(&mut expected.iter().copied()), shapes(&mut actual.iter()), "{}", case.id);
            stateful += 1;
            continue;
        }
        let flags = expected.iter().any(|e| matches!(e, Expected::Speech { phonemes, .. } if without_language_flags(phonemes).is_some()));
        let texts = |chunks: &mut dyn Iterator<Item = &Expected>| -> Vec<String> {
            chunks.filter_map(|c| if let Expected::Speech { text, .. } = c { Some(text.clone()) } else { None }).collect()
        };
        if flags && texts(&mut expected.iter().copied()) != texts(&mut actual.iter().take(expected.len())) {
            rechunked += 1;
            continue;
        }
        flagged += usize::from(flags);
        let same = |e: &Expected, a: &Expected| match (e, a) {
            (Expected::Speech { text, phonemes, .. }, Expected::Speech { text: t, phonemes: p, .. }) if flags => {
                text == t && without_language_flags(phonemes).as_ref().unwrap_or(phonemes) == p
            }
            _ => e == a,
        };
        let prefix_matches = expected.len() <= actual.len() && expected.iter().zip(&actual).all(|(e, a)| same(e, a));
        let ok = match error {
            // misaki's `TypeError` on `İ` after punctuation: the port reads it (docs/divergences.md).
            Some(e) if e.starts_with("TypeError: ord() expected a character") => prefix_matches,
            Some(_) => prefix_matches && actual[expected.len()..].iter().any(|a| matches!(a, Expected::Error { .. })),
            None => prefix_matches && expected.len() == actual.len(),
        };
        if !ok {
            let first =
                expected.iter().zip(&actual).position(|(e, a)| !same(e, a)).unwrap_or(expected.len().min(actual.len()));
            failures.push(format!(
                "{} {:?}\n  expected: {:?}\n  actual:   {:?}",
                case.id,
                case.text.chars().take(200).collect::<String>(),
                expected.get(first),
                actual.get(first)
            ));
        }
    }
    let compared = cases.len() - new_script - rechunked - stateful;
    println!(
        "{} of {compared} requests match ({flagged} of them once espeak's language-switch flags are dropped from the reference's); \
         not compared: {rechunked} chunked differently without the flags, {new_script} with other scripts, \
         {stateful} where the reference's espeak read a word by an earlier call's state",
        compared - failures.len(),
    );
    for f in failures.iter().take(10) {
        println!("{f}");
    }
    assert!(failures.is_empty(), "{} requests differ", failures.len());
}
