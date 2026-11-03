//! The text side of Kokoro-FastAPI (Apache-2.0, at the commit `reference/setup.sh` pins) before
//! G2P, ported to match the deployed server exactly: inline tags, SSML, line joining, English
//! normalization, sentence splitting and chunking. `reference/` holds the parity fixtures.
//!
//! A request's text goes through, in order: [`ssml::translate`] (when the request asks for it),
//! [`tags::split_by_voice`] (when voice tags are allowed), and [`split::smart_split`] per segment,
//! which handles pause tags, emoji, [`split::join_lines`], [`normalize`], [`split::sentences`] and
//! chunking with the caller's G2P.

mod english_data;
pub mod inflect;
pub mod normalize;
pub mod py;
pub mod pyre;
pub mod split;
pub mod ssml;
mod tables;
pub mod tags;

pub use normalize::{Normalizer, Options};
pub use split::{Chunk, DEFAULT_MAX_TOKENS, Phonemes, Piece, SmartSplit, join_lines, sentences, smart_split};
pub use tags::{Limits, Rejected, VoiceSegment, check_pause_budget, check_speakable, split_by_voice, strip_emoji};
