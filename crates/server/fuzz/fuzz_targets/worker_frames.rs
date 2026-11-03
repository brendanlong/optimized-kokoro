#![no_main]
#![forbid(unsafe_code)]

use kokoro_server::protocol::{Expected, Speed, read_frame};
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_current_thread().build().unwrap());

const CAPS: [u32; 4] = [2_880_000, 600 * 40, 1, u32::MAX];

// The first two bytes pick the speed and chunk cap of the busy slots, 0 to 3, which hold
// requests of 1 to 4 ids; the rest is the stream.
fuzz_target!(|data: &[u8]| {
    let Some((&[speed, cap], mut stream)) = data.split_first_chunk::<2>() else { return };
    let speed = Speed::from_hundredths(25 + u16::from(speed) % 376).unwrap();
    let cap = CAPS[usize::from(cap) % CAPS.len()];
    let busy = |slot: u16| (slot < 4).then(|| Expected { n_ids: slot + 1, speed, max_chunk_samples: cap });
    RUNTIME.block_on(async {
        while !stream.is_empty() {
            let before = stream.len();
            if read_frame(&mut stream, busy).await.is_err() {
                break;
            }
            assert!(stream.len() < before);
        }
    });
});
