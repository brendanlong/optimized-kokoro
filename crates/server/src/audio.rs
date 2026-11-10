//! Kokoro-FastAPI's per-chunk audio post-processing (`services/audio.py`): trimming each chunk
//! to its speech plus padding, which sets the gaps between sentences. Where a sentence was cut at
//! a clause, the cut ends are joined at the model's own timing instead ([`Joins`]).

use crate::protocol::{SAMPLE_RATE, SAMPLES_PER_FRAME};
use crate::stitch::{CLAUSE_IDS, SPACE_ID};
use kokoro_text::py;

/// `gap_trim_ms` = 1: cut from both ends of every chunk.
const EDGE_TRIM: usize = SAMPLE_RATE as usize / 1000;
/// 50 ms kept before the first loud sample.
const PAD_START: usize = 50 * SAMPLE_RATE as usize / 1000;
/// `dynamic_gap_trim_padding_ms`.
const PAD_END_MS: f64 = 410.0;
/// -45 dBFS.
const SILENCE_DB: f64 = -45.0;

/// Seconds of silence as samples, truncated like `int(pause * 24000)`.
pub fn silence(seconds: f64) -> usize {
    (seconds * f64::from(SAMPLE_RATE)) as usize
}

fn end_padding(text: &str, speed: f64) -> usize {
    let multiplier = match py::strip(text).chars().next_back() {
        Some('!') => 0.9,
        Some(',') => 0.8,
        _ => 1.0,
    };
    let pad = ((PAD_END_MS * f64::from(SAMPLE_RATE) * multiplier) / 1000.0) as usize;
    (pad.saturating_sub(PAD_START) as f64 / speed).ceil() as usize
}

/// `AudioService.trim_audio` for a chunk that isn't the stream's last (the reference never marks
/// one as last), keeping `lead` samples before the first loud one (the reference's `PAD_START`):
/// the part of `pcm` kept, as a range.
fn trim(pcm: &[i16], text: &str, speed: f64, lead: usize) -> std::ops::Range<usize> {
    let (offset, len) = if pcm.len() > 2 * EDGE_TRIM { (EDGE_TRIM, pcm.len() - 2 * EDGE_TRIM) } else { (0, pcm.len()) };
    let audio = &pcm[offset..offset + len];
    let threshold = 32767.0 * 10f64.powf(SILENCE_DB / 20.0);
    let loud = |s: &i16| f64::from(*s).abs() > threshold;
    let Some(first) = audio.iter().position(loud) else { return offset..offset + len };
    let last = audio.iter().rposition(loud).expect("there is a loud sample");
    let start = first.saturating_sub(lead);
    let end = (last + end_padding(text, speed)).min(len);
    offset + start..offset + end
}

/// Ids up to a space are punctuation; the ones after it are phonemes.
const FIRST_PHONEME_ID: u8 = SPACE_ID + 1;

/// The index in `ids` of the clause punctuation a chunk cut from its sentence ends at: the last of
/// [`CLAUSE_IDS`] (`;:,`, which the chunker's `，、；：` also are by the time they are phonemes)
/// with no phoneme after it. `None` if there is none (then the end is trimmed as
/// usual).
pub fn final_clause_mark(ids: &[u8]) -> Option<usize> {
    for (i, &id) in ids.iter().enumerate().rev() {
        if CLAUSE_IDS.contains(&id) {
            return Some(i);
        }
        if id >= FIRST_PHONEME_ID {
            return None;
        }
    }
    None
}

/// How a chunk's audio is cut where the chunker split its sentence at clause punctuation, so that
/// the pieces join as the model times its own comma (Kokoro-FastAPI trims these like sentence
/// ends; see `docs/divergences.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Joins {
    /// The sentence started in an earlier chunk: start at the first loud sample, without the 50 ms
    /// before it, so the pause before it is the previous chunk's comma alone. (Not at the end of
    /// the leading boundary id's frames: the first word's onset has started by then, up to 4,140
    /// in amplitude on real speech, so a cut there clips it and clicks.)
    pub start_at_speech: bool,
    /// The sentence goes on in the next chunk: end right after the frames of the rendered id at
    /// this index, the final clause punctuation, instead of 410 ms × 0.8 after the last loud sample.
    pub end_after_id: Option<usize>,
}

/// Where [`Joins`] cut a rendering: whether it starts at its first loud sample, and how many
/// samples to drop at its end. The end is measured on the rendering as the worker sent it, with
/// its durations (one per rendered id, with the boundary id 0 at either end): stitching changes
/// where a piece starts, but a last piece ends where its rendering does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cuts {
    pub start_at_speech: bool,
    pub end: Option<usize>,
}

impl Joins {
    pub fn cuts(&self, durations: &[u16], len: usize) -> Cuts {
        let samples = |ids: usize| {
            let frames: u64 = durations.iter().take(ids).map(|&d| u64::from(d)).sum();
            usize::try_from(frames * SAMPLES_PER_FRAME).unwrap_or(usize::MAX).min(len)
        };
        Cuts {
            start_at_speech: self.start_at_speech,
            end: self.end_after_id.map(|i| len - samples(i.saturating_add(2))),
        }
    }
}

/// The part of a chunk's audio kept: [`trim`], with either end replaced by its join cut.
pub fn keep_range(pcm: &[i16], text: &str, speed: f64, cuts: Cuts) -> std::ops::Range<usize> {
    let mut range = trim(pcm, text, speed, if cuts.start_at_speech { 0 } else { PAD_START });
    if let Some(end) = cuts.end {
        range.end = pcm.len().saturating_sub(end);
    }
    range.start.min(range.end)..range.end
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct TrimCase {
        runs: Vec<(usize, i16)>,
        text: String,
        speed: f64,
        start: usize,
        len: usize,
    }

    #[derive(Deserialize)]
    struct Fixture {
        trim: Vec<TrimCase>,
    }

    fn build(runs: &[(usize, i16)]) -> Vec<i16> {
        runs.iter().flat_map(|&(n, amp)| (0..n).map(move |i| if i % 2 == 0 { amp } else { -amp })).collect()
    }

    #[test]
    fn trims_like_the_reference() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../tests/fixtures/postprocess.json")).expect("fixture parses");
        for case in fixture.trim {
            let pcm = build(&case.runs);
            let range = keep_range(&pcm, &case.text, case.speed, Cuts::default());
            assert_eq!(range.len(), case.len, "{:?} {:?} {}", case.runs, case.text, case.speed);
            if case.len > 0 {
                assert_eq!(&pcm[range], &pcm[case.start..case.start + case.len]);
            }
        }
    }

    #[test]
    fn the_final_clause_mark_may_only_have_punctuation_after_it() {
        let (comma, semicolon, colon, period, quote, space, a) = (3, 1, 2, 4, 15, SPACE_ID, 43);
        assert_eq!(final_clause_mark(&[a, a, comma]), Some(2));
        assert_eq!(final_clause_mark(&[a, semicolon, a, colon, quote]), Some(3));
        assert_eq!(final_clause_mark(&[a, comma, space, quote, space]), Some(1));
        assert_eq!(final_clause_mark(&[a, comma, a]), None);
        assert_eq!(final_clause_mark(&[a, period]), None);
        assert_eq!(final_clause_mark(&[]), None);
    }

    #[test]
    fn joins_cut_at_the_durations() {
        // Boundary, 3 ids, boundary: frames 2, 3, 1, 4, 5 (600 samples each).
        let durations = [2, 3, 1, 4, 5];
        let len = 600 * 15;
        let joins = |start_at_speech, end_after_id| Joins { start_at_speech, end_after_id };
        let cuts = |start_at_speech, end| Cuts { start_at_speech, end };
        assert_eq!(joins(false, None).cuts(&durations, len), Cuts::default());
        // Ends after id 2's frames, so the last id's and the boundary's 5 frames are left out.
        assert_eq!(joins(true, Some(2)).cuts(&durations, len), cuts(true, Some(3000)));
        assert_eq!(joins(false, Some(0)).cuts(&durations, len), cuts(false, Some(6000)));
        // Hostile durations: an index past them, or audio shorter than them, cut nothing past the ends.
        assert_eq!(joins(true, Some(9)).cuts(&durations, len), cuts(true, Some(0)));
        assert_eq!(joins(false, Some(1)).cuts(&durations, 100), cuts(false, Some(0)));
        assert_eq!(joins(false, Some(1)).cuts(&[], 100), cuts(false, Some(100)));
    }

    #[test]
    fn join_cuts_replace_the_trimmed_ends() {
        // 1,200 silent samples, 6,000 loud, 4,800 silent.
        let pcm = build(&[(1200, 0), (6000, 1000), (4800, 0)]);
        let trimmed = keep_range(&pcm, "One,", 1.0, Cuts::default());
        // The 50 ms before the speech reaches the 24 samples always cut, and so does the padding after.
        assert_eq!(trimmed, 24..pcm.len() - 24);
        let both = Cuts { start_at_speech: true, end: Some(3000) };
        assert_eq!(keep_range(&pcm, "One,", 1.0, both), 1200..pcm.len() - 3000);
        let end = Cuts { start_at_speech: false, end: Some(3000) };
        assert_eq!(keep_range(&pcm, "One,", 1.0, end), trimmed.start..pcm.len() - 3000);
        let crossed = Cuts { start_at_speech: true, end: Some(11_000) };
        assert!(keep_range(&pcm, "One,", 1.0, crossed).is_empty());
    }

    #[test]
    fn silence_truncates() {
        assert_eq!(silence(0.5), 12000);
        assert_eq!(silence(0.00001), 0);
    }
}
