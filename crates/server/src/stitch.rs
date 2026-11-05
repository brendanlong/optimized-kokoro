//! Overlap stitching (Kokoro-FastAPI `inference/stitching.py`): a chunk with more phonemes than
//! one chunk may hold is rendered as overlapping pieces. Each piece is rendered with the words
//! around it, cut mid-way through the space between two words using the model's durations, and
//! crossfaded into the next.
//!
//! Kokoro-FastAPI stitches above 450 phonemes and renders each piece with up to 510 ids. Here a
//! worker request also has to fit the worker's chunk cap, so at a low cap or a slow speed both
//! shrink to [`max_render_ids`] (a divergence, `docs/divergences.md`). The chunker cuts sentences
//! at clause punctuation to that size first (`stream::max_chunk_phonemes`), so only a clause too
//! long by itself is stitched.

use crate::protocol::{SAMPLES_PER_FRAME, Speed};

/// `settings.absolute_max_tokens`: chunks with more phonemes than this are stitched.
pub const STITCH_ABOVE: usize = 450;
const MODEL_LIMIT: usize = 510;
const CROSSFADE: usize = 480;
const WORDS_AHEAD: usize = 4;
const MAX_PIECE: usize = MODEL_LIMIT - 60;
/// The frames to allow for a request of n ids at speed 1 (`frames × speed` is about constant):
/// `FRAMES_PER_ID_TENTHS / 10 × n + FRAMES_OVERHEAD`. The overhead is the boundary ids and the
/// pauses at a request's edges, which make short requests longer per id. Measured with
/// `worker/bench/frames_per_id.py`: fp16 durations at speeds 0.25 and 1 for every corpus chunk, for
/// windows of 8 to 510 ids of the corpus rendered with longer style rows up to 510 (as stitched
/// pieces are, `Plan::style_len`), and for the reference fixtures' other-language and mixed-script
/// chunks, in 8 English voices and the fixtures' own languages' voices. Every one of the 146,762
/// cases was under `3.8 n + 116` frames (the most: 109 frames for 8 ids, 340 for 64, 1,050 for
/// 256, 1,763 for 510; all af_nicole). `4.0 n + 120` leaves 8-10% over that for 64-256 ids, and
/// still allows 510 ids at the default cap (60 s) at speed 1, so nothing changes there.
const FRAMES_PER_ID_TENTHS: u64 = 40;
const FRAMES_OVERHEAD: u64 = 120;
/// Kokoro v1.0's id for a space.
pub const SPACE_ID: u8 = 16;
/// Kokoro v1.0's ids for `;`, `:` and `,`.
pub const CLAUSE_IDS: [u8; 3] = [1, 2, 3];
/// How far back from a piece's longest end clause punctuation is preferred to the last space.
const CLAUSE_SHARE_PERCENT: usize = 40;

fn find_space(ids: &[u8], from: usize) -> Option<usize> {
    ids.get(from..)?.iter().position(|&id| id == SPACE_ID).map(|i| from + i)
}

/// `[start, end)` ranges of at most `max_len` ids, ending just after clause punctuation (`,;:`)
/// followed by a space when there is one in the back `CLAUSE_SHARE` of the range, else at the last
/// space, where possible. The space between two pieces is in neither. Kokoro-FastAPI only looks for
/// spaces (a divergence, `docs/divergences.md`); a cut in a pause crossfades two silences.
fn pieces(ids: &[u8], max_len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < ids.len() {
        let mut end = start + max_len;
        if end >= ids.len() {
            end = ids.len();
        } else if let Some(space) = clause_space(ids, start, end, max_len) {
            end = space;
        } else if let Some(space) = ids[start + 1..=end].iter().rposition(|&id| id == SPACE_ID) {
            end = start + 1 + space;
        }
        out.push((start, end));
        start = if end < ids.len() && ids[end] == SPACE_ID { end + 1 } else { end };
    }
    out
}

/// The last space in `ids[start + 1..=end]` that follows clause punctuation, if it is in the back
/// `CLAUSE_SHARE` of a piece of `max_len`.
fn clause_space(ids: &[u8], start: usize, end: usize, max_len: usize) -> Option<usize> {
    let from = (start + max_len - max_len * CLAUSE_SHARE_PERCENT / 100).max(start + 2);
    (from..=end).rev().find(|&i| ids[i] == SPACE_ID && CLAUSE_IDS.contains(&ids[i - 1]))
}

/// The ids a piece is rendered with: up to `WORDS_AHEAD` words after it, and as much before it
/// as fits, starting at a word.
fn context(ids: &[u8], start: usize, end: usize, window: usize) -> (usize, usize) {
    let mut stop = end;
    for _ in 0..=WORDS_AHEAD {
        if stop >= ids.len() {
            stop = ids.len();
            break;
        }
        stop = find_space(ids, stop + 1).unwrap_or(ids.len());
    }
    let mut first = stop.saturating_sub(window);
    if first > 0
        && let Some(word) = find_space(ids, first)
        && word < start
    {
        first = word + 1;
    }
    first = first.min(start.saturating_sub(1));
    (first, stop.min(first + window))
}

/// The most ids one request to the worker may have, at `speed`, for its audio to stay within a
/// chunk cap of `max_chunk_samples`: `min(510, (cap frames × speed − 120) / 4)`, 0 if the cap is
/// that small (then [`Plan::new`] still renders windows of 2). A chunk with more ids is stitched
/// from pieces rendered with at most this many. The worker still refuses a chunk whose predicted
/// audio is over its cap.
pub fn max_render_ids(max_chunk_samples: u32, speed: Speed) -> usize {
    let frames = u64::from(max_chunk_samples) / SAMPLES_PER_FRAME * u64::from(speed.hundredths()) / 100;
    let ids = frames.saturating_sub(FRAMES_OVERHEAD) * 10 / FRAMES_PER_ID_TENTHS;
    usize::try_from(ids).unwrap_or(usize::MAX).min(MODEL_LIMIT)
}

/// How a long chunk is split: which ids each piece is rendered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    ids: Vec<u8>,
    ranges: Vec<(usize, usize)>,
    contexts: Vec<(usize, usize)>,
}

impl Plan {
    /// Pieces rendered with at most `render_ids` ids each (see [`max_render_ids`]). At 510 they are the
    /// reference's: at most 450 ids, with up to 510 of context. Below that, both shrink in
    /// proportion.
    pub fn new(ids: Vec<u8>, render_ids: usize) -> Self {
        // A piece needs at least one id of context before it, so a window holds two ids or more.
        let window = render_ids.clamp(2, MODEL_LIMIT);
        let piece = (window * MAX_PIECE / MODEL_LIMIT).clamp(1, window - 1);
        let ranges = pieces(&ids, piece);
        let contexts = ranges.iter().map(|&(start, end)| context(&ids, start, end, window)).collect();
        Self { ids, ranges, contexts }
    }

    pub fn pieces(&self) -> usize {
        self.ranges.len()
    }

    pub fn context_ids(&self, piece: usize) -> &[u8] {
        let (first, stop) = self.contexts[piece];
        &self.ids[first..stop]
    }

    /// The style length the reference renders every piece with: the whole chunk's.
    pub fn style_len(&self) -> u16 {
        self.ids.len().min(MODEL_LIMIT) as u16
    }
}

/// Carries the crossfade tail from one piece to the next. Pieces must be fed in order.
#[derive(Debug, Default)]
pub struct Stitcher {
    held: Option<Vec<i16>>,
}

impl Stitcher {
    /// Cuts piece `index`'s rendering, in place, to the part that belongs to it, crossfaded with
    /// the previous piece's tail.
    pub fn piece(&mut self, plan: &Plan, index: usize, mut audio: Vec<i16>, durations: &[u16]) -> Vec<i16> {
        let (start, _) = plan.ranges[index];
        let (first, _) = plan.contexts[index];
        let mut frames = Vec::with_capacity(durations.len() + 1);
        let mut total = 0u64;
        frames.push(0);
        for &d in durations {
            total += u64::from(d) * SAMPLES_PER_FRAME;
            frames.push(total);
        }
        let frame = |i: usize| frames.get(i).or(frames.last()).copied().unwrap_or(0) as usize;
        let cut = |boundary: usize| {
            if plan.ids[boundary - 1] == SPACE_ID {
                let i = boundary - 1 - first;
                (frame(1 + i) + frame(2 + i)) / 2
            } else {
                frame(1 + boundary - first)
            }
        };
        let last = index + 1 == plan.pieces();
        let begin = if index == 0 { 0 } else { cut(start).saturating_sub(CROSSFADE / 2) }.min(audio.len());
        let finish = if last { audio.len() } else { cut(plan.ranges[index + 1].0).saturating_sub(CROSSFADE / 2) }
            .clamp(begin, audio.len());
        let next_held = (!last).then(|| audio[finish..(finish + CROSSFADE).min(audio.len())].to_vec());
        audio.truncate(finish);
        audio.drain(..begin);
        let mut out = audio;
        if let Some(held) = self.held.take() {
            let n = held.len().min(out.len());
            for (i, (o, h)) in out.iter_mut().zip(&held).take(n).enumerate() {
                let ramp = ramp(i, n);
                *o = (f32::from(*h) * (1.0 - ramp) + f32::from(*o) * ramp) as i16;
            }
        }
        self.held = next_held;
        out
    }
}

/// `np.linspace(0, 1, n, dtype=np.float32)[i]`.
fn ramp(i: usize, n: usize) -> f32 {
    if n < 2 {
        0.0
    } else if i + 1 == n {
        1.0
    } else {
        (i as f64 * (1.0 / (n - 1) as f64)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Piece {
        len: usize,
        head: Vec<i16>,
        rest_sum: i64,
        rest_weighted: i64,
    }

    #[derive(Deserialize)]
    struct Case {
        ids: Vec<u8>,
        contexts: Vec<Vec<u8>>,
        pieces: Vec<Piece>,
    }

    #[derive(Deserialize)]
    struct Fixture {
        stitch: Vec<Case>,
    }

    /// The fake model `postprocess.py` renders with.
    fn render(ids: &[u8], row: usize) -> (Vec<i16>, Vec<u16>) {
        let mut durations = vec![2];
        durations.extend(ids.iter().enumerate().map(|(j, &id)| 1 + ((id as usize * 31 + j * 7 + row) % 4) as u16));
        durations.push(2);
        let total = 600 * durations.iter().map(|&d| d as usize).sum::<usize>();
        let seed = ids.iter().map(|&id| id as usize).sum::<usize>() % 997;
        let audio = (0..total).map(|k| ((k * 13 + seed) % 4001) as i16 - 2000).collect();
        (audio, durations)
    }

    #[test]
    fn stitches_like_the_reference() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../tests/fixtures/postprocess.json")).expect("fixture parses");
        for case in fixture.stitch {
            let plan = Plan::new(case.ids, MODEL_LIMIT);
            let contexts: Vec<_> = (0..plan.pieces()).map(|i| plan.context_ids(i).to_vec()).collect();
            assert_eq!(contexts, case.contexts);
            let mut stitcher = Stitcher::default();
            for (index, expected) in case.pieces.iter().enumerate() {
                let (audio, durations) = render(plan.context_ids(index), plan.style_len().into());
                let out = stitcher.piece(&plan, index, audio, &durations);
                assert_eq!(out.len(), expected.len);
                for (a, b) in out.iter().zip(&expected.head) {
                    assert!((a - b).abs() <= 1, "piece {index}: {a} vs {b}");
                }
                let rest: Vec<i64> = out.iter().skip(480).map(|&s| s as i64).collect();
                assert_eq!(rest.iter().sum::<i64>(), expected.rest_sum);
                let weighted: i64 = rest.iter().enumerate().map(|(i, s)| s * (i as i64 % 1009)).sum();
                assert_eq!(weighted, expected.rest_weighted);
            }
        }
    }

    #[test]
    fn render_ids_follow_the_cap_and_the_speed() {
        assert_eq!(max_render_ids(1_440_000, Speed::NORMAL), MODEL_LIMIT);
        assert_eq!(max_render_ids(720_000, Speed::NORMAL), 270);
        assert_eq!(max_render_ids(720_000, Speed::from_hundredths(50).unwrap()), 120);
        assert_eq!(max_render_ids(720_000, Speed::MIN), 45);
        assert_eq!(max_render_ids(1_440_000, Speed::MIN), 120);
        assert_eq!(max_render_ids(600_000, Speed::MIN), 32);
        assert_eq!(max_render_ids(24_000 * 12, Speed::MIN), 0);
        assert_eq!(max_render_ids(720_000, Speed::MAX), MODEL_LIMIT);
        assert_eq!(max_render_ids(1, Speed::NORMAL), 0);
    }

    /// Words of 3 to 9 ids.
    fn words(n: usize) -> Vec<u8> {
        let mut ids = Vec::new();
        for w in 0.. {
            if !ids.is_empty() {
                ids.push(SPACE_ID);
            }
            ids.extend((0..3 + w * 5 % 7).map(|j| 43 + (j % 5) as u8));
            if ids.len() >= n {
                break;
            }
        }
        ids
    }

    #[test]
    fn small_windows_render_every_id_within_the_window() {
        let ids = words(1200);
        for window in [0, 1, 2, 3, 10, 32, 45, 120, 270, 509, 510, 2000] {
            let plan = Plan::new(ids.clone(), window);
            let limit = window.clamp(2, MODEL_LIMIT);
            let mut covered = Vec::new();
            for (i, (&(start, end), &(first, stop))) in plan.ranges.iter().zip(&plan.contexts).enumerate() {
                assert!(stop - first <= limit, "window {window}, piece {i}: {first}..{stop}");
                assert!(first <= start.saturating_sub(1) && end <= stop, "window {window}, piece {i}");
                // Pieces end at a space where one fits; that space is in neither.
                let gap = &ids[covered.len()..start];
                assert!(gap.is_empty() || gap == [SPACE_ID], "window {window}, piece {i}");
                covered.extend_from_slice(&ids[covered.len()..end]);
            }
            assert_eq!(covered, ids, "window {window}: the pieces are the chunk");
        }
        let longest = Plan::new(ids.clone(), 510).ranges.iter().map(|&(s, e)| e - s).max().unwrap();
        assert!((440..=450).contains(&longest), "{longest}");
    }

    /// With durations that don't depend on the context, stitched pieces give the chunk's length.
    #[test]
    fn stitching_small_pieces_keeps_the_length() {
        let ids = words(700);
        for window in [3, 10, 45, 270, 510] {
            let plan = Plan::new(ids.clone(), window);
            let mut stitcher = Stitcher::default();
            let total: usize = (0..plan.pieces())
                .map(|i| {
                    let n = plan.context_ids(i).len() + 2;
                    stitcher.piece(&plan, i, vec![1; 600 * 2 * n], &vec![2; n]).len()
                })
                .sum();
            assert_eq!(total, 600 * 2 * (ids.len() + 2), "window {window}: {} pieces", plan.pieces());
        }
    }

    /// A piece ends after the last comma (or `;`, `:`) in the back 40% of its longest range, else at
    /// its last space.
    #[test]
    fn pieces_end_at_clause_punctuation_near_their_end() {
        let a = 43;
        let comma = 3;
        // Words of 4 ids; a comma ends the word at `at` (the comma replacing its last id).
        let ids_with_comma = |at: usize| -> Vec<u8> {
            let mut ids = Vec::new();
            for w in 0..40 {
                if w > 0 {
                    ids.push(SPACE_ID);
                }
                ids.extend([a; 4]);
                if w == at {
                    *ids.last_mut().unwrap() = comma;
                }
            }
            ids
        };
        // 40 words of 4 ids and 39 spaces: 199 ids, word w's space at 5w + 4. Pieces of at most 100
        // ids, so a comma counts from the space at 60: word 12's does, word 11's doesn't, and word
        // 20's is past the piece.
        for (at, end) in [(15, 79), (12, 64), (11, 99), (19, 99), (20, 99)] {
            let ids = ids_with_comma(at);
            let first = pieces(&ids, 100)[0];
            assert_eq!(first, (0, end), "comma ending word {at}");
            if end != 99 {
                assert_eq!((ids[end - 1], ids[end]), (comma, SPACE_ID));
            }
        }
    }

    #[test]
    fn hostile_durations_dont_panic() {
        let ids: Vec<u8> = (0..1200).map(|i| if i % 7 == 6 { SPACE_ID } else { 43 }).collect();
        let plan = Plan::new(ids, MODEL_LIMIT);
        let mut stitcher = Stitcher::default();
        for index in 0..plan.pieces() {
            let n = plan.context_ids(index).len() + 2;
            stitcher.piece(&plan, index, vec![5; 10], &vec![0; n]);
            stitcher.piece(&plan, index, Vec::new(), &[]);
        }
    }
}
