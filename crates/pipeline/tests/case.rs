//! Case doesn't change how a word is read: sentences written all in capitals read as they do
//! written normally (docs/divergences.md, "misaki English: case"). `case_sentences.txt` is 400
//! sentences picked at random from four Project Gutenberg books (public domain: The Time
//! Machine, Pride and Prejudice, The Adventures of Sherlock Holmes, Frankenstein), written by
//! `reference/case_sentences.py`.
//!
//! The score is the share of words read alike, word by word in order. The few misses left are
//! the light stress of a capital after "!" or "?", which isn't always a sentence's start.

use kokoro_pipeline::{Lang, Pipeline};

fn read(pipeline: &Pipeline, text: &str) -> Vec<String> {
    pipeline
        .chunks(text)
        .map(|c| c.unwrap().phonemes)
        .flat_map(|p| p.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
        .collect()
}

/// How many words of `a` the longest common subsequence with `b` keeps.
fn kept(a: &[String], b: &[String]) -> usize {
    let mut row = vec![0; b.len() + 1];
    for x in a {
        let mut diagonal = 0;
        for (j, y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y { diagonal + 1 } else { row[j + 1].max(row[j]) };
            diagonal = above;
        }
    }
    row[b.len()]
}

#[test]
fn capitals_read_as_normal_case() {
    let pipeline = Pipeline::new(Lang::AmericanEnglish).unwrap();
    let (mut words, mut same) = (0, 0);
    let mut differ = Vec::new();
    for sentence in include_str!("case_sentences.txt").lines().chain(NUMBERS) {
        let (normal, capitals) = (read(&pipeline, sentence), read(&pipeline, &sentence.to_uppercase()));
        let k = kept(&normal, &capitals);
        words += normal.len();
        same += k;
        if k < normal.len() {
            differ.push(format!("{sentence}\n  normal:   {}\n  capitals: {}", normal.join(" "), capitals.join(" ")));
        }
    }
    let share = same as f64 / words as f64;
    println!("{same}/{words} words read alike ({:.1}%)", 100.0 * share);
    for d in differ.iter().take(std::env::var("CASE_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(10)) {
        println!("{d}");
    }
    assert!(share >= TARGET, "{:.1}% of words read alike, below {:.1}%", 100.0 * share, 100.0 * TARGET);
}

/// Sentences with what the normalizer writes words for, which the books hardly have: numbers,
/// money, times, percentages and abbreviations.
const NUMBERS: [&str; 6] = [
    "Dr. Watson met Holmes in 1812.",
    "Mr. Bennet paid $5 for 3 books at 10:30 in the morning.",
    "The 2nd edition sold 40% more copies than the first.",
    "St. Louis is 300 miles from Chicago, and the trip takes 5 hours.",
    "In 1999, Mrs. Hudson rented the rooms for £12 a week.",
    "Chapter XXXV begins on page 412 of the 1818 edition.",
];

/// main read 73.4% alike, before the caps pass was replaced (issue #69); this reads 99.96%.
const TARGET: f64 = 0.995;
