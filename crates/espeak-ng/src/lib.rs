//! A port of espeak-ng 1.52.0's text-to-phoneme engine (`espeak_TextToPhonemes`), reading
//! espeak-ng's compiled data (`phontab`, `phonindex`, `*_dict`, `lang/`) unchanged.
//!
//! The modules follow the C files they port, and functions keep their C names in snake_case,
//! so they can be compared with C side by side. C's globals live in [`Espeak`]; the data shared
//! by all instances lives in [`EspeakData`].
//!
//! ```no_run
//! use kokoro_espeak_ng::{Espeak, EspeakData, PHONEMES_IPA_TIE};
//! let data = EspeakData::load("espeak-ng-data").unwrap();
//! let mut espeak = Espeak::new(data);
//! espeak.set_voice_by_name("gmw/en-US").unwrap();
//! let clauses = espeak.text_to_phonemes("Hello world.", PHONEMES_IPA_TIE);
//! ```

#![forbid(unsafe_code)]
// The port keeps C's control flow (nested and empty branches, index loops, long parameter lists)
// so that each function can be read side by side with its C original.
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_else_if,
    clippy::collapsible_match,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

mod common;
mod data;
mod defs;
mod dictionary;
mod encoding;
mod engine;
mod numbers;
mod phoneme;
mod phonemelist;
mod readclause;
mod tr_languages;
mod translate;
mod translateword;
mod ucd;
mod ucd_tables;
mod voices;

pub use data::{EspeakData, Error, Result, VoiceInfo};
pub use engine::Espeak;

/// The espeak-ng 1.52 data the reference pipeline reads (bundled with espeakng-loader 0.2.4), cut
/// down to the phoneme tables, the voice files and the dictionaries misaki and kokoro use.
pub fn default_data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("espeak-ng-data")
}

use engine::Tr;

/// `espeakPHONEMES_IPA`.
pub const PHONEMES_IPA: i32 = 0x02;
/// `espeakPHONEMES_TIE`: bits 8-23 hold the tie character.
pub const PHONEMES_TIE: i32 = 0x80;
/// The mode phonemizer uses: IPA, with U+0361 tying multi-letter phonemes.
pub const PHONEMES_IPA_TIE: i32 = PHONEMES_IPA | PHONEMES_TIE | (0x361 << 8);

impl Espeak {
    /// One call of `espeak_TextToPhonemes`: decodes `text` (a UTF-8 C string; it ends at its
    /// first NUL), translates its first clause, and returns the clause's phonemes together with
    /// the text not yet read (`None` at the end, like C's `*textptr`).
    pub fn text_to_phonemes_clause<'t>(&mut self, text: &'t [u8], phoneme_mode: i32) -> Option<(String, Option<&'t [u8]>)> {
        if !self.voice_loaded || self.translator.is_none() {
            return None;
        }
        let text = &text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())];
        self.decoder.decode_string(text);
        self.translate_clause(Tr::Main);
        let remaining = self.decoder.remaining().map(|r| r.len());
        let phonemes = self.get_translated_phoneme_string(phoneme_mode);
        let rest = remaining.map(|n| &text[text.len() - n..]);
        Some((String::from_utf8_lossy(&phonemes).into_owned(), rest))
    }

    /// Calls `espeak_TextToPhonemes` until the text is used up, as phonemizer does, returning
    /// each clause's phonemes.
    ///
    /// C decodes the remaining text again on each call. The UTF-8 decoder carries no state
    /// between characters, so decoding once and continuing from where the last clause stopped
    /// gives the same characters, in time linear in the length of the text.
    pub fn text_to_phonemes(&mut self, text: &str, phoneme_mode: i32) -> Vec<String> {
        let mut out = Vec::new();
        if !self.voice_loaded || self.translator.is_none() {
            return out;
        }
        self.decoder.decode_string(text.as_bytes());
        loop {
            self.translate_clause(Tr::Main);
            let done = self.decoder.remaining().is_none();
            out.push(String::from_utf8_lossy(&self.get_translated_phoneme_string(phoneme_mode)).into_owned());
            if done {
                return out;
            }
        }
    }

    /// The last clause's `phoneme_list`, one entry per line, for comparing with C while debugging.
    #[doc(hidden)]
    pub fn debug_phoneme_list(&self) -> String {
        let mut s = String::new();
        for (ix, p) in self.phoneme_list.iter().take(self.n_phoneme_list.max(0) as usize).enumerate() {
            let m = p.ph.map_or(0, |ph| ph.mnemonic).to_le_bytes();
            let name: String = m.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
            s.push_str(&format!(
                "  [{ix}] code={} ph={name} phcode={} type={} stress={} wstress={} flags={:x} src={} new={} tone={}\n",
                p.ph.map_or(-1, |ph| ph.code as i32),
                p.phcode,
                p.typ,
                p.stresslevel,
                p.wordstress,
                p.synthflags,
                p.sourceix,
                p.newword,
                p.tone_ph
            ));
        }
        s
    }
}
