//! Random and mutated worker frames: the decoder and the connection must reject or accept them
//! without panicking, hanging or over-allocating. `fuzz/` has a coverage-guided target for longer
//! runs (`cargo +nightly fuzz run worker_frames`).

mod common;

use common::*;
use kokoro_server::StubPipeline;
use kokoro_server::protocol::worker_side;
use kokoro_server::protocol::{FailCode, read_frame};
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn valid_frames(rng: &mut Rng) -> Vec<u8> {
    let mut out = worker_side::hello(&hello());
    for _ in 0..rng.below(4) {
        let id = rng.below(4) as u64;
        let ids: Vec<u8> = (0..1 + rng.below(5)).map(|_| 1 + rng.below(177) as u8).collect();
        match rng.below(3) {
            0 => out.extend(respond(id, &ids)),
            1 => out.extend(worker_side::failed(id, if rng.below(2) == 0 { FailCode::Cancelled } else { FailCode::Internal })),
            _ => out.extend(worker_side::durations(id, &render(&ids).0)),
        }
    }
    out
}

fn mutate(rng: &mut Rng, bytes: &mut Vec<u8>) {
    for _ in 0..1 + rng.below(4) {
        if bytes.is_empty() {
            bytes.push(rng.next() as u8);
            continue;
        }
        let at = rng.below(bytes.len());
        match rng.below(6) {
            0 => bytes[at] ^= 1 << rng.below(8),
            1 => bytes[at] = rng.next() as u8,
            2 => bytes.insert(at, rng.next() as u8),
            3 => {
                bytes.remove(at);
            }
            4 => bytes.truncate(at),
            _ => {
                let interesting = [0u32, 1, 2, 9, 10, 13, 0xffff, 0x10000, 0x7fff_ffff, u32::MAX];
                let v = interesting[rng.below(interesting.len())].to_le_bytes();
                for (i, b) in v.iter().enumerate() {
                    if let Some(slot) = bytes.get_mut(at + i) {
                        *slot = *b;
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn decoder_survives_mutated_frames() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut accepted = 0;
    for _ in 0..100_000 {
        let mut bytes = valid_frames(&mut rng);
        mutate(&mut rng, &mut bytes);
        let mut reader = &bytes[..];
        let expect = [None, Some(0), Some(600), Some(600 * 7), Some(u64::MAX)][rng.below(5)];
        while !reader.is_empty() {
            let before = reader.len();
            match read_frame(&mut reader, |_| expect).await {
                Ok(_) => accepted += 1,
                Err(_) => break,
            }
            assert!(reader.len() < before);
        }
    }
    assert!(accepted > 10_000, "mutations too aggressive to exercise parsing: {accepted}");
}

#[tokio::test]
async fn decoder_survives_random_bytes() {
    let mut rng = Rng(42);
    for _ in 0..100_000 {
        let len = rng.below(64);
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let mut reader = &bytes[..];
        while !reader.is_empty() {
            if read_frame(&mut reader, |_| Some(rng.next() % 64)).await.is_err() {
                break;
            }
        }
    }
}

/// A worker that answers each request with a mutated reply and hangs up: the server must fail
/// or finish the request, and reconnect, every time.
#[tokio::test]
async fn connection_survives_mutated_replies() {
    let server = start(|_| {}, Arc::new(StubPipeline)).await;
    let mut worker = ScriptedWorker::listen(&server.socket, worker_side::hello(&hello()));
    let mut rng = Rng(7);
    let mut outcomes = std::collections::BTreeMap::new();
    for _ in 0..200 {
        while worker.next().await != Event::Connected {}
        server.worker.connected().await;
        let client = tokio::spawn(speak(server.addr, pcm("Hi.")));
        let (id, _, _, _, ids) = worker.synthesize().await;
        let mut reply = respond(id, &ids);
        if rng.below(10) != 0 {
            mutate(&mut rng, &mut reply);
        }
        worker.send(reply);
        worker.hang_up();
        let response = client.await.unwrap();
        let status = response.status().as_u16();
        let complete = body_bytes(response).await.is_ok();
        *outcomes.entry((status, complete)).or_insert(0) += 1;
        assert!(matches!(status, 200 | 500 | 503), "{status}");
    }
    assert!(outcomes.keys().any(|&(s, _)| s == 200) && outcomes.keys().any(|&(s, _)| s == 503), "{outcomes:?}");
}
