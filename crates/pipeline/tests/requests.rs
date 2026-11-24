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

use kokoro_pipeline::foreign::{in_foreign_py, script};
use kokoro_pipeline::{Lang, Pipeline};
use kokoro_reference::divergence::is_symbol;
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
    let mut seen = std::collections::HashSet::new();
    paths
        .iter()
        .flat_map(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
                .collect::<Vec<Case>>()
        })
        // The committed fixtures are the start of the dumps in `out/requests`: each request once.
        .filter(|case| seen.insert(case.id.clone()))
        .map(as_the_port_reads)
        .collect()
}

/// A recorded request as the port takes it and the model reads it: links' phonemes in IPA, and the
/// chunks' phonemes as the model's ids in `config.json` symbols.
fn as_the_port_reads(mut case: Case) -> Case {
    case.text = kokoro_reference::with_ipa_links(&case.text);
    for chunk in &mut case.chunks {
        if let Expected::Speech { text, phonemes, .. } = chunk {
            *text = kokoro_reference::with_ipa_links(text);
            *phonemes = kokoro_reference::canonical(phonemes);
        }
    }
    case
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

/// Whether `ours` are `theirs` split finer: a run read by another G2P is a word per word of its
/// text where the counts match, where the reference made it one. The same text and ids.
fn refines(theirs: &[(String, usize, usize)], ours: &[(String, usize, usize)]) -> bool {
    // Words are the text after CJK punctuation is made Western, as English words were.
    let text = |ws: &[(String, usize, usize)]| {
        let words: String = ws.iter().map(|w| w.0.as_str()).collect();
        kokoro_pipeline::foreign::translate_cjk_punctuation(&words).split_whitespace().collect::<String>()
    };
    let ids = |ws: &[(String, usize, usize)]| ws.iter().flat_map(|w| w.1..w.2).collect::<Vec<_>>();
    theirs.is_empty() || text(theirs) == text(ours) && ids(theirs).iter().all(|i| ids(ours).contains(i))
}

fn uses_new_script(text: &str) -> bool {
    text.chars().filter_map(script).any(|s| !in_foreign_py(s))
}

/// The character fuzz of `request_cases.py`: letters of several scripts with digits or symbols
/// glued to them, which the word-by-word check of `kokoro_reference::divergence` can't line up.
/// There, a digit or symbol in the text is counted as the explanation, apart.
fn fuzz(text: &str) -> bool {
    let other_script = |c: char| c.is_alphabetic() && !c.is_ascii() && !matches!(c as u32, 0xC0..=0x24F);
    text.chars().any(other_script) && text.chars().any(|c| c.is_numeric() || is_symbol(c))
}

/// The phonemes of the names the G2P links in `text` of `language` (espeak's code, `en` for English).
fn names_in(text: &str, language: &str) -> Vec<String> {
    static G2P: std::sync::LazyLock<kokoro_g2p_en::G2p> = std::sync::LazyLock::new(|| kokoro_g2p_en::G2p::new(false));
    G2P.names_in(text, language)
}

/// The language a pipeline reads Latin text in, as `link_names` takes it.
fn latin(lang: &str) -> &'static str {
    match lang {
        "e" => "es",
        "f" => "fr",
        "i" => "it",
        "p" => "pt",
        _ => "en",
    }
}

#[test]
fn requests_match_the_reference() {
    let pipelines: Vec<Pipeline> = Lang::ALL.into_iter().map(|l| Pipeline::new(l).unwrap()).collect();
    let cases = cases();
    let mut failures = Vec::new();
    let (mut flagged, mut rechunked, mut new_script, mut stateful, mut numbers, mut fuzzed) = (0, 0, 0, 0, 0, 0);
    let mut roman = 0;
    let mut regrouped = 0;
    let mut links = 0;
    let mut names = 0;
    let mut runs_joined = 0;
    let mut cased = 0;
    let (us, gb) = (kokoro_g2p_en::G2p::new(false), kokoro_g2p_en::G2p::new(true));
    for case in &cases {
        if uses_new_script(&case.text) {
            new_script += 1;
            continue;
        }
        if kokoro_reference::divergence::has_link_with_tap_or_glottal_stop(&case.lang, &case.text)
            || kokoro_reference::divergence::has_link_read_as_text(&case.lang, &case.text)
        {
            links += 1;
            continue;
        }
        let pipeline = pipelines.iter().find(|p| p.lang().code() == case.lang).expect("a pipeline language");
        let mut actual = Vec::new();
        for chunk in pipeline.chunks(&case.text) {
            actual.push(match chunk {
                Ok(c) => match c.pause {
                    Some(pause) => Expected::Pause { pause },
                    None => Expected::Speech {
                        dropped: c.dropped,
                        words: c.words().into_iter().map(|w| (w.text, w.ids.start, w.ids.end)).collect(),
                        text: c.text,
                        phonemes: kokoro_reference::ours(&c.phonemes),
                        ids: c.ids,
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
        let english = matches!(case.lang.as_str(), "a" | "b");
        if STATE_DEPENDENT.contains(&case.id.as_str()) {
            assert!(error.is_none() && expected.len() == actual.len(), "{}", case.id);
            let mut changed = 0;
            for (e, a) in expected.iter().zip(&actual) {
                match (e, a) {
                    (
                        Expected::Speech { text, phonemes, dropped, words, .. },
                        Expected::Speech { text: t, phonemes: p, dropped: d, words: w, .. },
                    ) => {
                        // Everything but the phonemes of the words the carried-over state changed
                        // (and the ids and word positions after them).
                        assert_eq!(text, t, "{}", case.id);
                        if english {
                            assert_eq!(dropped, d, "{}", case.id);
                        }
                        // After the port's spacing where runs join (docs/divergences.md).
                        let phonemes = &kokoro_reference::divergence::respaced(phonemes, p);
                        let (ew, aw): (Vec<&str>, Vec<&str>) = (phonemes.split(' ').collect(), p.split(' ').collect());
                        assert_eq!(ew.len(), aw.len(), "{}", case.id);
                        let differ = ew.iter().zip(&aw).filter(|(x, y)| x != y).count();
                        assert!(differ <= 1, "{}: {differ} words differ in {text:?}", case.id);
                        changed += differ;
                        let names = |ws: &[(String, usize, usize)]| ws.iter().map(|w| w.0.clone()).collect::<Vec<_>>();
                        // Word spans come from each run's words (docs/divergences.md, "Pipeline: one
                        // way to join runs"), as the reference has them only in English.
                        if english {
                            assert_eq!(names(words), names(w), "{}", case.id);
                        }
                        if differ == 0 && english {
                            assert_eq!(*e, a, "{}", case.id);
                        }
                    }
                    _ => assert_eq!(*e, a, "{}", case.id),
                }
            }
            assert!(changed > 0, "{} now matches: take it off STATE_DEPENDENT", case.id);
            stateful += 1;
            continue;
        }
        let flags = expected
            .iter()
            .any(|e| matches!(e, Expected::Speech { phonemes, .. } if without_language_flags(phonemes).is_some()));
        let texts = |chunks: &mut dyn Iterator<Item = &Expected>| -> Vec<String> {
            chunks
                .filter_map(|c| if let Expected::Speech { text, .. } = c { Some(text.clone()) } else { None })
                .collect()
        };
        if flags && texts(&mut expected.iter().copied()) != texts(&mut actual.iter().take(expected.len())) {
            let (theirs, ours) = (texts(&mut expected.iter().copied()).join(" "), texts(&mut actual.iter()).join(" "));
            if theirs != ours {
                // The text differs by its numbers or symbols, not by where it was chunked.
                match kokoro_reference::divergence::explain(&case.text, &theirs, &ours) {
                    Ok(kinds) => {
                        numbers += 1;
                        roman += usize::from(kinds.contains(&kokoro_reference::divergence::Kind::Roman));
                    }
                    Err(_) if fuzz(&case.text) => fuzzed += 1,
                    Err(why) => failures.push(format!("{}: {why}", case.id)),
                }
                continue;
            }
            rechunked += 1;
            continue;
        }
        flagged += usize::from(flags);
        // Runs of different G2Ps are joined with one rule, and word spans come from each run's
        // words (docs/divergences.md, "Pipeline: one way to join runs"): a chunk spaced so, or
        // with other word spans outside English, is counted apart.
        let joined = std::cell::Cell::new(false);
        let named = std::cell::Cell::new(false);
        let same = |e: &Expected, a: &Expected| match (e, a) {
            (Expected::Speech { text, phonemes, .. }, Expected::Speech { text: t, phonemes: p, .. }) if flags => {
                text == t && without_language_flags(phonemes).as_ref().unwrap_or(phonemes) == p
            }
            (
                Expected::Speech { text, phonemes, ids, dropped, words },
                Expected::Speech { text: t, phonemes: p, ids: i, dropped: d, words: w },
            ) if e != a => {
                // A space now where the reference had whitespace the vocabulary lacks (U+2009) or
                // none: the dropped count and the word spans follow.
                let respaced = kokoro_reference::divergence::differs_only_in_run_spacing(text, phonemes, p);
                // Outside English both the reference's and the port's word spans are a guess that
                // pairs text words with phoneme words, which espeak sometimes merges ("from the"):
                // they aren't compared there. Nor is the dropped count, where whitespace the
                // vocabulary lacks inside a run (a tab) is now a space between its words.
                // A name from names.json, read in its own language (docs/divergences.md, "misaki
                // English: names"): its phonemes, and so the ids and word spans after it, differ.
                if text == t
                    && kokoro_reference::divergence::differs_only_in_names(
                        &names_in(text, latin(&case.lang)),
                        phonemes,
                        p,
                    )
                {
                    named.set(true);
                    return true;
                }
                let ok = text == t
                    && (respaced || phonemes == p && ids == i && (!english || dropped == d && refines(words, w)));
                joined.set(joined.get() || ok);
                ok
            }
            _ => e == a,
        };
        let prefix_matches = expected.len() <= actual.len() && expected.iter().zip(&actual).all(|(e, a)| same(e, a));
        let no_error = error.is_none();
        let ok = match error {
            // misaki's `TypeError` on `İ` after punctuation: the port reads it (docs/divergences.md).
            Some(e) if e.starts_with("TypeError: ord() expected a character") => prefix_matches,
            // misaki's `OverflowError` on a number too large to name: the number reader reads it
            // digit by digit (docs/divergences.md, "Numbers").
            Some(e) if e.starts_with("OverflowError") && prefix_matches => {
                numbers += 1;
                continue;
            }
            Some(_) => prefix_matches && actual[expected.len()..].iter().any(|a| matches!(a, Expected::Error { .. })),
            None => prefix_matches && expected.len() == actual.len(),
        };
        if ok && named.get() {
            names += 1;
            continue;
        }
        if ok && joined.get() {
            runs_joined += 1;
            continue;
        }
        // Pauses and errors stay in, as chunks of their own, so they must match too.
        let pairs = |chunks: &mut dyn Iterator<Item = &Expected>| -> Vec<(String, String)> {
            chunks
                .map(|c| match c {
                    Expected::Speech { text, phonemes, .. } => (text.clone(), phonemes.clone()),
                    other => (format!("{other:?}"), String::new()),
                })
                .collect()
        };
        // A chunk with the same text on both sides must be the same, ids and words included.
        let shared_equal = actual.iter().all(|a| {
            expected.iter().all(|e| match (e, a) {
                (Expected::Speech { text, .. }, Expected::Speech { text: t, .. }) if text == t => same(e, a),
                _ => true,
            })
        });
        // Short clauses are regrouped on purpose (docs/divergences.md).
        if !ok
            && no_error
            && shared_equal
            && kokoro_reference::clause_regrouping(
                &pairs(&mut expected.iter().copied()),
                &pairs(&mut actual.iter()),
                kokoro_text::MIN_CLAUSE_CHUNK,
                None,
            )
            .is_some_and(|n| n > 0)
        {
            regrouped += 1;
            continue;
        }
        if !ok {
            // Numbers and symbols are read differently on purpose (docs/divergences.md): where the
            // chunks' text differs only by them, the request is counted apart.
            let joined = |chunks: &mut dyn Iterator<Item = &Expected>| texts(chunks).join(" ");
            let (theirs, ours) = (joined(&mut expected.iter().copied()), joined(&mut actual.iter()));
            if theirs != ours
                && let Ok(kinds) = kokoro_reference::divergence::explain(&case.text, &theirs, &ours)
            {
                numbers += 1;
                roman += usize::from(kinds.contains(&kokoro_reference::divergence::Kind::Roman));
                continue;
            }
            // The character fuzz glues digits and symbols to letters of many scripts in ways the
            // word-by-word check can't line up: there a digit or symbol in the text is counted as
            // the explanation, apart.
            // The reference said nothing where the port reads a symbol's name ("[😀").
            let silent_symbols = theirs.is_empty() && case.text.chars().any(|c| !c.is_ascii() && is_symbol(c));
            if theirs != ours && (fuzz(&case.text) || silent_symbols) {
                fuzzed += 1;
                continue;
            }
            // Case is read differently on purpose (docs/divergences.md, "misaki English: case").
            if case.lang == "a" && us.differs_from_misaki_by_case(&case.text)
                || case.lang == "b" && gb.differs_from_misaki_by_case(&case.text)
            {
                cased += 1;
                continue;
            }
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
    let compared =
        cases.len() - new_script - rechunked - stateful - numbers - fuzzed - regrouped - links - runs_joined - cased;
    println!(
        "{} of {compared} requests match ({flagged} of them once espeak's language-switch flags are dropped from the reference's); \
         not compared: {rechunked} chunked differently without the flags, {new_script} with other scripts, \
         {stateful} where the reference's espeak read a word by an earlier call's state, \
         {numbers} with numbers or symbols read differently ({roman} with Roman numerals), {fuzzed} more fuzzed ones with digits or symbols, \
         {regrouped} with short clauses regrouped, {links} with a link the reference read otherwise (as text, or ɾ and ʔ as the English flap and t), {runs_joined} with runs joined by the port's rule, \
         {cased} with case read differently, {names} with a name read in its own language",
        compared - failures.len(),
    );
    for f in failures.iter().take(10) {
        println!("{f}");
    }
    assert!(failures.is_empty(), "{} requests differ", failures.len());
}
