//! Kokoro-FastAPI's per-chunk audio post-processing (`services/audio.py`): trimming each chunk
//! to its speech plus padding, which sets the gaps between sentences.

use crate::protocol::SAMPLE_RATE;
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
/// one as last): the part of `pcm` kept, as a range.
pub fn trim_range(pcm: &[i16], text: &str, speed: f64) -> std::ops::Range<usize> {
    let (offset, len) = if pcm.len() > 2 * EDGE_TRIM { (EDGE_TRIM, pcm.len() - 2 * EDGE_TRIM) } else { (0, pcm.len()) };
    let audio = &pcm[offset..offset + len];
    let threshold = 32767.0 * 10f64.powf(SILENCE_DB / 20.0);
    let loud = |s: &i16| f64::from(*s).abs() > threshold;
    let Some(first) = audio.iter().position(loud) else { return offset..offset + len };
    let last = audio.iter().rposition(loud).expect("there is a loud sample");
    let start = first.saturating_sub(PAD_START);
    let end = (last + end_padding(text, speed)).min(len);
    offset + start..offset + end
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
            let range = trim_range(&pcm, &case.text, case.speed);
            assert_eq!(range.len(), case.len, "{:?} {:?} {}", case.runs, case.text, case.speed);
            if case.len > 0 {
                assert_eq!(&pcm[range], &pcm[case.start..case.start + case.len]);
            }
        }
    }

    #[test]
    fn silence_truncates() {
        assert_eq!(silence(0.5), 12000);
        assert_eq!(silence(0.00001), 0);
    }
}
