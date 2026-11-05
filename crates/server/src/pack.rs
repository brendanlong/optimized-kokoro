//! Sentences packed into chunks: consecutive chunks of the pipeline (whole sentences, and the
//! clause pieces of sentences too long for one chunk) are joined greedily up to a soft size, so
//! Kokoro reads short sentences with their neighbours and pauses between them itself. Kokoro-FastAPI
//! renders every sentence alone (a divergence, `docs/divergences.md`).
//!
//! - **Greedy:** a chunk takes the next unit while they fit within [`SOFT_MAX_PHONEMES`] together;
//!   a unit longer than that is a chunk by itself.
//! - **Cleanup:** a chunk left under [`MIN_CHUNK`] phonemes joins the shorter of the chunks before
//!   and after it that it fits with, past the soft size if need be.
//! - **Never** past the request's chunk limit (the one the clause split uses, so a sentence that
//!   fits one chunk is never split to make room), and never across a `[pause:Xs]`, an error, or a
//!   chunk without ids.
//!
//! Each unit keeps its own phonemes (G2P is unchanged); a packed chunk is their phonemes joined by
//! a space.

use crate::pipeline::{Chunk, PipelineError, Split};
use crate::stitch::SPACE_ID;
use std::collections::VecDeque;

/// The size a chunk is filled to. Decoding at batch 1 on the 3060 Ti takes about 13 ms plus
/// 0.25 ms per id (43 ms at 130 ids, 126 ms at 450; `worker/bench/pack_listen.py`), so this bounds
/// the first chunk's decoding to about 45 ms (the clean-up may add a short sentence) while still
/// reading a few short sentences together. A constant: it trades first-audio latency against
/// context, and a faster GPU only makes every size cheaper.
pub const SOFT_MAX_PHONEMES: usize = 130;

/// The fewest phonemes a chunk should have, as for chunks cut from a sentence: Kokoro drawls a
/// short input rendered alone (`kokoro_text::MIN_CLAUSE_CHUNK`; `worker/bench/short_words.py`:
/// louder, and Whisper heard "No." alone as "now").
pub const MIN_CHUNK: usize = kokoro_text::MIN_CLAUSE_CHUNK;

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

/// [`pack`]'s iterator.
pub struct Pack<I> {
    chunks: I,
    soft: usize,
    max: usize,
    /// The last packed chunk, held back while a short chunk after it may still join it.
    held: Option<Chunk>,
    /// A short chunk the greedy pass finished, waiting for the chunk after it to choose a neighbour.
    short: Option<Chunk>,
    /// The chunk being filled.
    current: Option<Chunk>,
    ready: VecDeque<Result<Chunk, PipelineError>>,
    done: bool,
}

/// Packs the speech chunks of `chunks` (see the module docs) to `soft` phonemes, never past `max`.
/// A chunk is handed out once nothing after it can join it.
pub fn pack<I: Iterator<Item = Result<Chunk, PipelineError>>>(chunks: I, soft: usize, max: usize) -> Pack<I> {
    Pack {
        chunks,
        soft: soft.min(max),
        max,
        held: None,
        short: None,
        current: None,
        ready: VecDeque::new(),
        done: false,
    }
}

impl<I> Pack<I> {
    fn push(&mut self, next: Chunk) {
        let current = match self.current.take() {
            Some(current) if fits(&current, &next, self.soft) => join(current, next),
            Some(current) => {
                self.finish(Some(current));
                next
            }
            None => next,
        };
        // A long enough chunk being filled stays long enough, so nothing will join the held one.
        if self.short.is_none() && len(&current) >= MIN_CHUNK {
            self.ready.extend(self.held.take().map(Ok));
        }
        self.current = Some(current);
    }

    /// The cleanup, on each chunk the greedy pass finishes (`None` before a pause or the end): a
    /// short chunk waits for the next, then joins whichever neighbour it fits with that is
    /// shorter (the one before on a tie), or stays alone.
    fn finish(&mut self, next: Option<Chunk>) {
        let mut next = next;
        if let Some(short) = self.short.take() {
            let before = self.held.as_ref().filter(|held| fits(held, &short, self.max)).map(len);
            let after = next.as_ref().filter(|next| fits(&short, next, self.max)).map(len);
            match (before, after) {
                (Some(b), a) if a.is_none_or(|a| b <= a) => {
                    self.held = self.held.take().map(|held| join(held, short));
                }
                (_, Some(_)) => next = next.map(|next| join(short, next)),
                _ => {
                    self.ready.extend(self.held.take().map(Ok));
                    self.held = Some(short);
                }
            }
        }
        match next {
            Some(chunk) if len(&chunk) < MIN_CHUNK => self.short = Some(chunk),
            Some(chunk) => {
                self.ready.extend(self.held.take().map(Ok));
                self.held = Some(chunk);
            }
            None => {}
        }
    }

    fn flush(&mut self) {
        let current = self.current.take();
        if current.is_some() {
            self.finish(current);
        }
        self.finish(None);
        self.ready.extend(self.held.take().map(Ok));
    }
}

impl<I: Iterator<Item = Result<Chunk, PipelineError>>> Iterator for Pack<I> {
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

    /// A unit of `n` phonemes, named by its text.
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

    /// A clause piece of a sentence cut at clauses.
    fn piece(text: &str, n: usize, before: bool, after: bool) -> Result<Chunk, PipelineError> {
        s(text, n).map(|c| Chunk { split: Split { before, after }, ..c })
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

    fn packed_with(chunks: Vec<Result<Chunk, PipelineError>>, soft: usize, max: usize) -> Vec<String> {
        pack(chunks.into_iter(), soft, max)
            .map(|c| match c {
                Ok(c) if c.pause.is_some() => "<pause>".to_owned(),
                Ok(c) => {
                    assert_eq!(c.phonemes.chars().count(), c.ids.len());
                    assert!(c.ids.len() <= max);
                    c.text
                }
                Err(e) => format!("<{e}>"),
            })
            .collect()
    }

    fn packed(chunks: Vec<Result<Chunk, PipelineError>>) -> Vec<String> {
        packed_with(chunks, 130, 450)
    }

    #[test]
    fn sentences_are_packed_greedily_to_the_soft_size() {
        let run =
            vec![s("Hi.", 3), s("Hi.", 3), s("Blah blah.", 10), s("A.", 40), s("B.", 40), s("C.", 40), s("D.", 40)];
        // 3 + 3 + 10 + 40 + 40 and the spaces is 100; with C it would be 141.
        assert_eq!(packed(run), ["Hi. Hi. Blah blah. A. B.", "C. D."]);
        // A sentence longer than the soft size is a chunk by itself.
        assert_eq!(packed(vec![s("A.", 60), s("Long.", 300), s("B.", 60)]), ["A.", "Long.", "B."]);
        assert_eq!(packed(vec![s("A.", 60), s("B.", 69), s("C.", 60)]), ["A. B.", "C."]);
    }

    #[test]
    fn a_short_chunk_left_over_joins_a_neighbour_within_the_limit() {
        // "Hi." before a sentence that fits whole: they go together if both fit, else "Hi." stays.
        assert_eq!(packed(vec![s("Hi.", 3), s("Long.", 300)]), ["Hi. Long."]);
        assert_eq!(packed(vec![s("Hi.", 3), s("Long.", 447)]), ["Hi.", "Long."]);
        // The chunk before is preferred, past the soft size.
        assert_eq!(packed(vec![s("A.", 128), s("No.", 3), s("Long.", 300)]), ["A. No.", "Long."]);
        // Else the one after; at the end, the one before.
        assert_eq!(packed(vec![s("Long.", 447), s("No.", 3), s("Long.", 300)]), ["Long.", "No. Long."]);
        assert_eq!(packed(vec![s("Long.", 300), s("Yes.", 4)]), ["Long. Yes."]);
        assert_eq!(packed(vec![s("No.", 3)]), ["No."]);
        // Of two neighbours it fits with, the shorter.
        let lens = [3, 3, 10, 8, 10, 10, 11, 9];
        let run = lens.iter().enumerate().map(|(i, &n)| s(&i.to_string(), n)).collect();
        assert_eq!(packed_with(run, 30, 70), ["0 1 2 3", "4 5 6 7"]);
        // Short chunks on either side of a long one: each joins it in turn.
        assert_eq!(packed(vec![s("No.", 3), s("Long.", 300), s("Yes.", 4)]), ["No. Long. Yes."]);
    }

    #[test]
    fn a_short_sentence_packs_with_the_clause_piece_next_to_it() {
        let chunks = vec![s("Hi.", 3), piece("First half,", 400, false, true), piece("rest.", 300, true, false)];
        let out: Vec<Chunk> = pack(chunks.into_iter(), 130, 450).map(Result::unwrap).collect();
        let shape: Vec<_> = out.iter().map(|c| (c.text.as_str(), c.split)).collect();
        assert_eq!(
            shape,
            [
                ("Hi. First half,", Split { before: false, after: true }),
                ("rest.", Split { before: true, after: false })
            ]
        );
        assert_eq!(out[0].ids[3], SPACE_ID);
        let after = vec![piece("First half,", 400, false, true), piece("rest.", 300, true, false), s("Hi.", 3)];
        assert_eq!(packed(after), ["First half,", "rest. Hi."]);
    }

    #[test]
    fn nothing_crosses_the_limit_or_a_pause() {
        // A limit under the soft size is the soft size.
        assert_eq!(packed_with(vec![s("a", 10), s("b", 10), s("c", 10)], 130, 21), ["a b", "c"]);
        assert_eq!(packed_with(vec![s("a", 10); 40], 130, 120).len(), 4);
        let around = vec![s("One.", 4), pause(), s("Two.", 4), s("Three.", 6), pause()];
        assert_eq!(packed(around), ["One.", "<pause>", "Two. Three.", "<pause>"]);
        let failed = vec![s("One.", 4), Err(PipelineError("bad".into())), s("Two.", 4)];
        assert_eq!(packed(failed), ["One.", "<bad>", "Two."]);
        let unread = |n| s("x.", 40).map(|c| Chunk { unread: n, ..c });
        let out: Vec<Chunk> = pack(vec![unread(1), unread(2)].into_iter(), 130, 450).map(Result::unwrap).collect();
        assert_eq!(out.iter().map(|c| c.unread).collect::<Vec<_>>(), [3]);
    }
}
