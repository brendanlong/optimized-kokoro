//! Compares tokens and tags with spaCy's, over every recording available:
//! - `tests/data/*.jsonl` (committed; from `reference.py`),
//! - `reference/out/spacy-en/*.jsonl` or `$KOKORO_REFERENCE_OUT/spacy-en/*.jsonl` (from `reference.py`
//!   over the downloaded corpus),
//! - the `spacy` field of every English call in the reference pipeline dumps.

use kokoro_spacy_en::{Nlp, TAGS, Token};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Case {
    text: String,
    tokens: Vec<(String, String, String)>,
}

fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    files
}

fn read_cases(path: &Path) -> Vec<Case> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .collect()
}

/// misaki's `G2P.preprocess`: strips leading whitespace and replaces `[text](...)` links with `text`.
fn misaki_preprocess(text: &str) -> String {
    let text = text.trim_start();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find(']').filter(|&close| close > 0).and_then(|close| {
            let target = after[close + 1..].strip_prefix('(')?;
            let end = target.find(')')?;
            Some((&after[..close], close + 1 + 1 + end + 1))
        });
        match link {
            Some((label, len)) => {
                out.push_str(&rest[..open]);
                out.push_str(label);
                rest = &after[len..];
            }
            None => {
                out.push_str(&rest[..open + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// spaCy's inputs and outputs from the reference dumps: each English call's segments without a
/// language, as `KPipeline.en_g2p_tokens` passes them to misaki.
fn dump_cases() -> (Vec<Case>, usize) {
    let mut cases = Vec::new();
    let mut skipped = 0;
    for record in kokoro_reference::all() {
        for call in record.calls.iter().filter(|c| !c.spacy.is_empty()) {
            let inputs: Vec<_> = call
                .segments
                .iter()
                .filter(|(_, lang)| lang.is_none())
                .map(|(s, _)| s.trim_start())
                .filter(|s| !s.is_empty())
                .map(misaki_preprocess)
                .collect();
            if inputs.len() != call.spacy.len() {
                skipped += 1;
                continue;
            }
            for (text, tokens) in inputs.into_iter().zip(&call.spacy) {
                let tokens = tokens.iter().map(|t| (t.text.clone(), t.tag.clone(), t.whitespace.clone())).collect();
                cases.push(Case { text, tokens });
            }
        }
    }
    (cases, skipped)
}

#[derive(Default)]
struct Stats {
    texts: usize,
    texts_exact: usize,
    tokenization_diffs: usize,
    tokens: usize,
    tags_compared: usize,
    tag_diffs: usize,
}

fn margin(scores: &[f32], a: usize, b: usize) -> f32 {
    scores[a] - scores[b]
}

fn check(nlp: &Nlp, name: &str, cases: &[Case], stats: &mut Stats, failures: &mut Vec<String>) {
    let mut local = Stats::default();
    for case in cases {
        local.texts += 1;
        local.tokens += case.tokens.len();
        let scored = nlp.tag_scores(&case.text);
        let ours: Vec<Token> = nlp.process(&case.text);
        let same_tokens = ours.len() == case.tokens.len()
            && ours.iter().zip(&case.tokens).all(|(o, (t, _, w))| o.text == t && o.whitespace == w);
        if !same_tokens {
            local.tokenization_diffs += 1;
            failures.push(format!(
                "{name}: tokens differ for {:?}\n  spaCy: {:?}\n  ours:  {:?}",
                case.text,
                case.tokens.iter().map(|(t, _, w)| format!("{t}{w}")).collect::<Vec<_>>(),
                ours.iter().map(|t| format!("{}{}", t.text, t.whitespace)).collect::<Vec<_>>(),
            ));
            continue;
        }
        let mut exact = true;
        for (i, (o, (text, tag, _))) in ours.iter().zip(&case.tokens).enumerate() {
            local.tags_compared += 1;
            if o.tag != tag {
                local.tag_diffs += 1;
                exact = false;
                let scores = &scored[i].1;
                let expected = TAGS.iter().position(|t| t == tag).expect("known tag");
                let got = TAGS.iter().position(|t| *t == o.tag).unwrap();
                failures.push(format!(
                    "{name}: tag of {text:?} (token {i}) is {} but spaCy says {tag}; our margin {:.3e} in {:?}",
                    o.tag,
                    margin(scores, got, expected),
                    case.text
                ));
            }
        }
        local.texts_exact += usize::from(exact);
    }
    eprintln!(
        "{name}: {}/{} texts exact; {} tokenization diffs; tags {}/{} match ({} tokens)",
        local.texts_exact,
        local.texts,
        local.tokenization_diffs,
        local.tags_compared - local.tag_diffs,
        local.tags_compared,
        local.tokens,
    );
    stats.texts += local.texts;
    stats.texts_exact += local.texts_exact;
    stats.tokenization_diffs += local.tokenization_diffs;
    stats.tokens += local.tokens;
    stats.tags_compared += local.tags_compared;
    stats.tag_diffs += local.tag_diffs;
}

#[test]
fn parity_with_spacy() {
    let nlp = Nlp::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::var_os("KOKORO_REFERENCE_OUT")
        .map_or_else(|| root.join("../../reference/out"), PathBuf::from)
        .join("spacy-en");
    let mut stats = Stats::default();
    let mut failures = Vec::new();
    for path in jsonl_files(&root.join("tests/data")).into_iter().chain(jsonl_files(&out)) {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        check(&nlp, &name, &read_cases(&path), &mut stats, &mut failures);
    }
    let (cases, skipped) = dump_cases();
    assert_eq!(skipped, 0, "calls whose spaCy inputs couldn't be reconstructed");
    check(&nlp, "reference dumps", &cases, &mut stats, &mut failures);
    for f in failures.iter().take(200) {
        eprintln!("{f}");
    }
    eprintln!(
        "total: {}/{} texts exact; {} tokenization diffs; {} tag diffs in {} tags",
        stats.texts_exact, stats.texts, stats.tokenization_diffs, stats.tag_diffs, stats.tags_compared
    );
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}

#[test]
fn preprocess_replaces_links() {
    assert_eq!(misaki_preprocess("  [Kokoro](/kˈOkəɹO/) is [a] (b) [c](d"), "Kokoro is [a] (b) [c](d");
}

#[derive(Deserialize)]
struct PatternCase {
    text: String,
    prefix: usize,
    suffix: usize,
    infix: Vec<(usize, usize)>,
    url: bool,
}

/// The prefix, suffix, infix and URL patterns on their own, against Python's results on random
/// strings (`reference.py --patterns`).
#[test]
fn patterns_match_python() {
    let nlp = Nlp::new();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = std::env::var_os("KOKORO_REFERENCE_OUT")
        .map_or_else(|| root.join("../../reference/out"), PathBuf::from)
        .join("spacy-en");
    let files: Vec<_> = jsonl_files(&root.join("tests/data"))
        .into_iter()
        .chain(jsonl_files(&out))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("patterns")))
        .collect();
    assert!(!files.is_empty());
    let mut failures = Vec::new();
    let mut total = 0;
    for path in files {
        for line in std::fs::read_to_string(&path).unwrap().lines().filter(|l| !l.trim().is_empty()) {
            let case: PatternCase = serde_json::from_str(line).unwrap();
            total += 1;
            let chars = |bytes: usize| case.text[..bytes].chars().count();
            let m = nlp.pattern_matches(&case.text);
            let suffix = m.suffix.map_or(0, |(a, b)| chars(b) - chars(a));
            let infix: Vec<_> = m.infixes.into_iter().map(|(a, b)| (chars(a), chars(b))).collect();
            let ours = (chars(m.prefix), suffix, infix, m.url);
            let python = (case.prefix, case.suffix, case.infix.clone(), case.url);
            if ours != python {
                failures.push(format!("{:?}: ours {ours:?}, Python {python:?}", case.text));
            }
        }
    }
    for f in failures.iter().take(50) {
        eprintln!("{f}");
    }
    eprintln!("patterns: {}/{total} strings match", total - failures.len());
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}
