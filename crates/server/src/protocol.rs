//! Worker protocol v1 frames (`docs/worker-protocol.md`): length-prefixed, little-endian.
//!
//! Synthesize is `u8 type, u64 request_id, u8 priority, f32 speed, u8 n_voices,
//! n_voices × (u16 voice_index, f32 weight), u16 n_ids, u16 style_len, n_ids × u8 id`. The style
//! vector is row `style_len - 1` of the mixed voice pack (`1..=max_phonemes`): `n_ids` for a
//! normal chunk, and the whole clause's length (capped at 510) for each piece of a stitched one.
//!
//! What the front accepts from the worker, beyond the frame layouts:
//! - Hello comes first and only once, with version 1, `2 <= vocab_size <= 256`,
//!   `max_phonemes == 510`, `max_batch >= 1`, `sample_rate == 24000`, and 1 or more distinct voice
//!   names of 1 to 64 bytes of `[a-z0-9_]`.
//! - Every other frame names a request the front sent and that hasn't ended.
//! - Durations: at most one per request, before its Audio, with exactly `n_ids + 2` values, each
//!   at most `ceil(50 / speed) + 1` (Kokoro's `max_dur` is 50 sigmoid bins, divided by speed).
//! - Audio: after the request's Durations, with exactly `600 × Σ durations` samples (Kokoro v1.0
//!   renders 600 samples per duration unit; measured exact on the real model).
//! - Failed: code 1 (cancelled) only after the front sent Cancel for that request, or 2.
//! - Each length prefix is checked against its frame type's bound before anything is allocated:
//!   Hello at most 19 + 65,535 × 65 bytes, Durations at most 1,035, Failed exactly 10, and Audio
//!   exactly the size its Durations imply, so the reader asks the caller how many samples to expect.
//!
//! Anything else closes the connection, failing every request in flight on it.

use std::fmt;
use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

pub const MAGIC: [u8; 4] = *b"KOKW";
pub const VERSION: u16 = 1;
pub const MAX_PHONEMES: u16 = 510;
pub const SAMPLE_RATE: u32 = 24_000;
/// Kokoro v1.0 renders 600 samples per predicted duration unit, so an Audio frame has exactly
/// `600 * sum(durations)` samples.
pub const SAMPLES_PER_FRAME: u64 = 600;
pub const MAX_FRONT_BODY: usize = 64 * 1024;
/// Kokoro v1.0's `max_dur`: an id's duration is a sum of this many sigmoids, divided by speed.
const DURATION_BINS: f64 = 50.0;
pub const MAX_VOICES_PER_MIX: usize = 8;
pub const MAX_VOICE_NAME: usize = 64;
pub const SPEED_MIN: f32 = 0.25;
pub const SPEED_MAX: f32 = 4.0;

const TYPE_HELLO: u8 = 0x01;
const TYPE_AUDIO: u8 = 0x02;
const TYPE_DURATIONS: u8 = 0x03;
const TYPE_FAILED: u8 = 0x04;
const TYPE_SYNTHESIZE: u8 = 0x10;
const TYPE_CANCEL: u8 = 0x11;

/// Fixed part of Hello: type, magic, version, vocab_size, max_phonemes, max_batch, sample_rate, n_voices.
const HELLO_FIXED: usize = 1 + 4 + 2 + 2 + 2 + 2 + 4 + 2;
const HELLO_MAX: usize = HELLO_FIXED + u16::MAX as usize * (1 + MAX_VOICE_NAME);
const DURATIONS_MAX: usize = 1 + 8 + 2 + 2 * (MAX_PHONEMES as usize + 2);
const FAILED_LEN: usize = 1 + 8 + 1;
const AUDIO_HEADER: usize = 1 + 8 + 4;

/// The longest duration Kokoro can predict for one id at `speed`, plus one for float rounding.
/// It bounds how much audio the front accepts for a request.
pub fn max_duration(speed: f32) -> u16 {
    ((DURATION_BINS / f64::from(speed)).ceil() + 1.0) as u16
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub vocab_size: u16,
    pub max_phonemes: u16,
    pub max_batch: u16,
    pub voices: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailCode {
    Cancelled,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerFrame {
    Hello(Hello),
    Audio { id: u64, pcm: Vec<i16> },
    Durations { id: u64, durations: Vec<u16> },
    Failed { id: u64, code: FailCode },
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
    fn u64(&mut self) -> Result<u64, ReadError> {
        self.take().map(u64::from_le_bytes)
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
    let sample_rate = c.u32()?;
    if sample_rate != SAMPLE_RATE {
        return violation(format!("sample_rate {sample_rate}, expected {SAMPLE_RATE}"));
    }
    let n_voices = c.u16()?;
    if n_voices == 0 {
        return violation("no voices");
    }
    let mut voices = Vec::with_capacity(n_voices.into());
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
        if voices.contains(&name) {
            return violation(format!("voice {name} listed twice"));
        }
        voices.push(name);
    }
    c.finish()?;
    Ok(Hello { vocab_size, max_phonemes, max_batch, voices })
}

/// Reads one frame. `expected_samples(id)` gives the sample count an Audio frame for `id` must
/// have (from its Durations), or `None` if no Audio is expected for it.
pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    expected_samples: impl FnOnce(u64) -> Option<u64>,
) -> Result<WorkerFrame, ReadError> {
    let len = reader.read_u32_le().await? as usize;
    if len == 0 {
        return violation("empty frame");
    }
    let kind = reader.read_u8().await?;
    let max = match kind {
        TYPE_HELLO => HELLO_MAX,
        TYPE_DURATIONS => DURATIONS_MAX,
        TYPE_FAILED => FAILED_LEN,
        TYPE_AUDIO => return read_audio(reader, len, expected_samples).await,
        _ => return violation(format!("unknown frame type {kind:#04x}")),
    };
    if len > max {
        return violation(format!("frame type {kind:#04x} is {len} bytes, more than {max}"));
    }
    let mut body = vec![0; len];
    body[0] = kind;
    reader.read_exact(&mut body[1..]).await?;
    parse_body(&body)
}

async fn read_audio<R: AsyncRead + Unpin>(
    reader: &mut R,
    len: usize,
    expected_samples: impl FnOnce(u64) -> Option<u64>,
) -> Result<WorkerFrame, ReadError> {
    if len < AUDIO_HEADER {
        return violation("audio frame too short");
    }
    let id = reader.read_u64_le().await?;
    let n = reader.read_u32_le().await?;
    let Some(expected) = expected_samples(id) else {
        return violation(format!("audio for request {id}, which expects none"));
    };
    if u64::from(n) != expected {
        return violation(format!("request {id}: {n} samples, durations say {expected}"));
    }
    if len != AUDIO_HEADER + 2 * n as usize {
        return violation("audio frame length doesn't match its sample count");
    }
    let mut raw = vec![0; 2 * n as usize];
    reader.read_exact(&mut raw).await?;
    let pcm = raw.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
    Ok(WorkerFrame::Audio { id, pcm })
}

/// Parses a whole frame body other than Audio, which [`read_frame`] reads incrementally.
fn parse_body(body: &[u8]) -> Result<WorkerFrame, ReadError> {
    match body[0] {
        TYPE_HELLO => parse_hello(body).map(WorkerFrame::Hello),
        TYPE_DURATIONS => {
            let mut c = Cursor(body);
            c.u8()?;
            let id = c.u64()?;
            let n = c.u16()?;
            let durations = (0..n).map(|_| c.u16()).collect::<Result<_, _>>()?;
            c.finish()?;
            Ok(WorkerFrame::Durations { id, durations })
        }
        TYPE_FAILED => {
            let mut c = Cursor(body);
            c.u8()?;
            let id = c.u64()?;
            let code = match c.u8()? {
                1 => FailCode::Cancelled,
                2 => FailCode::Internal,
                other => return violation(format!("unknown failure code {other}")),
            };
            c.finish()?;
            Ok(WorkerFrame::Failed { id, code })
        }
        _ => unreachable!("read_frame dispatches on the type"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Waiting = 0,
    Prefetch = 1,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Synthesize<'a> {
    pub id: u64,
    pub priority: Priority,
    pub speed: f32,
    pub voices: &'a [(u16, f32)],
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
    let mut body = Vec::with_capacity(1 + 8 + 1 + 4 + 1 + 6 * s.voices.len() + 4 + s.ids.len());
    body.push(TYPE_SYNTHESIZE);
    body.extend_from_slice(&s.id.to_le_bytes());
    body.push(s.priority as u8);
    body.extend_from_slice(&s.speed.to_le_bytes());
    body.push(s.voices.len() as u8);
    for &(index, weight) in s.voices {
        body.extend_from_slice(&index.to_le_bytes());
        body.extend_from_slice(&weight.to_le_bytes());
    }
    body.extend_from_slice(&(s.ids.len() as u16).to_le_bytes());
    body.extend_from_slice(&s.style_len.to_le_bytes());
    body.extend_from_slice(s.ids);
    debug_assert!(body.len() <= MAX_FRONT_BODY);
    frame(body)
}

pub fn encode_cancel(id: u64) -> Vec<u8> {
    let mut body = Vec::with_capacity(9);
    body.push(TYPE_CANCEL);
    body.extend_from_slice(&id.to_le_bytes());
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
        body.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        body.extend_from_slice(&(h.voices.len() as u16).to_le_bytes());
        for v in &h.voices {
            body.push(v.len() as u8);
            body.extend_from_slice(v.as_bytes());
        }
        frame(body)
    }

    pub fn audio(id: u64, pcm: &[i16]) -> Vec<u8> {
        let mut body = vec![TYPE_AUDIO];
        body.extend_from_slice(&id.to_le_bytes());
        body.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        for s in pcm {
            body.extend_from_slice(&s.to_le_bytes());
        }
        frame(body)
    }

    pub fn durations(id: u64, durations: &[u16]) -> Vec<u8> {
        let mut body = vec![TYPE_DURATIONS];
        body.extend_from_slice(&id.to_le_bytes());
        body.extend_from_slice(&(durations.len() as u16).to_le_bytes());
        for d in durations {
            body.extend_from_slice(&d.to_le_bytes());
        }
        frame(body)
    }

    pub fn failed(id: u64, code: FailCode) -> Vec<u8> {
        let mut body = vec![TYPE_FAILED];
        body.extend_from_slice(&id.to_le_bytes());
        body.push(match code {
            FailCode::Cancelled => 1,
            FailCode::Internal => 2,
        });
        frame(body)
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum FrontFrame {
        Synthesize { id: u64, priority: u8, speed: f32, voices: Vec<(u16, f32)>, ids: Vec<u8>, style_len: u16 },
        Cancel { id: u64 },
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
                let id = c.u64()?;
                let priority = c.u8()?;
                let speed = f32::from_le_bytes(c.take()?);
                let n_voices = c.u8()?;
                let voices = (0..n_voices)
                    .map(|_| Ok((c.u16()?, f32::from_le_bytes(c.take()?))))
                    .collect::<Result<_, ReadError>>()?;
                let n_ids = c.u16()?;
                let style_len = c.u16()?;
                let ids = c.bytes(n_ids.into())?.to_vec();
                FrontFrame::Synthesize { id, priority, speed, voices, ids, style_len }
            }
            TYPE_CANCEL => FrontFrame::Cancel { id: c.u64()? },
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
        Hello { vocab_size: 178, max_phonemes: 510, max_batch: 8, voices: vec!["af_heart".into(), "am_adam".into()] }
    }

    async fn read(bytes: &[u8], expect: Option<u64>) -> Result<WorkerFrame, ReadError> {
        read_frame(&mut &bytes[..], |_| expect).await
    }

    #[tokio::test]
    async fn worker_frames_round_trip() {
        assert_eq!(read(&worker_side::hello(&hello()), None).await.unwrap(), WorkerFrame::Hello(hello()));
        assert_eq!(
            read(&durations(7, &[1, 2, 3]), None).await.unwrap(),
            WorkerFrame::Durations { id: 7, durations: vec![1, 2, 3] }
        );
        assert_eq!(
            read(&audio(7, &[1, -2, 3]), Some(3)).await.unwrap(),
            WorkerFrame::Audio { id: 7, pcm: vec![1, -2, 3] }
        );
        assert_eq!(
            read(&failed(9, FailCode::Cancelled), None).await.unwrap(),
            WorkerFrame::Failed { id: 9, code: FailCode::Cancelled }
        );
    }

    #[tokio::test]
    async fn front_frames_round_trip() {
        let voices = [(1, 0.5), (0, 2.0)];
        let bytes = encode_synthesize(&Synthesize {
            id: 3,
            priority: Priority::Prefetch,
            speed: 1.5,
            voices: &voices,
            ids: &[43, 16, 44],
            style_len: 7,
        });
        assert_eq!(
            read_front(&mut &bytes[..]).await.unwrap(),
            FrontFrame::Synthesize {
                id: 3,
                priority: 1,
                speed: 1.5,
                voices: voices.to_vec(),
                ids: vec![43, 16, 44],
                style_len: 7
            }
        );
        assert_eq!(read_front(&mut &encode_cancel(4)[..]).await.unwrap(), FrontFrame::Cancel { id: 4 });
    }

    #[tokio::test]
    async fn audio_must_match_its_durations() {
        assert!(read(&audio(7, &[1, 2]), Some(3)).await.is_err());
        assert!(read(&audio(7, &[1, 2]), None).await.is_err());
        let mut lying = audio(7, &[1, 2]);
        lying[0..4].copy_from_slice(&1000u32.to_le_bytes());
        assert!(read(&lying, Some(2)).await.is_err());
    }

    #[tokio::test]
    async fn rejects_bad_hellos() {
        let cases: [fn(&mut Hello); 7] = [
            |h| h.voices.clear(),
            |h| h.voices.push("Bad".into()),
            |h| h.voices.push(String::new()),
            |h| h.voices.push("af_heart".into()),
            |h| h.vocab_size = 300,
            |h| h.max_phonemes = 600,
            |h| h.max_batch = 0,
        ];
        for mutate in cases {
            let mut h = hello();
            mutate(&mut h);
            assert!(read(&worker_side::hello(&h), None).await.is_err(), "{h:?}");
        }
        let mut wrong_magic = worker_side::hello(&hello());
        wrong_magic[5] = b'X';
        assert!(read(&wrong_magic, None).await.is_err());
        let mut trailing = worker_side::hello(&hello());
        trailing.push(0);
        let len = trailing.len() as u32 - 4;
        trailing[0..4].copy_from_slice(&len.to_le_bytes());
        assert!(read(&trailing, None).await.is_err());
    }

    #[tokio::test]
    async fn oversized_lengths_are_refused_before_reading() {
        let mut huge = durations(1, &[1]);
        huge[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(read(&huge, None).await, Err(ReadError::Violation(_))));
        assert!(matches!(read(&[0, 0, 0, 0], None).await, Err(ReadError::Violation(_))));
        assert!(matches!(read(&[1, 0, 0, 0, 0x7f], None).await, Err(ReadError::Violation(_))));
    }
}
