//! Overlap stitching (Kokoro-FastAPI `inference/stitching.py`): a chunk with more phonemes than
//! one chunk may hold is rendered as overlapping pieces. Each piece is rendered with the words
//! around it, cut mid-way through the space between two words using the model's durations, and
//! crossfaded into the next.

use crate::protocol::SAMPLES_PER_FRAME;

/// `settings.absolute_max_tokens`: chunks with more phonemes than this are stitched.
pub const STITCH_ABOVE: usize = 450;
const MODEL_LIMIT: usize = 510;
const CROSSFADE: usize = 480;
const WORDS_AHEAD: usize = 4;
const MAX_PIECE: usize = MODEL_LIMIT - 60;
/// Kokoro v1.0's id for a space.
pub const SPACE_ID: u8 = 16;

fn find_space(ids: &[u8], from: usize) -> Option<usize> {
    ids.get(from..)?.iter().position(|&id| id == SPACE_ID).map(|i| from + i)
}

/// `[start, end)` ranges of at most `max_len` ids, ending at spaces where possible. The space
/// between two pieces is in neither.
fn pieces(ids: &[u8], max_len: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < ids.len() {
        let mut end = start + max_len;
        if end >= ids.len() {
            end = ids.len();
        } else if let Some(space) = ids[start + 1..=end].iter().rposition(|&id| id == SPACE_ID) {
            end = start + 1 + space;
        }
        out.push((start, end));
        start = if end < ids.len() && ids[end] == SPACE_ID { end + 1 } else { end };
    }
    out
}

/// The ids a piece is rendered with: up to `WORDS_AHEAD` words after it, and as much before it
/// as fits, starting at a word.
fn context(ids: &[u8], start: usize, end: usize) -> (usize, usize) {
    let mut stop = end;
    for _ in 0..=WORDS_AHEAD {
        if stop >= ids.len() {
            stop = ids.len();
            break;
        }
        stop = find_space(ids, stop + 1).unwrap_or(ids.len());
    }
    let mut first = stop.saturating_sub(MODEL_LIMIT);
    if first > 0
        && let Some(word) = find_space(ids, first)
        && word < start
    {
        first = word + 1;
    }
    first = first.min(start.saturating_sub(1));
    (first, stop.min(first + MODEL_LIMIT))
}

/// How a long chunk is split: which ids each piece is rendered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    ids: Vec<u8>,
    ranges: Vec<(usize, usize)>,
    contexts: Vec<(usize, usize)>,
}

impl Plan {
    pub fn new(ids: Vec<u8>) -> Self {
        let ranges = pieces(&ids, MAX_PIECE.min(STITCH_ABOVE));
        let contexts = ranges.iter().map(|&(start, end)| context(&ids, start, end)).collect();
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
    /// The part of piece `index`'s rendering that belongs to it, crossfaded with the previous
    /// piece's tail.
    pub fn piece(&mut self, plan: &Plan, index: usize, audio: &[i16], durations: &[u16]) -> Vec<i16> {
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
        let mut out = audio[begin..finish].to_vec();
        if let Some(held) = self.held.take() {
            let n = held.len().min(out.len());
            for (i, (o, h)) in out.iter_mut().zip(&held).take(n).enumerate() {
                let ramp = ramp(i, n);
                *o = (f32::from(*h) * (1.0 - ramp) + f32::from(*o) * ramp) as i16;
            }
        }
        if !last {
            self.held = Some(audio[finish..(finish + CROSSFADE).min(audio.len())].to_vec());
        }
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
            let plan = Plan::new(case.ids);
            let contexts: Vec<_> = (0..plan.pieces()).map(|i| plan.context_ids(i).to_vec()).collect();
            assert_eq!(contexts, case.contexts);
            let mut stitcher = Stitcher::default();
            for (index, expected) in case.pieces.iter().enumerate() {
                let (audio, durations) = render(plan.context_ids(index), plan.style_len().into());
                let out = stitcher.piece(&plan, index, &audio, &durations);
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
    fn hostile_durations_dont_panic() {
        let ids: Vec<u8> = (0..1200).map(|i| if i % 7 == 6 { SPACE_ID } else { 43 }).collect();
        let plan = Plan::new(ids);
        let mut stitcher = Stitcher::default();
        for index in 0..plan.pieces() {
            let n = plan.context_ids(index).len() + 2;
            stitcher.piece(&plan, index, &[5; 10], &vec![0; n]);
            stitcher.piece(&plan, index, &[], &[]);
        }
    }
}
