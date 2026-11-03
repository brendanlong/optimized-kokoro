//! The C library's global state, gathered into one struct. One [`Espeak`] is one independent
//! copy of the C library: use one per thread.

use crate::data::{EspeakData, PhonemeTab};
use crate::defs::*;
use crate::encoding::TextDecoder;
use std::sync::Arc;

/// Which of C's three translator globals (`translator`, `translator2`, `translator3`) a
/// `Translator *` points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tr {
    Main,
    Second,
    Third,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ReplacePhonemes {
    pub old_ph: u8,
    pub new_ph: u8,
    pub typ: i8,
}

/// A text-to-phoneme engine for one voice: C espeak-ng's global state, over data shared with
/// other engines. Each [`text_to_phonemes`](Espeak::text_to_phonemes) call starts from the state
/// C has right after `espeak_SetVoiceByName`, so nothing one call reads (embedded commands,
/// look-ahead characters, buffer contents) can affect another.
#[derive(Clone)]
pub struct Espeak {
    pub(crate) data: Arc<EspeakData>,
    /// The state right after `set_voice_by_name`, copied back before each call. It leaves out
    /// the large buffers, which are all default at that point and are cleared instead.
    pub(crate) after_set_voice: Option<Arc<Espeak>>,
    /// The state has been used by a call since it was copied from `after_set_voice`.
    pub(crate) used: bool,

    // synthdata.c
    pub(crate) phoneme_tab: [Option<PhonemeTab>; N_PHONEME_TAB],
    pub(crate) n_phoneme_tab: i32,
    pub(crate) current_phoneme_table: i32,
    pub(crate) phoneme_tab_number: i32,

    // voices.c
    pub(crate) voice_phoneme_tab_ix: i32,
    pub(crate) voice_language_name: Vec<u8>,
    pub(crate) voice_loaded: bool,
    pub(crate) replace_phonemes: Vec<ReplacePhonemes>,
    pub(crate) dictionary_name: Vec<u8>,

    // translate.c
    pub(crate) translator: Option<Box<Translator>>,
    pub(crate) translator2: Option<Box<Translator>>,
    pub(crate) translator2_language: Vec<u8>,
    pub(crate) translator3: Option<Box<Translator>>,
    pub(crate) translator3_language: Vec<u8>,
    pub(crate) dummy_translator: Box<Translator>,

    pub(crate) option_tone_flags: i32,
    pub(crate) option_phonemes: i32,
    pub(crate) option_endpause: i32,
    pub(crate) option_capitals: i32,
    pub(crate) option_punctuation: i32,
    pub(crate) option_sayas: i32,
    pub(crate) option_sayas2: i32,
    pub(crate) option_emphasis: i32,
    pub(crate) option_phoneme_input: i32,
    pub(crate) option_wordgap: i32,
    pub(crate) option_linelength: i32,
    pub(crate) option_punctlist: [u32; N_PUNCTLIST],

    pub(crate) count_sayas_digits: i32,
    pub(crate) skip_sentences: i32,
    pub(crate) skip_words: i32,
    pub(crate) skip_characters: i32,
    pub(crate) skipping_text: bool,
    pub(crate) end_character_position: i32,
    pub(crate) count_sentences: i32,
    pub(crate) count_words: i32,
    pub(crate) clause_start_char: i32,
    pub(crate) clause_start_word: i32,
    pub(crate) new_sentence: bool,
    pub(crate) word_emphasis: u32,
    pub(crate) embedded_flag: u16,
    pub(crate) max_clause_pause: i32,
    pub(crate) any_stressed_words: bool,
    pub(crate) pre_pause: i32,

    pub(crate) word_phonemes: [u8; N_WORD_PHONEMES],
    pub(crate) n_ph_list2: i32,
    pub(crate) ph_list2: Vec<PhonemeList2>,

    pub(crate) embedded_ix: i32,
    pub(crate) embedded_read: i32,
    pub(crate) embedded_list: [u32; N_EMBEDDED_LIST],
    pub(crate) source: Vec<u8>,
    pub(crate) ignore_next_n: i32,

    // dictionary.c
    pub(crate) dictionary_skipwords: i32,
    pub(crate) word_replacement: [u8; N_WORD_BYTES],

    // numbers.c
    pub(crate) n_digit_lookup: i32,
    pub(crate) digit_lookup: Vec<u8>,
    pub(crate) speak_missing_thousands: i32,
    pub(crate) number_control: i32,
    /// `ph_ordinal2` and `ph_ordinal2x`, adjacent as in C.
    pub(crate) ph_ordinal: [u8; 24],

    // readclause.c
    pub(crate) decoder: TextDecoder,
    pub(crate) ungot_char: i32,
    pub(crate) ungot_char2: i32,
    pub(crate) ungot_string: [u8; 24],
    pub(crate) ungot_string_ix: i32,
    pub(crate) ignore_text: bool,
    pub(crate) audio_text: bool,
    pub(crate) clear_skipping_text: bool,
    pub(crate) count_characters: i32,
    pub(crate) sayas_mode: i32,

    // phonemelist.c / synthesize.c
    pub(crate) phoneme_list: Vec<PhonemeList>,
    pub(crate) n_phoneme_list: i32,
    pub(crate) ph_list3: Vec<PhonemeList>,

    // dictionary.c's phoneme string output buffer
    pub(crate) phon_out: Vec<u8>,

    /// TranslateClause's large stack arrays, kept to reuse their allocations. Each clause
    /// starts with them zeroed.
    pub(crate) clause_sbuf: Vec<u8>,
    pub(crate) clause_words: Vec<WordTab>,
    pub(crate) clause_charix: Vec<i16>,
}

pub(crate) const N_PUNCTLIST: usize = 60;
pub(crate) const N_EMBEDDED_LIST: usize = 250;

impl Espeak {
    /// `espeak_Initialize`: a fresh copy of the library's state, with no voice selected.
    pub fn new(data: Arc<EspeakData>) -> Self {
        let dummy = Box::new(crate::tr_languages::select_translator(b""));
        let mut e = Espeak {
            data,
            after_set_voice: None,
            used: false,
            phoneme_tab: [None; N_PHONEME_TAB],
            n_phoneme_tab: 0,
            current_phoneme_table: -1,
            phoneme_tab_number: 0,
            voice_phoneme_tab_ix: 0,
            voice_language_name: Vec::new(),
            voice_loaded: false,
            replace_phonemes: Vec::new(),
            dictionary_name: Vec::new(),
            translator: None,
            translator2: None,
            translator2_language: Vec::new(),
            translator3: None,
            translator3_language: Vec::new(),
            dummy_translator: dummy,
            option_tone_flags: 0,
            option_phonemes: 0,
            option_endpause: 0,
            option_capitals: 0,
            option_punctuation: 0,
            option_sayas: 0,
            option_sayas2: 0,
            option_emphasis: 0,
            option_phoneme_input: 0,
            option_wordgap: 0,
            option_linelength: 0,
            option_punctlist: [0; N_PUNCTLIST],
            count_sayas_digits: 0,
            skip_sentences: 0,
            skip_words: 0,
            skip_characters: 0,
            skipping_text: false,
            end_character_position: 0,
            count_sentences: 0,
            count_words: 0,
            clause_start_char: 0,
            clause_start_word: 0,
            new_sentence: false,
            word_emphasis: 0,
            embedded_flag: 0,
            max_clause_pause: 0,
            any_stressed_words: false,
            pre_pause: 0,
            word_phonemes: [0; N_WORD_PHONEMES],
            n_ph_list2: 0,
            ph_list2: vec![PhonemeList2::default(); N_PHONEME_LIST],
            embedded_ix: 0,
            embedded_read: 0,
            embedded_list: [0; N_EMBEDDED_LIST],
            source: vec![0; N_TR_SOURCE + 40],
            ignore_next_n: 0,
            dictionary_skipwords: 0,
            word_replacement: [0; N_WORD_BYTES],
            n_digit_lookup: 0,
            digit_lookup: Vec::new(),
            speak_missing_thousands: 0,
            number_control: 0,
            ph_ordinal: [0; 24],
            decoder: TextDecoder::default(),
            ungot_char: 0,
            ungot_char2: 0,
            ungot_string: [0; 24],
            ungot_string_ix: -1,
            ignore_text: false,
            audio_text: false,
            clear_skipping_text: false,
            count_characters: 0,
            sayas_mode: 0,
            phoneme_list: vec![PhonemeList::default(); N_PHONEME_LIST + 1],
            n_phoneme_list: 0,
            ph_list3: vec![PhonemeList::default(); N_PHONEME_LIST],
            phon_out: Vec::new(),
            clause_sbuf: vec![0; N_TR_SOURCE + 8],
            clause_words: vec![WordTab::default(); N_CLAUSE_WORDS],
            clause_charix: vec![0; N_TR_SOURCE + 4],
        };
        // LoadPhData leaves phoneme table 0 unselected; SetVoiceByName selects one.
        e.option_phonemes = 0;
        e
    }

    pub(crate) fn t(&self, t: Tr) -> &Translator {
        let tr = match t {
            Tr::Main => self.translator.as_deref(),
            Tr::Second => self.translator2.as_deref(),
            Tr::Third => self.translator3.as_deref(),
        };
        tr.unwrap_or(&self.dummy_translator)
    }

    pub(crate) fn tm(&mut self, t: Tr) -> &mut Translator {
        let tr = match t {
            Tr::Main => self.translator.as_deref_mut(),
            Tr::Second => self.translator2.as_deref_mut(),
            Tr::Third => self.translator3.as_deref_mut(),
        };
        match tr {
            Some(tr) => tr,
            None => &mut self.dummy_translator,
        }
    }

    /// `strcpy(word_phonemes, src)`: bytes after the terminator keep their old values, which C
    /// sometimes reads.
    pub(crate) fn set_word_phonemes(&mut self, src: &[u8]) {
        let s = crate::common::cstr(src, 0);
        crate::common::cstrcpy(&mut self.word_phonemes, 0, s);
    }

    pub(crate) fn exists(&self, t: Tr) -> bool {
        match t {
            Tr::Main => self.translator.is_some(),
            Tr::Second => self.translator2.is_some(),
            Tr::Third => self.translator3.is_some(),
        }
    }
}
