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
pub use tr_languages::alphabet_language;

/// The espeak-ng 1.52 data the reference pipeline reads (bundled with espeakng-loader 0.2.4), cut
/// down to the phoneme tables, the voice files and the dictionaries misaki and kokoro use.
pub fn default_data_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("espeak-ng-data")
}

use engine::Tr;
use std::sync::Arc;

/// `espeakPHONEMES_IPA`.
pub const PHONEMES_IPA: i32 = 0x02;
/// `espeakPHONEMES_TIE`: bits 8-23 hold the tie character.
pub const PHONEMES_TIE: i32 = 0x80;
/// The mode phonemizer uses: IPA, with U+0361 tying multi-letter phonemes.
pub const PHONEMES_IPA_TIE: i32 = PHONEMES_IPA | PHONEMES_TIE | (0x361 << 8);

impl Espeak {
    /// `espeak_SetVoiceByName` on a freshly initialised library: `name` is a voice file name
    /// (e.g. `gmw/en-US`) or voice name.
    pub fn set_voice_by_name(&mut self, name: &str) -> Result<()> {
        *self = Espeak::new(self.data.clone());
        self.set_voice_by_name_c(name)?;
        debug_assert!(self.buffers_are_default());
        let mut snapshot = self.clone();
        snapshot.phoneme_list = Vec::new();
        snapshot.ph_list2 = Vec::new();
        snapshot.ph_list3 = Vec::new();
        snapshot.clause_sbuf = Vec::new();
        snapshot.clause_words = Vec::new();
        snapshot.clause_charix = Vec::new();
        snapshot.source = Vec::new();
        self.after_set_voice = Some(Arc::new(snapshot));
        Ok(())
    }

    /// Puts every field back to its value in `after_set_voice`. The large buffers, which are all
    /// default there, are cleared in place instead of copied.
    fn reset(&mut self, after_set_voice: Arc<Espeak>) {
        let mut phoneme_list = std::mem::take(&mut self.phoneme_list);
        let mut ph_list2 = std::mem::take(&mut self.ph_list2);
        let mut ph_list3 = std::mem::take(&mut self.ph_list3);
        let mut sbuf = std::mem::take(&mut self.clause_sbuf);
        let mut words = std::mem::take(&mut self.clause_words);
        let mut charix = std::mem::take(&mut self.clause_charix);
        let mut source = std::mem::take(&mut self.source);
        self.clone_from(&after_set_voice);
        self.after_set_voice = Some(after_set_voice);
        phoneme_list.fill(Default::default());
        ph_list2.fill(Default::default());
        ph_list3.fill(Default::default());
        sbuf.fill(0);
        words.fill(Default::default());
        charix.fill(0);
        source.fill(0);
        self.phoneme_list = phoneme_list;
        self.ph_list2 = ph_list2;
        self.ph_list3 = ph_list3;
        self.clause_sbuf = sbuf;
        self.clause_words = words;
        self.clause_charix = charix;
        self.source = source;
    }

    fn buffers_are_default(&self) -> bool {
        self.phoneme_list.iter().all(|p| *p == Default::default())
            && self.ph_list2.iter().all(|p| *p == Default::default())
            && self.ph_list3.iter().all(|p| *p == Default::default())
            && self.clause_sbuf.iter().all(|&b| b == 0)
            && self.clause_words.iter().all(|w| *w == Default::default())
            && self.clause_charix.iter().all(|&c| c == 0)
            && self.source.iter().all(|&b| b == 0)
    }

    /// Calls `espeak_TextToPhonemes` until the text is used up, as phonemizer does, returning
    /// each clause's phonemes.
    ///
    /// Each call starts from the state a new library has after `espeak_SetVoiceByName`. C keeps
    /// its globals from one call to the next, so in C (and phonemizer) an embedded command such
    /// as `\x01B` or a character read ahead at the end of one text changes later calls; here
    /// it can't (docs/divergences.md).
    ///
    /// C decodes the remaining text again for each clause. The UTF-8 decoder carries no state
    /// between characters, so decoding once and continuing from where the last clause stopped
    /// gives the same characters, in time linear in the length of the text.
    pub fn text_to_phonemes(&mut self, text: &str, phoneme_mode: i32) -> Vec<String> {
        self.translate_text(text, phoneme_mode, |_| {})
    }

    /// [`text_to_phonemes`](Self::text_to_phonemes), calling `after_clause` after each clause,
    /// e.g. to print [`debug_phoneme_list`](Self::debug_phoneme_list).
    #[doc(hidden)]
    pub fn translate_text(&mut self, text: &str, phoneme_mode: i32, mut after_clause: impl FnMut(&Self)) -> Vec<String> {
        let mut out = Vec::new();
        if self.used {
            let Some(after_set_voice) = self.after_set_voice.clone() else { return out };
            self.reset(after_set_voice);
        }
        self.used = true;
        if !self.voice_loaded || self.translator.is_none() {
            return out;
        }
        self.decoder.decode_string(text.as_bytes());
        loop {
            self.translate_clause(Tr::Main);
            after_clause(self);
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
