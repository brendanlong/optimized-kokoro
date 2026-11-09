//! The text side of Kokoro-FastAPI (Apache-2.0, at the commit `reference/setup.sh` pins) before
//! G2P, ported to match the deployed server exactly: pause tags, line joining, English
//! normalization, sentence splitting and chunking. `reference/` holds the parity fixtures, and
//! `docs/divergences.md` lists what differs on purpose (SSML and voice and rate tags aren't
//! supported).
//!
//! A request's text goes through [`split::smart_split`], which handles pause tags, emoji,
//! [`split::join_lines`], [`normalize`], [`split::sentences`] and chunking with the caller's G2P.
#![forbid(unsafe_code)]

mod abbreviations;
mod english_data;
mod footnotes;
pub mod normalize;
pub mod numbers;
pub mod py;
pub mod pyre;
mod roman;
mod scan;
pub mod split;
mod symbols;
mod tables;
pub mod tags;

pub use normalize::{Normalizer, Options};
pub use split::{
    Chunk, DEFAULT_MAX_TOKENS, MIN_CLAUSE_CHUNK, Phonemes, Piece, SmartSplit, Split, join_lines, sentences,
    smart_split, try_smart_split,
};
pub use tags::{Limits, Rejected, check_pause_budget, check_speakable, strip_emoji};
