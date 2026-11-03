#![no_main]

use kokoro_server::protocol::read_frame;
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

static RUNTIME: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Builder::new_current_thread().build().unwrap());

// The first byte picks the Audio length the reader is told to expect; the rest is the stream.
fuzz_target!(|data: &[u8]| {
    let Some((&pick, mut stream)) = data.split_first() else { return };
    let expected = [None, Some(0), Some(600), Some(600 * 9), Some(u64::MAX)][pick as usize % 5];
    RUNTIME.block_on(async {
        while !stream.is_empty() {
            let before = stream.len();
            if read_frame(&mut stream, |_| expected).await.is_err() {
                break;
            }
            assert!(stream.len() < before);
        }
    });
});
