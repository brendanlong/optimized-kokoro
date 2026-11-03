//! Replays every English G2P call recorded by reference/dump.py and tools/gen_cases.py: spaCy's tokens and the espeak answers
//! come from the recording, everything else is this crate. Foreign-script runs are spliced in from the
//! recording the way kokoro does, so whole calls can be compared.
//!
//! Prints match rates and mismatches (`PARITY_SHOW=n` shows up to n, default 30).

use kokoro_g2p_en::{Fallback, G2p, TaggedToken, Tagger, Token, kokoro};
use kokoro_reference::{Call, SpacyToken};
use std::collections::{HashMap, VecDeque};

struct Recorded<'a> {
    calls: VecDeque<&'a [SpacyToken]>,
    problems: Vec<String>,
}

impl Tagger for Recorded<'_> {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken> {
        let Some(tokens) = self.calls.pop_front() else {
            self.problems.push(format!("spaCy called more often than recorded, on {text:?}"));
            return Vec::new();
        };
        let joined: String = tokens.iter().flat_map(|t| [t.text.as_str(), t.whitespace.as_str()]).collect();
        if joined != text {
            self.problems.push(format!("spaCy input differs: recorded {joined:?}, got {text:?}"));
        }
        tokens
            .iter()
            .map(|t| TaggedToken { text: t.text.clone(), tag: t.tag.clone(), whitespace: t.whitespace.clone() })
            .collect()
    }
}

struct Espeak<'a> {
    answers: HashMap<&'a str, Option<&'a str>>,
    problems: Vec<String>,
}

impl Fallback for Espeak<'_> {
    fn phonemize(&mut self, word: &str) -> Option<String> {
        match self.answers.get(word) {
            Some(ps) => ps.map(str::to_string),
            None => {
                self.problems.push(format!("fallback asked for {word:?}, which wasn't recorded"));
                None
            }
        }
    }
}

/// `KPipeline.en_g2p_tokens` with the recorded segmentation and foreign phonemes.
fn replay(g2p: &G2p, call: &Call) -> (Vec<Token>, Vec<String>) {
    let mut tagger = Recorded { calls: call.spacy.iter().map(Vec::as_slice).collect(), problems: Vec::new() };
    let mut espeak = Espeak {
        answers: call.fallback.iter().map(|(w, ps)| (w.as_str(), ps.as_deref())).collect(),
        problems: Vec::new(),
    };
    let mut foreign = call.foreign.iter();
    let mut tokens = Vec::new();
    let mut problems = Vec::new();
    for (segment, language) in &call.segments {
        if language.is_some() {
            let Some((_, _, ps)) = foreign.next() else {
                problems.push("foreign segment wasn't recorded".to_string());
                continue;
            };
            let words: Vec<&str> = ps.split_whitespace().collect();
            tokens.extend(words.iter().enumerate().map(|(i, w)| Token {
                text: if i == 0 { segment.clone() } else { String::new() },
                tag: String::new(),
                whitespace: if i + 1 < words.len() { " ".to_string() } else { String::new() },
                phonemes: Some(w.to_string()),
            }));
            continue;
        }
        if let Err(e) = kokoro::append_segment(g2p, &mut tokens, segment, &mut tagger, Some(&mut espeak)) {
            problems.push(format!("error: {e}"));
        }
    }
    problems.extend(tagger.problems);
    problems.extend(espeak.problems);
    (tokens, problems)
}

fn show(t: &Token) -> String {
    format!("{:?}/{}/{:?} -> {:?}", t.text, t.tag, t.whitespace, t.phonemes.as_deref().unwrap_or(""))
}

#[test]
fn matches_reference() {
    let mut records = kokoro_reference::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/misaki_cases.jsonl"));
    records.extend(kokoro_reference::all());
    let us = G2p::new(false);
    let gb = G2p::new(true);
    let show_max: usize = std::env::var("PARITY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(30);
    let (mut calls, mut calls_ok, mut tokens, mut tokens_ok) = (0usize, 0usize, 0usize, 0usize);
    let mut shown = 0;
    for record in records.iter().filter(|r| r.lang == "a" || r.lang == "b") {
        let g2p = if record.lang == "b" { &gb } else { &us };
        for call in &record.calls {
            calls += 1;
            let (got, problems) = replay(g2p, call);
            let expected: Vec<Token> = call
                .tokens
                .iter()
                .map(|t| Token {
                    text: t.text.clone(),
                    tag: t.tag.clone(),
                    whitespace: t.whitespace.clone(),
                    phonemes: t.phonemes.clone(),
                })
                .collect();
            let got_norm: Vec<Token> = got
                .iter()
                .map(|t| Token { phonemes: Some(t.phonemes.clone().unwrap_or_default()), ..t.clone() })
                .collect();
            let same_tokens = got_norm.len() == expected.len();
            let token_matches = got_norm.iter().zip(&expected).filter(|(a, b)| a == b).count();
            tokens += expected.len();
            tokens_ok += if same_tokens { token_matches } else { 0 };
            let ps = kokoro::tokens_to_ps(&got);
            let ok = problems.is_empty() && same_tokens && token_matches == expected.len() && ps == call.phonemes;
            calls_ok += usize::from(ok);
            if !ok && shown < show_max {
                shown += 1;
                println!("--- {} [{}] {:?}", record.id, record.lang, call.input);
                for p in &problems {
                    println!("  problem: {p}");
                }
                if ps != call.phonemes {
                    println!("  expected: {}\n  got:      {}", call.phonemes, ps);
                }
                for i in 0..got_norm.len().max(expected.len()) {
                    let (a, b) = (got_norm.get(i), expected.get(i));
                    if a != b {
                        println!(
                            "  token {i}: expected {}, got {}",
                            b.map_or("-".to_string(), show),
                            a.map_or("-".to_string(), show)
                        );
                    }
                }
            }
        }
    }
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    println!(
        "calls: {calls_ok}/{calls} exact ({:.3}%); tokens: {tokens_ok}/{tokens} exact ({:.3}%)",
        pct(calls_ok, calls),
        pct(tokens_ok, tokens)
    );
    assert!(calls > 0);
    assert_eq!(calls_ok, calls, "calls differ from the reference");
}
