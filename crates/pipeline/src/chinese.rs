//! Chinese: misaki's legacy Chinese G2P ([`kokoro_g2p_zh`]) as a [`ForeignG2p`], for Han runs in
//! the other pipelines and the Chinese pipeline's (`lang` z) own text.

use crate::ForeignG2p;
use kokoro_g2p_zh::ZhG2p;
use std::sync::OnceLock;

static G2P: OnceLock<ZhG2p> = OnceLock::new();

/// The dictionaries (about 45 MB, 80–95 ms to load), loaded on the first Han character or Chinese
/// request and shared by every pipeline, so English-only text never pays for them (Brendan's
/// decision, 2026-11-03).
fn g2p() -> &'static ZhG2p {
    G2P.get_or_init(ZhG2p::new)
}

/// Whether the Chinese G2P has been loaded in this process.
pub fn loaded() -> bool {
    G2P.get().is_some()
}

/// `ZHG2P(version=None)` for runs of Han characters.
pub struct Chinese;

impl ForeignG2p for Chinese {
    fn phonemize(&self, text: &str) -> Result<String, String> {
        g2p().phonemize(text).map_err(|e| e.to_string())
    }

    fn reads(&self, c: char) -> bool {
        g2p().reads(c)
    }
}
