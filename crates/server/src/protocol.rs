//! Worker protocol v3 frames (`docs/worker-protocol.md`): length-prefixed, little-endian, no floats.
//!
//! Synthesize is `u8 type, u16 slot, u8 priority, u8 waiting, u16 speed (hundredths),
//! u16 voice_index, u16 n_ids, u16 style_len, n_ids × u8 id`. The style vector is row `style_len - 1` of the voice
//! pack (`1..=max_phonemes`): `n_ids` for a normal chunk, and the whole clause's length (capped at
//! 510) for each piece of a stitched one.
//!
//! What the front accepts from the worker, beyond the frame layouts:
//! - Hello comes first and only once, with version 3, `2 <= vocab_size <= 256`,
//!   `max_phonemes == 510`, `max_batch >= 1`, `slots >= 1`, `sample_rate == 24000`,
//!   `max_chunk_samples >= 1` and at most the configured ceiling (`worker_max_chunk_samples`), and
//!   1 or more distinct voice names of 1 to 64 bytes of `[a-z0-9_]`.
//! - Every other frame names a busy slot, and ends that slot's request.
//! - Result: exactly `n_ids + 2` durations, each from 1 to `ceil(50 / speed)` (Kokoro's `max_dur`
//!   is 50 sigmoid bins, divided by speed), then exactly `600 × Σ durations` samples (Kokoro v1.0
//!   renders 600 samples per duration unit; measured exact on the real model), no more than the
//!   request's chunk cap, `ceil(max_chunk_samples / speed)` (see [`Speed::chunk_cap`]).
//! - Failed: code 1 (cancelled) only after the front sent Cancel for that slot, 2 (internal
//!   error), or 3 (too long: the predicted audio exceeds the chunk cap).
//! - Each length prefix is checked against its frame type's bound before anything is allocated:
//!   Hello at most 25 + 65,535 × 65 bytes, Failed exactly 4, and Result at most what its slot's
//!   request allows, then exactly what its durations imply, so the reader asks the caller what
//!   the slot holds.
//!
//! Anything else closes the connection, failing every request in flight on it.

use std::fmt;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

pub const MAGIC: [u8; 4] = *b"KOKW";
pub const VERSION: u16 = 3;
pub const MAX_PHONEMES: u16 = 510;
pub const SAMPLE_RATE: u32 = 24_000;
/// Kokoro v1.0 renders 600 samples per predicted duration unit, so a Result has exactly
/// `600 * sum(durations)` samples.
pub const SAMPLES_PER_FRAME: u64 = 600;
/// Synthesize's fixed part: type, slot, priority, waiting, speed, voice_index, n_ids, style_len.
const SYNTHESIZE_FIXED: usize = 1 + 2 + 1 + 1 + 2 + 2 + 2 + 2;
/// The largest front→worker body: a Synthesize with `MAX_PHONEMES` ids.
pub const MAX_FRONT_BODY: usize = SYNTHESIZE_FIXED + MAX_PHONEMES as usize;
const CANCEL_LEN: usize = 1 + 2;
/// Kokoro v1.0's `max_dur`: an id's duration is a sum of this many sigmoids, divided by speed.
const DURATION_BINS: u32 = 50;
pub const MAX_VOICE_NAME: usize = 64;

const TYPE_HELLO: u8 = 0x01;
const TYPE_RESULT: u8 = 0x02;
const TYPE_FAILED: u8 = 0x03;
const TYPE_SYNTHESIZE: u8 = 0x10;
const TYPE_CANCEL: u8 = 0x11;

/// Fixed part of Hello: type, magic, version, vocab_size, max_phonemes, max_batch, slots,
/// sample_rate, max_chunk_samples, n_voices.
const HELLO_FIXED: usize = 1 + 4 + 2 + 2 + 2 + 2 + 2 + 4 + 4 + 2;
const HELLO_MAX: usize = HELLO_FIXED + u16::MAX as usize * (1 + MAX_VOICE_NAME);
const FAILED_LEN: usize = 1 + 2 + 1;
/// Result's header: type, slot, n.
const RESULT_HEADER: usize = 1 + 2 + 2;

/// A speaking rate in hundredths of normal speed, 25..=400 (0.25× to 4×).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Speed(u16);

impl Speed {
    pub const MIN: Speed = Speed(25);
    pub const MAX: Speed = Speed(400);
    pub const NORMAL: Speed = Speed(100);

    pub fn from_hundredths(hundredths: u16) -> Option<Self> {
        (Self::MIN.0..=Self::MAX.0).contains(&hundredths).then_some(Self(hundredths))
    }

    /// The nearest hundredth to `speed`, if `speed` is from 0.25 to 4.
    pub fn from_f64(speed: f64) -> Option<Self> {
        (Self::MIN.as_f64()..=Self::MAX.as_f64()).contains(&speed).then(|| Self((speed * 100.0).round() as u16))
    }

    pub fn hundredths(self) -> u16 {
        self.0
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / 100.0
    }

    /// The longest duration Kokoro can predict for one id at this speed, `ceil(50 / speed)`. It
    /// bounds how much audio the front accepts for a request.
    pub fn max_duration(self) -> u16 {
        (DURATION_BINS * 100).div_ceil(u32::from(self.0)) as u16
    }

    /// The most samples the worker produces for one chunk at this speed:
    /// `ceil(max_chunk_samples / speed)`, in integers as the worker computes it.
    pub fn chunk_cap(self, max_chunk_samples: u32) -> u64 {
        (u64::from(max_chunk_samples) * 100).div_ceil(u64::from(self.0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub vocab_size: u16,
    pub max_phonemes: u16,
    pub max_batch: u16,
    /// Requests the worker accepts at once; each is named by a slot index below this.
    pub slots: u16,
    /// The worker's output cap for one chunk at speed 1, in samples.
    pub max_chunk_samples: u32,
    pub voices: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    Cancelled,
    Internal,
    /// The predicted output is over the chunk cap; retrying the same chunk won't help.
    TooLong,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerFrame {
    Hello(Hello),
    Result { slot: u16, durations: Vec<u16>, pcm: Vec<i16> },
    Failed { slot: u16, code: FailCode },
}

/// What a Result for a busy slot must look like, from the request in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expected {
    pub n_ids: u16,
    pub speed: Speed,
    pub max_chunk_samples: u32,
}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Violation(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "worker connection: {e}"),
            Self::Violation(why) => write!(f, "worker protocol violation: {why}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<io::Error> for ReadError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn violation<T>(why: impl Into<String>) -> Result<T, ReadError> {
    Err(ReadError::Violation(why.into()))
}

struct Cursor<'a>(&'a [u8]);

impl Cursor<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
        let Some((head, rest)) = self.0.split_first_chunk::<N>() else { return violation("frame body too short") };
        self.0 = rest;
        Ok(*head)
    }
    fn u8(&mut self) -> Result<u8, ReadError> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, ReadError> {
        self.take().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Result<u32, ReadError> {
        self.take().map(u32::from_le_bytes)
    }
    fn bytes(&mut self, n: usize) -> Result<&[u8], ReadError> {
        if self.0.len() < n {
            return violation("frame body too short");
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }
    fn finish(self) -> Result<(), ReadError> {
        if self.0.is_empty() { Ok(()) } else { violation("frame body too long") }
    }
}

fn parse_hello(body: &[u8]) -> Result<Hello, ReadError> {
    let mut c = Cursor(body);
    c.u8()?;
    if c.take::<4>()? != MAGIC {
        return violation("bad hello magic");
    }
    let version = c.u16()?;
    if version != VERSION {
        return violation(format!("unsupported protocol version {version}"));
    }
    let vocab_size = c.u16()?;
    if !(2..=256).contains(&vocab_size) {
        return violation(format!("vocab_size {vocab_size} outside 2..=256"));
    }
    let max_phonemes = c.u16()?;
    if max_phonemes != MAX_PHONEMES {
        return violation(format!("max_phonemes {max_phonemes}, expected {MAX_PHONEMES}"));
    }
    let max_batch = c.u16()?;
    if max_batch == 0 {
        return violation("max_batch is 0");
    }
    let slots = c.u16()?;
    if slots == 0 {
        return violation("no slots");
    }
    let sample_rate = c.u32()?;
    if sample_rate != SAMPLE_RATE {
        return violation(format!("sample_rate {sample_rate}, expected {SAMPLE_RATE}"));
    }
    let max_chunk_samples = c.u32()?;
    if max_chunk_samples == 0 {
        return violation("max_chunk_samples is 0");
    }
    let n_voices = c.u16()?;
    if n_voices == 0 {
        return violation("no voices");
    }
    let mut voices = Vec::with_capacity(n_voices.into());
    let mut seen = std::collections::HashSet::with_capacity(n_voices.into());
    for _ in 0..n_voices {
        let len = c.u8()?;
        let name = c.bytes(len.into())?;
        if name.is_empty()
            || name.len() > MAX_VOICE_NAME
            || !name.iter().all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return violation("bad voice name");
        }
        let name = String::from_utf8(name.to_vec()).expect("checked ASCII");
        if !seen.insert(name.clone()) {
            return violation(format!("voice {name} listed twice"));
        }
        voices.push(name);
    }
    c.finish()?;
    Ok(Hello { vocab_size, max_phonemes, max_batch, slots, max_chunk_samples, voices })
}

/// Reads one frame. `expected(slot)` describes the request in `slot`, or is `None` if the slot is
/// free; a Result is checked against it as it's read.
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    expected: impl FnOnce(u16) -> Option<Expected>,
) -> Result<WorkerFrame, ReadError> {
    let len = reader.read_u32_le().await? as usize;
    if len == 0 {
        return violation("empty frame");
    }
    let kind = reader.read_u8().await?;
    let max = match kind {
        TYPE_HELLO => HELLO_MAX,
        TYPE_FAILED => FAILED_LEN,
        TYPE_RESULT => return read_result(reader, len, expected).await,
        _ => return violation(format!("unknown frame type {kind:#04x}")),
    };
    if len > max {
        return violation(format!("frame type {kind:#04x} is {len} bytes, more than {max}"));
    }
    let mut body = vec![0; len];
    body[0] = kind;
    reader.read_exact(&mut body[1..]).await?;
    match kind {
        TYPE_HELLO => parse_hello(&body).map(WorkerFrame::Hello),
        _ => parse_failed(&body),
    }
}

fn parse_failed(body: &[u8]) -> Result<WorkerFrame, ReadError> {
    let mut c = Cursor(body);
    c.u8()?;
    let slot = c.u16()?;
    let code = match c.u8()? {
        1 => FailCode::Cancelled,
        2 => FailCode::Internal,
        3 => FailCode::TooLong,
        other => return violation(format!("unknown failure code {other}")),
    };
    c.finish()?;
    Ok(WorkerFrame::Failed { slot, code })
}

async fn read_result<R: AsyncRead + Unpin>(
    reader: &mut R,
    len: usize,
    expected: impl FnOnce(u16) -> Option<Expected>,
) -> Result<WorkerFrame, ReadError> {
    if len < RESULT_HEADER {
        return violation("result frame too short");
    }
    let slot = reader.read_u16_le().await?;
    let Some(e) = expected(slot) else { return violation(format!("result for slot {slot}, which is free")) };
    let n = usize::from(e.n_ids) + 2;
    let max_samples = e.speed.chunk_cap(e.max_chunk_samples);
    let header = RESULT_HEADER + 2 * n;
    if len < header || (len - header) as u64 > 2 * max_samples {
        return violation(format!("slot {slot}: a result of {len} bytes for {} ids", e.n_ids));
    }
    let got = usize::from(reader.read_u16_le().await?);
    if got != n {
        return violation(format!("slot {slot}: {got} durations for {} ids", e.n_ids));
    }
    let mut raw = vec![0; 2 * n];
    reader.read_exact(&mut raw).await?;
    let durations: Vec<u16> = raw.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    let max_duration = e.speed.max_duration();
    if durations.iter().any(|&d| d == 0 || d > max_duration) {
        return violation(format!("slot {slot}: a duration of 0 or over {max_duration}"));
    }
    let samples = durations.iter().map(|&d| u64::from(d)).sum::<u64>() * SAMPLES_PER_FRAME;
    if samples > max_samples {
        return violation(format!("slot {slot}: {samples} samples, over the chunk cap {max_samples}"));
    }
    if (len - header) as u64 != 2 * samples {
        return violation(format!("slot {slot}: {} bytes of audio, durations say {samples} samples", len - header));
    }
    // Decoded through a small buffer, so only the samples themselves are ever held.
    let mut pcm = Vec::with_capacity(samples as usize);
    let mut buf = [0; 16 * 1024];
    let mut left = 2 * samples as usize;
    while left > 0 {
        let take = left.min(buf.len());
        reader.read_exact(&mut buf[..take]).await?;
        pcm.extend(buf[..take].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])));
        left -= take;
    }
    Ok(WorkerFrame::Result { slot, durations, pcm })
}

/// How urgent a chunk is. The worker serves the greatest first (then the oldest): the request's
/// level, higher first, and within a level a chunk a listener is waiting for before prefetch.
/// The derived order is exactly that, lexicographic on `(level, waiting)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Priority {
    /// The request's priority, 0..=255, higher more urgent.
    pub level: u8,
    /// A listener is waiting for this chunk now (the first in its request's window).
    pub waiting: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Synthesize<'a> {
    pub slot: u16,
    pub priority: Priority,
    pub speed: Speed,
    pub voice: u16,
    pub ids: &'a [u8],
    pub style_len: u16,
}

fn frame(body: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Encodes a Synthesize frame. The caller has checked every field against the worker's Hello.
pub fn encode_synthesize(s: &Synthesize) -> Vec<u8> {
    let mut body = Vec::with_capacity(SYNTHESIZE_FIXED + s.ids.len());
    body.push(TYPE_SYNTHESIZE);
    body.extend_from_slice(&s.slot.to_le_bytes());
    body.push(s.priority.level);
    body.push(u8::from(s.priority.waiting));
    body.extend_from_slice(&s.speed.hundredths().to_le_bytes());
    body.extend_from_slice(&s.voice.to_le_bytes());
    body.extend_from_slice(&(s.ids.len() as u16).to_le_bytes());
    body.extend_from_slice(&s.style_len.to_le_bytes());
    body.extend_from_slice(s.ids);
    debug_assert!(body.len() <= MAX_FRONT_BODY);
    frame(body)
}

pub fn encode_cancel(slot: u16) -> Vec<u8> {
    let mut body = Vec::with_capacity(CANCEL_LEN);
    body.push(TYPE_CANCEL);
    body.extend_from_slice(&slot.to_le_bytes());
    frame(body)
}

/// Encoders for the worker's side, for tests and fake workers.
pub mod worker_side {
    use super::*;

    pub fn hello(h: &Hello) -> Vec<u8> {
        let mut body = vec![TYPE_HELLO];
        body.extend_from_slice(&MAGIC);
        body.extend_from_slice(&VERSION.to_le_bytes());
        body.extend_from_slice(&h.vocab_size.to_le_bytes());
        body.extend_from_slice(&h.max_phonemes.to_le_bytes());
        body.extend_from_slice(&h.max_batch.to_le_bytes());
        body.extend_from_slice(&h.slots.to_le_bytes());
        body.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        body.extend_from_slice(&h.max_chunk_samples.to_le_bytes());
        body.extend_from_slice(&(h.voices.len() as u16).to_le_bytes());
        for v in &h.voices {
            body.push(v.len() as u8);
            body.extend_from_slice(v.as_bytes());
        }
        frame(body)
    }

    pub fn result(slot: u16, durations: &[u16], pcm: &[i16]) -> Vec<u8> {
        let mut body = vec![TYPE_RESULT];
        body.extend_from_slice(&slot.to_le_bytes());
        body.extend_from_slice(&(durations.len() as u16).to_le_bytes());
        for d in durations {
            body.extend_from_slice(&d.to_le_bytes());
        }
        for s in pcm {
            body.extend_from_slice(&s.to_le_bytes());
        }
        frame(body)
    }

    pub fn failed(slot: u16, code: FailCode) -> Vec<u8> {
        let mut body = vec![TYPE_FAILED];
        body.extend_from_slice(&slot.to_le_bytes());
        body.push(match code {
            FailCode::Cancelled => 1,
            FailCode::Internal => 2,
            FailCode::TooLong => 3,
        });
        frame(body)
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum FrontFrame {
        Synthesize { slot: u16, priority: u8, waiting: u8, speed: u16, voice: u16, ids: Vec<u8>, style_len: u16 },
        Cancel { slot: u16 },
    }

    /// Reads one front→worker frame, checking only its structure.
    pub async fn read_front<R: AsyncRead + Unpin>(reader: &mut R) -> Result<FrontFrame, ReadError> {
        let len = reader.read_u32_le().await? as usize;
        if len == 0 || len > MAX_FRONT_BODY {
            return violation(format!("front frame of {len} bytes"));
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).await?;
        let mut c = Cursor(&body);
        let frame = match c.u8()? {
            TYPE_SYNTHESIZE => {
                let slot = c.u16()?;
                let priority = c.u8()?;
                let waiting = c.u8()?;
                let speed = c.u16()?;
                let voice = c.u16()?;
                let n_ids = c.u16()?;
                let style_len = c.u16()?;
                let ids = c.bytes(n_ids.into())?.to_vec();
                FrontFrame::Synthesize { slot, priority, waiting, speed, voice, ids, style_len }
            }
            TYPE_CANCEL => FrontFrame::Cancel { slot: c.u16()? },
            other => return violation(format!("unknown front frame type {other:#04x}")),
        };
        c.finish()?;
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::worker_side::*;
    use super::*;

    fn hello() -> Hello {
        Hello {
            vocab_size: 178,
            max_phonemes: 510,
            max_batch: 8,
            slots: 64,
            max_chunk_samples: 2_880_000,
            voices: vec!["af_heart".into(), "am_adam".into()],
        }
    }

    fn expect(n_ids: u16) -> Expected {
        Expected { n_ids, speed: Speed::NORMAL, max_chunk_samples: 2_880_000 }
    }

    async fn read(bytes: &[u8], expected: Option<Expected>) -> Result<WorkerFrame, ReadError> {
        read_frame(&mut &bytes[..], |_| expected).await
    }

    #[tokio::test]
    async fn worker_frames_round_trip() {
        assert_eq!(read(&worker_side::hello(&hello()), None).await.unwrap(), WorkerFrame::Hello(hello()));
        let pcm = vec![7i16; 600 * 6];
        assert_eq!(
            read(&result(7, &[1, 2, 3], &pcm), Some(expect(1))).await.unwrap(),
            WorkerFrame::Result { slot: 7, durations: vec![1, 2, 3], pcm }
        );
        assert_eq!(
            read(&failed(9, FailCode::Cancelled), None).await.unwrap(),
            WorkerFrame::Failed { slot: 9, code: FailCode::Cancelled }
        );
        assert_eq!(
            read(&failed(65535, FailCode::TooLong), None).await.unwrap(),
            WorkerFrame::Failed { slot: 65535, code: FailCode::TooLong }
        );
    }

    #[tokio::test]
    async fn front_frames_round_trip() {
        let speed = Speed::from_hundredths(150).unwrap();
        let bytes = encode_synthesize(&Synthesize {
            slot: 3,
            priority: Priority { level: 200, waiting: false },
            speed,
            voice: 1,
            ids: &[43, 16, 44],
            style_len: 7,
        });
        assert_eq!(bytes.len(), 4 + 13 + 3);
        assert_eq!(
            read_front(&mut &bytes[..]).await.unwrap(),
            FrontFrame::Synthesize {
                slot: 3,
                priority: 200,
                waiting: 0,
                speed: 150,
                voice: 1,
                ids: vec![43, 16, 44],
                style_len: 7
            }
        );
        assert_eq!(read_front(&mut &encode_cancel(4)[..]).await.unwrap(), FrontFrame::Cancel { slot: 4 });
    }

    #[tokio::test]
    async fn results_must_match_their_request() {
        let ok = |d: &[u16]| result(1, d, &vec![1; 600 * d.iter().map(|&x| usize::from(x)).sum::<usize>()]);
        assert!(read(&ok(&[1, 1, 1]), Some(expect(1))).await.is_ok());
        assert!(read(&ok(&[1, 1, 1]), None).await.is_err(), "free slot");
        assert!(read(&ok(&[1, 1, 1]), Some(expect(2))).await.is_err(), "too few durations");
        assert!(read(&ok(&[1, 0, 1]), Some(expect(1))).await.is_err(), "zero duration");
        assert!(read(&ok(&[1, 51, 1]), Some(expect(1))).await.is_err(), "duration over ceil(50 / speed)");
        assert!(read(&ok(&[1, 50, 1]), Some(expect(1))).await.is_ok());
        let slow = Expected { speed: Speed::MIN, ..expect(1) };
        assert!(read(&ok(&[1, 200, 1]), Some(slow)).await.is_ok());
        assert!(read(&ok(&[1, 201, 1]), Some(slow)).await.is_err());
        let thirds = Expected { speed: Speed::from_hundredths(30).unwrap(), ..expect(1) };
        assert!(read(&ok(&[1, 167, 1]), Some(thirds)).await.is_ok());
        assert!(read(&ok(&[1, 168, 1]), Some(thirds)).await.is_err());

        let short = result(1, &[1, 1, 1], &[1; 600 * 3 - 1]);
        assert!(read(&short, Some(expect(1))).await.is_err());
        let long = result(1, &[1, 1, 1], &[1; 600 * 3 + 1]);
        assert!(read(&long, Some(expect(1))).await.is_err());
        let capped = Expected { max_chunk_samples: 600 * 3 - 1, ..expect(1) };
        assert!(read(&ok(&[1, 1, 1]), Some(capped)).await.is_err(), "over the chunk cap");
    }

    #[tokio::test]
    async fn a_result_longer_than_its_slot_allows_is_refused_before_reading() {
        let mut huge = result(1, &[1, 1, 1], &[1; 1800]);
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(read(&huge, Some(expect(1))).await, Err(ReadError::Violation(_))));
    }

    #[tokio::test]
    async fn rejects_bad_hellos() {
        let cases: [fn(&mut Hello); 8] = [
            |h| h.voices.clear(),
            |h| h.voices.push("Bad".into()),
            |h| h.voices.push(String::new()),
            |h| h.voices.push("af_heart".into()),
            |h| h.vocab_size = 300,
            |h| h.max_phonemes = 600,
            |h| h.max_batch = 0,
            |h| h.slots = 0,
        ];
        for mutate in cases {
            let mut h = hello();
            mutate(&mut h);
            assert!(read(&worker_side::hello(&h), None).await.is_err(), "{h:?}");
        }
        let mut wrong_magic = worker_side::hello(&hello());
        wrong_magic[5] = b'X';
        assert!(read(&wrong_magic, None).await.is_err());
        let mut v1 = worker_side::hello(&hello());
        v1[9] = 1;
        assert!(read(&v1, None).await.is_err());
        let mut trailing = worker_side::hello(&hello());
        trailing.push(0);
        let len = trailing.len() as u32 - 4;
        trailing[0..4].copy_from_slice(&len.to_le_bytes());
        assert!(read(&trailing, None).await.is_err());
    }

    #[tokio::test]
    async fn oversized_lengths_are_refused_before_reading() {
        let mut huge = failed(1, FailCode::Internal);
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(read(&huge, None).await, Err(ReadError::Violation(_))));
        assert!(matches!(read(&[0, 0, 0, 0], None).await, Err(ReadError::Violation(_))));
        assert!(matches!(read(&[1, 0, 0, 0, 0x7f], None).await, Err(ReadError::Violation(_))));
    }

    #[test]
    fn priorities_order_by_level_then_waiting() {
        let p = |level, waiting| Priority { level, waiting };
        let mut order = [p(0, true), p(255, false), p(128, false), p(128, true), p(127, true), p(255, true)];
        order.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(order, [p(255, true), p(255, false), p(128, true), p(128, false), p(127, true), p(0, true)]);
    }

    #[test]
    fn speed_bounds_are_exact_integers() {
        assert_eq!(Speed::from_f64(1.234), Speed::from_hundredths(123));
        assert_eq!(Speed::from_f64(0.245), None);
        assert_eq!(Speed::from_f64(0.25), Some(Speed::MIN));
        assert_eq!(Speed::from_f64(4.0), Some(Speed::MAX));
        assert_eq!(Speed::from_f64(4.004), None);
        assert_eq!(Speed::from_f64(0.254), Some(Speed::MIN));
        assert_eq!(Speed::from_f64(f64::NAN), None);
        assert_eq!(Speed::from_hundredths(24), None);
        assert_eq!(Speed::from_hundredths(401), None);
        assert_eq!(Speed::MIN.max_duration(), 200);
        assert_eq!(Speed::NORMAL.max_duration(), 50);
        assert_eq!(Speed::MAX.max_duration(), 13);
        assert_eq!(Speed::from_hundredths(30).unwrap().max_duration(), 167);
        assert_eq!(Speed::MIN.chunk_cap(2_880_000), 11_520_000);
        assert_eq!(Speed::from_hundredths(30).unwrap().chunk_cap(1), 4);
    }
}
