//! Short sentences rendered together: consecutive chunks are merged into one until it has at least
//! [`MIN_SENTENCE_CHUNK`] phonemes, so Kokoro reads a short sentence ("No.", "Chapter one.") with
//! the sentences around it and pauses between them itself. Kokoro-FastAPI renders every sentence
//! alone (a divergence, `docs/divergences.md`).
//!
//! Each sentence keeps its own phonemes (the chunks' G2P is unchanged); a merged chunk is their
//! phonemes joined by a space. Chunks never merge past the request's chunk limit, and never across
//! a `[pause:Xs]` (or an error, or a chunk without ids).

use crate::pipeline::{Chunk, PipelineError, Split};
use crate::stitch::SPACE_ID;
use std::collections::VecDeque;

/// The fewest phonemes a chunk should have. Rendered alone, Kokoro reads a short input louder and
/// slower than the same words in a sentence (`worker/bench/short_words.py`: 1–2.5 dB louder, and
/// Whisper heard "No." alone as "now"), and clauses under 25 phonemes are drawn out up to 1.9×
/// (`split::MIN_CLAUSE_CHUNK`). Chosen from the listening test (`worker/bench/merge_short.py`).
pub const MIN_SENTENCE_CHUNK: usize = 50;

fn len(chunk: &Chunk) -> usize {
    chunk.phonemes.chars().count()
}

fn fits(a: &Chunk, b: &Chunk, max: usize) -> bool {
    len(a) + 1 + len(b) <= max
}

/// `a` then `b` as one chunk: texts and phonemes joined by a space. It starts where `a` does and
/// ends where `b` does, so clause cuts at its ends still join at the model's timing.
fn join(mut a: Chunk, b: Chunk) -> Chunk {
    a.text.push(' ');
    a.text.push_str(&b.text);
    a.phonemes.push(' ');
    a.phonemes.push_str(&b.phonemes);
    a.ids.push(SPACE_ID);
    a.ids.extend(b.ids);
    a.unread += b.unread;
    a.split = Split { before: a.split.before, after: b.split.after };
    a
}

/// [`merge_short`]'s iterator.
pub struct MergeShort<I> {
    chunks: I,
    max: usize,
    /// A finished chunk, held back in case the chunk after it ends short and joins it.
    held: Option<Chunk>,
    /// The chunk being built while it is short: it takes the next if they fit.
    current: Option<Chunk>,
    ready: VecDeque<Result<Chunk, PipelineError>>,
    done: bool,
}

/// Merges consecutive speech chunks of `chunks`: a chunk of fewer than [`MIN_SENTENCE_CHUNK`]
/// phonemes takes the next one while they fit within `max` phonemes together. One still short
/// (the next doesn't fit, or a pause or the end comes next) joins the chunk before it if they fit.
/// Pauses, errors and chunks without ids are never merged, and nothing merges across them. A chunk
/// is handed out once the chunk after it is long enough, or can no longer join it.
pub fn merge_short<I: Iterator<Item = Result<Chunk, PipelineError>>>(chunks: I, max: usize) -> MergeShort<I> {
    MergeShort { chunks, max, held: None, current: None, ready: VecDeque::new(), done: false }
}

impl<I> MergeShort<I> {
    fn push(&mut self, next: Chunk) {
        let chunk = match self.current.take() {
            Some(current) if fits(&current, &next, self.max) => join(current, next),
            Some(current) => {
                self.finish(current);
                next
            }
            None => next,
        };
        if len(&chunk) >= MIN_SENTENCE_CHUNK {
            self.finish(chunk);
        } else {
            // The held chunk can go once the short one being built can no longer join it.
            if self.held.as_ref().is_some_and(|held| !fits(held, &chunk, self.max)) {
                self.ready.extend(self.held.take().map(Ok));
            }
            self.current = Some(chunk);
        }
    }

    fn finish(&mut self, chunk: Chunk) {
        match self.held.take() {
            Some(held) if len(&chunk) < MIN_SENTENCE_CHUNK && fits(&held, &chunk, self.max) => {
                self.held = Some(join(held, chunk));
            }
            held => {
                self.ready.extend(held.map(Ok));
                self.held = Some(chunk);
            }
        }
    }

    fn flush(&mut self) {
        if let Some(current) = self.current.take() {
            self.finish(current);
        }
        self.ready.extend(self.held.take().map(Ok));
    }
}

impl<I: Iterator<Item = Result<Chunk, PipelineError>>> Iterator for MergeShort<I> {
    type Item = Result<Chunk, PipelineError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(chunk) = self.ready.pop_front() {
                return Some(chunk);
            }
            if self.done {
                return None;
            }
            match self.chunks.next() {
                Some(Ok(chunk)) if chunk.pause.is_none() && !chunk.ids.is_empty() => self.push(chunk),
                Some(other) => {
                    self.flush();
                    self.ready.push_back(other);
                }
                None => {
                    self.flush();
                    self.done = true;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sentence of `n` phonemes, named by its text.
    fn s(text: &str, n: usize) -> Result<Chunk, PipelineError> {
        Ok(Chunk {
            text: text.into(),
            phonemes: "x".repeat(n),
            ids: vec![43; n],
            pause: None,
            unread: 0,
            split: Split::default(),
        })
    }

    fn pause() -> Result<Chunk, PipelineError> {
        Ok(Chunk {
            text: String::new(),
            phonemes: String::new(),
            ids: Vec::new(),
            pause: Some(1.0),
            unread: 0,
            split: Split::default(),
        })
    }

    fn merged(chunks: Vec<Result<Chunk, PipelineError>>, max: usize) -> Vec<String> {
        merge_short(chunks.into_iter(), max)
            .map(|c| match c {
                Ok(c) if c.pause.is_some() => "<pause>".to_owned(),
                Ok(c) => {
                    assert_eq!(c.phonemes.chars().count(), c.ids.len());
                    c.text
                }
                Err(e) => format!("<{e}>"),
            })
            .collect()
    }

    const MIN: usize = MIN_SENTENCE_CHUNK;

    #[test]
    fn short_sentences_merge_until_long_enough() {
        // "Hi. Hi. Blah blah." is one chunk, and the one after it its own.
        let short = [s("Hi.", 3), s("Hi.", 3), s("Blah blah.", MIN - 8), s("Long.", MIN), s("Next.", MIN)];
        assert_eq!(merged(short.to_vec(), 450), ["Hi. Hi. Blah blah.", "Long.", "Next."]);
        // A short sentence takes the long one after it.
        assert_eq!(merged(vec![s("No.", 3), s("Long.", 200), s("More.", 200)], 450), ["No. Long.", "More."]);
        // Long sentences stay one chunk each.
        assert_eq!(merged(vec![s("A.", MIN), s("B.", 300), s("C.", MIN)], 450), ["A.", "B.", "C."]);
    }

    #[test]
    fn a_short_sentence_that_cannot_go_forward_joins_the_one_before() {
        // At the end of the text.
        assert_eq!(merged(vec![s("Long.", 100), s("Yes.", 4)], 450), ["Long. Yes."]);
        // Before a sentence it doesn't fit with.
        assert_eq!(merged(vec![s("Long.", 100), s("No.", 3), s("Huge.", 447)], 450), ["Long. No.", "Huge."]);
        // Alone when neither fits.
        assert_eq!(merged(vec![s("Huge.", 447), s("No.", 3), s("Huge.", 447)], 450), ["Huge.", "No.", "Huge."]);
        assert_eq!(merged(vec![s("No.", 3)], 450), ["No."]);
    }

    #[test]
    fn nothing_merges_past_the_limit_or_across_a_pause() {
        let merged_len = |chunks, max| -> Vec<usize> {
            merge_short(Vec::into_iter(chunks), max).map(|c| c.unwrap().phonemes.chars().count()).collect()
        };
        assert_eq!(merged_len(vec![s("a", 10), s("b", 10), s("c", 10)], 21), [21, 10]);
        assert!(merged_len(vec![s("a", 10); 40], 120).iter().all(|&n| n <= 120));
        let around = vec![s("One.", 4), pause(), s("Two.", 4), s("Three.", 6), pause()];
        assert_eq!(merged(around, 450), ["One.", "<pause>", "Two. Three.", "<pause>"]);
        let failed = vec![s("One.", 4), Err(PipelineError("bad".into())), s("Two.", 4)];
        assert_eq!(merged(failed, 450), ["One.", "<bad>", "Two."]);
    }

    #[test]
    fn a_merged_chunk_keeps_its_outer_clause_cuts() {
        let cut =
            |text: &str, n, before, after| s(text, n).map(|c| Chunk { split: Split { before, after }, unread: 1, ..c });
        let chunks =
            vec![cut("No.", 3, false, false), cut("First half,", 200, false, true), cut("rest.", 250, true, false)];
        let out: Vec<Chunk> = merge_short(chunks.into_iter(), 450).map(Result::unwrap).collect();
        assert_eq!(out.len(), 2);
        assert_eq!(
            (out[0].text.as_str(), out[0].split, out[0].unread),
            ("No. First half,", Split { before: false, after: true }, 2)
        );
        assert_eq!(out[0].ids[3], SPACE_ID);
        assert_eq!(out[1].split, Split { before: true, after: false });
    }
}
