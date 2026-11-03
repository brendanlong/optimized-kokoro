//! translate.c: splitting a clause into words and translating them into the first-stage
//! phoneme list.

use crate::common::*;
use crate::defs::*;
use crate::engine::{Espeak, Tr, N_EMBEDDED_LIST};
use crate::tr_languages::select_translator;
use crate::ucd;

const BREAKS: [u16; 2] = [b'_' as u16, 0];

const UCASE_GA: [&[u8]; 11] = [b"bp", b"bhf", b"dt", b"gc", b"hA", b"mb", b"nd", b"ng", b"ts", b"tA", b"nA"];

const HANGUL_COMPATIBILITY: [u8; 0x34] = [
    0, 0x00, 0x01, 0xaa, 0x02, 0xac, 0xad, 0x03, 0x04, 0x05, 0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb4, 0xb6, 0x06, 0x07, 0x08, 0xb9, 0x09, 0x0a, 0xbc, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x61,
    0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75,
];

fn wt(words: &[WordTab], ix: isize) -> WordTab {
    if ix < 0 { WordTab::default() } else { words.get(ix as usize).copied().unwrap_or_default() }
}

fn wt_mut(words: &mut [WordTab], ix: isize) -> Option<&mut WordTab> {
    if ix < 0 { None } else { words.get_mut(ix as usize) }
}

impl Espeak {
    /// `TranslateWord`: `TranslateWord3`, then re-translates replacement text word by word.
    pub(crate) fn translate_word(&mut self, tr: Tr, w: &mut [u8], word_start: isize, wtab: Option<&mut [WordTab]>, in_clause: bool, word_out: Option<&mut Vec<u8>>) -> u32 {
        let mut words_phonemes = [0u8; N_WORD_PHONEMES];
        let mut empty: [WordTab; 0] = [];
        let has_wtab = wtab.is_some();
        let wtab: &mut [WordTab] = match wtab {
            Some(w) => w,
            None => &mut empty,
        };
        let want_out = word_out.is_some();
        let mut out_text = Vec::new();
        let flags = self.translate_word3(tr, w, word_start, wtab, if want_out { Some(&mut out_text) } else { None }, in_clause);
        if (flags & FLAG_TEXTMODE) != 0 && want_out {
            if let Some(o) = word_out {
                o.clear();
                o.extend_from_slice(&out_text);
            }
            let mut word = [0u8; N_WORD_BYTES + 1];
            word[1] = b' ';
            cstrcpy(&mut word, 2, &out_text);
            let mut wo = 2isize;
            let mut first_word = true;
            let mut available: i32 = N_WORD_PHONEMES as i32;
            let mut phonemes = 0usize;
            let mut guard = 0;
            while at(&word, wo) != 0 && available > 1 && guard < N_WORD_BYTES {
                guard += 1;
                let c = utf8_in(&word, wo).0;
                if ucd::isupper(c) {
                    if let Some(w0) = wtab.first_mut() {
                        w0.flags |= FLAG_FIRST_UPPER;
                    }
                    utf8_out(ucd::tolower(c), &mut word, wo);
                } else if let Some(w0) = wtab.first_mut() {
                    w0.flags &= !FLAG_FIRST_UPPER;
                }
                let skipwords = self.dictionary_skipwords;
                let _ = has_wtab;
                self.translate_word3(tr, &mut word, wo, wtab, None, false);
                let mut piece = Vec::new();
                if first_word {
                    first_word = false;
                } else {
                    piece.push(PHON_END_WORD);
                }
                piece.extend_from_slice(cstr(&self.word_phonemes, 0));
                let n = piece.len() as i32;
                if available > 0 {
                    let room = (available - 1) as usize;
                    let k = piece.len().min(room);
                    if phonemes < words_phonemes.len() {
                        cstrcpy(&mut words_phonemes, phonemes as isize, &piece[..k]);
                    }
                }
                available -= n;
                phonemes += n as usize;
                self.dictionary_skipwords += 1;
                while self.dictionary_skipwords > 0 {
                    while at(&word, wo) != 0 && !c_isspace(sat(&word, wo)) {
                        wo += 1;
                    }
                    while c_isspace(sat(&word, wo)) {
                        wo += 1;
                    }
                    self.dictionary_skipwords -= 1;
                }
                self.dictionary_skipwords = skipwords;
            }
            if phonemes != 0 {
                let wp = cstr(&words_phonemes, 0).to_vec();
                cstrcpy(&mut self.word_phonemes, 0, &wp);
            }
        }
        flags
    }

    fn set_plist2(&mut self, ix: usize, phcode: u8) {
        let ef = self.embedded_flag;
        if let Some(p) = self.ph_list2.get_mut(ix) {
            p.phcode = phcode;
            p.stresslevel = 0;
            p.tone_ph = 0;
            p.synthflags = ef;
            p.sourceix = 0;
        }
        self.embedded_flag = 0;
    }

    fn count_syllables(&self, phonemes: &[u8]) -> i32 {
        cstr(phonemes, 0).iter().filter(|&&p| self.phz(p).typ == PH_VOWEL).count() as i32
    }

    fn word_embedded_cmd(&mut self) {
        loop {
            let embedded_cmd = self.embedded_list.get(self.embedded_read as usize).copied().unwrap_or(0x80);
            self.embedded_read += 1;
            let value = (embedded_cmd >> 8) as i32;
            match (embedded_cmd & 0x1f) as i32 {
                EMBED_Y => self.option_sayas = value,
                EMBED_F => self.option_emphasis = value,
                EMBED_B => {
                    if value == 0 {
                        self.pre_pause = 0;
                    } else {
                        self.pre_pause += value;
                    }
                }
                _ => {}
            }
            if (embedded_cmd & 0x80) != 0 || self.embedded_read >= self.embedded_ix {
                break;
            }
        }
    }

    /// `SetAlternateTranslator`.
    fn set_alternate_translator(&mut self, new_language: &[u8], which: Tr) -> i32 {
        let mut new_phoneme_tab = self.select_phoneme_table_name(new_language);
        if new_phoneme_tab >= 0 {
            let lang_ok = match which {
                Tr::Second => self.translator2_language == new_language,
                _ => self.translator3_language == new_language,
            };
            if self.exists(which) && !lang_ok {
                match which {
                    Tr::Second => self.translator2 = None,
                    _ => self.translator3 = None,
                }
            }
            if !self.exists(which) {
                let mut t = Box::new(select_translator(new_language));
                match which {
                    Tr::Second => self.translator2_language = new_language.to_vec(),
                    _ => self.translator3_language = new_language.to_vec(),
                }
                let name = t.dictionary_name.clone();
                if self.load_dictionary(&mut t, &name) != 0 {
                    let ix = self.voice_phoneme_tab_ix;
                    self.select_phoneme_table(ix);
                    new_phoneme_tab = -1;
                    match which {
                        Tr::Second => self.translator2_language.clear(),
                        _ => self.translator3_language.clear(),
                    }
                }
                t.phoneme_tab_ix = new_phoneme_tab;
                match which {
                    Tr::Second => self.translator2 = Some(t),
                    _ => self.translator3 = Some(t),
                }
            }
        }
        if self.exists(which) {
            self.tm(which).phonemes_repeat[0] = 0;
        }
        new_phoneme_tab
    }

    pub(crate) fn set_translator2(&mut self, new_language: &[u8]) -> i32 {
        self.set_alternate_translator(new_language, Tr::Second)
    }

    pub(crate) fn set_translator3(&mut self, new_language: &[u8]) -> i32 {
        self.set_alternate_translator(new_language, Tr::Third)
    }

    /// `TranslateWord2`: translates the word at `w[word]` (whose entry is `words[wix]`) and
    /// adds its phonemes to `ph_list2`.
    #[allow(clippy::too_many_arguments)]
    fn translate_word2(&mut self, tr: Tr, w: &mut [u8], word: isize, words: &mut [WordTab], wix: usize, mut pre_pause: i32, in_clause: bool) -> u32 {
        let mut flags: u32;
        let mut stress: i32;
        let mut next_stress: i32;
        let mut next_tone: u8 = 0;
        let mut srcix: u16;
        let mut max_stress: i32;
        let mut max_stress_ix: usize = 0;
        let mut prev_vowel: isize = -1;
        let mut pitch_raised = 0;
        let mut switch_phonemes: i32 = -1;
        let mut first_phoneme = true;
        let mut word_copy = [0u8; N_WORD_BYTES + 1];
        let mut word_replaced = [0u8; N_WORD_BYTES + 1];

        let w0 = wt(words, wix as isize);
        let mut len = w0.length as u16;
        if len > 31 {
            len = 31;
        }
        let source_ix: u16 = (w0.sourceix & 0x7ff) | (len << 11);

        let word_flags = w0.flags;
        if word_flags & FLAG_EMBEDDED != 0 {
            if let Some(e) = wt_mut(words, wix as isize) {
                e.flags &= !FLAG_EMBEDDED;
            }
            self.embedded_flag = SFLAG_EMBEDDED;
            self.word_embedded_cmd();
        }

        if self.n_ph_list2 >= N_PHONEME_LIST as i32 - 2 {
            return 0;
        }

        if at(w, word) == 0 || (word_flags & FLAG_DELETE_WORD) != 0 {
            if self.embedded_flag != 0 {
                let n = self.n_ph_list2 as usize;
                self.set_plist2(n, PHON_END_WORD);
                self.ph_list2[n].wordstress = 0;
                self.n_ph_list2 += 1;
                self.embedded_flag = 0;
            }
            self.word_phonemes[0] = 0;
            return 0;
        }

        if self.n_ph_list2 >= N_PHONEME_LIST as i32 - 7 - 2 {
            return 0;
        }

        if self.t(tr).prepause_timeout > 0 {
            self.tm(tr).prepause_timeout -= 1;
        }

        if (self.option_sayas & 0xf0) == 0x10 && (word_flags & FLAG_FIRST_WORD) == 0 {
            pre_pause += 4;
        }

        if word_flags & FLAG_FIRST_UPPER != 0 && self.option_capitals > 2 && self.embedded_ix < N_EMBEDDED_LIST as i32 - 6 {
            if self.embedded_flag != 0 {
                if let Some(e) = self.embedded_list.get_mut((self.embedded_ix - 1) as usize) {
                    *e &= !0x80;
                }
            }
            pitch_raised = self.option_capitals;
            if pitch_raised == 3 {
                pitch_raised = 20;
            }
            self.embedded_list[self.embedded_ix as usize] = (EMBED_P + 0x40 + 0x80 + (pitch_raised << 8)) as u32;
            self.embedded_ix += 1;
            self.embedded_flag = SFLAG_EMBEDDED;
        }

        let mut old_dictionary_name = Vec::new();
        if word_flags & FLAG_PHONEMES != 0 {
            if at(w, word) == b'_' && at(w, word + 1) == b'^' && at(w, word + 2) == b'_' {
                self.switch_language(w, word);
            } else {
                let mut wp = self.word_phonemes;
                self.encode_phonemes(w, word, &mut wp, 0);
                self.word_phonemes = wp;
            }
            flags = FLAG_FOUND;
        } else {
            let mut ix = 0usize;
            loop {
                let c2 = at(w, word + ix as isize);
                word_copy[ix] = c2;
                if c2 == b' ' || c2 == 0 || ix >= N_WORD_BYTES {
                    break;
                }
                ix += 1;
            }
            let word_copy_len = ix;

            word_replaced[2] = 0;
            let mut replaced = Vec::new();
            flags = self.translate_word(Tr::Main, w, word, Some(&mut words[wix..]), in_clause, Some(&mut replaced));
            if !replaced.is_empty() {
                cstrcpy(&mut word_replaced, 2, &replaced);
            }

            if flags & FLAG_SPELLWORD != 0 {
                memcpy(w, word, &word_copy, 0, word_copy_len);
                return flags;
            }

            if (flags & FLAG_COMBINE) != 0 && (wt(words, wix as isize + 1).flags & FLAG_PHONEMES) == 0 {
                self.combine_flag(tr, w, word, words, wix, &mut flags, in_clause);
            }

            if self.word_phonemes[0] == PHON_SWITCH {
                old_dictionary_name = self.dictionary_name.clone();
                for _switch_attempt in 0..2 {
                    memcpy(w, word, &word_copy, 0, word_copy_len);
                    let mut new_language = cstr(&self.word_phonemes, 1).to_vec();
                    if new_language.is_empty() {
                        new_language = ESPEAKNG_DEFAULT_VOICE.to_vec();
                    }
                    switch_phonemes = self.set_translator2(&new_language);
                    if switch_phonemes >= 0 {
                        if let Some(e) = wt_mut(words, wix as isize) {
                            e.flags |= FLAG_TRANSLATOR2;
                        }
                        if word_replaced[2] != 0 {
                            word_replaced[0] = 0;
                            word_replaced[1] = b' ';
                            let mut wr = word_replaced;
                            flags = self.translate_word(Tr::Second, &mut wr, 1, Some(&mut words[wix..]), false, None);
                        } else {
                            let mut rep = Vec::new();
                            flags = self.translate_word(Tr::Second, w, word, Some(&mut words[wix..]), in_clause, Some(&mut rep));
                            if !rep.is_empty() {
                                cstrcpy(&mut word_replaced, 2, &rep);
                            }
                        }
                    }
                    if self.word_phonemes[0] != PHON_SWITCH {
                        break;
                    }
                }
                if self.word_phonemes[0] == PHON_SWITCH {
                    return FLAG_SPELLWORD;
                }
                if switch_phonemes < 0 {
                    self.word_phonemes[0] = PHON_SCHWA;
                    self.word_phonemes[1] = PHON_SCHWA;
                    self.word_phonemes[2] = 0;
                }
                if switch_phonemes == -1 {
                    self.dictionary_name = old_dictionary_name.clone();
                    let ix = self.voice_phoneme_tab_ix;
                    self.select_phoneme_table(ix);
                    switch_phonemes = self.voice_phoneme_tab_ix;
                }
            }

            if word_flags & FLAG_HYPHEN == 0 {
                if (flags & FLAG_PAUSE1) != 0 && pre_pause < 1 {
                    pre_pause = 1;
                }
                if (flags & FLAG_PREPAUSE) != 0
                    && (word_flags & (FLAG_LAST_WORD | FLAG_FIRST_WORD)) == 0
                    && (wt(words, wix as isize - 1).flags & FLAG_FIRST_WORD) == 0
                    && self.t(tr).prepause_timeout == 0
                {
                    if pre_pause < 4 {
                        pre_pause = 4;
                    }
                    self.tm(tr).prepause_timeout = 3;
                }
            }

            if self.option_emphasis >= 3 && pre_pause < 1 {
                pre_pause = 1;
            }
        }

        stress = 0;
        next_stress = 1;
        srcix = 0;
        max_stress = -1;

        let found_dict_flag = if (flags & FLAG_FOUND) != 0 && (flags & FLAG_TEXTMODE) == 0 { SFLAG_DICTIONARY } else { 0 };

        while pre_pause > 0 && self.n_ph_list2 < N_PHONEME_LIST as i32 - 7 - 2 {
            let n = self.n_ph_list2 as usize;
            self.n_ph_list2 += 1;
            if pre_pause > 1 {
                self.set_plist2(n, PHON_PAUSE);
                pre_pause -= 2;
            } else {
                self.set_plist2(n, PHON_PAUSE_NOLINK);
                pre_pause -= 1;
            }
            let t = self.tm(tr);
            t.end_stressed_vowel = 0;
            t.prev_dict_flags = [0, 0];
        }
        let plist2 = self.n_ph_list2 as usize;

        if self.option_capitals == 1 && (word_flags & FLAG_FIRST_UPPER) != 0 {
            for k in 0..2 {
                if k == 1 && !((word_flags & FLAG_ALL_UPPER) != 0 && is_alpha(at(w, word + 1) as i32)) {
                    break;
                }
                let n = self.n_ph_list2 as usize;
                self.set_plist2(n, PHON_PAUSE_SHORT);
                self.set_plist2(n + 1, PHON_CAPITAL);
                self.n_ph_list2 += 2;
            }
        }

        let mut p = 0usize;
        if switch_phonemes >= 0 {
            let n = self.n_ph_list2 as usize;
            if self.word_phonemes[0] == PHON_PAUSE && self.word_phonemes[1] == PHON_SWITCH {
                if n > 0 && self.ph_list2[n - 1].phcode == PHON_SWITCH {
                    self.n_ph_list2 -= 1;
                }
            } else {
                if n > 0 && self.ph_list2[n - 1].phcode == PHON_SWITCH {
                    self.n_ph_list2 -= 1;
                } else {
                    self.set_plist2(n, PHON_SWITCH);
                }
                let n = self.n_ph_list2 as usize;
                self.ph_list2[n].tone_ph = switch_phonemes as u8;
                self.n_ph_list2 += 1;
            }
        }

        if (word_flags & FLAG_HYPHEN) != 0 && self.phz(self.word_phonemes[0]).typ == PH_PAUSE {
            p += 1;
        }

        if self.word_phonemes[p] == 0 && self.embedded_flag != 0 {
            self.word_phonemes[p] = PHON_PAUSE_VSHORT;
            if p + 1 < N_WORD_PHONEMES {
                self.word_phonemes[p + 1] = 0;
            }
        }

        loop {
            let ph_code = self.word_phonemes.get(p).copied().unwrap_or(0);
            p += 1;
            if ph_code == 0 || self.n_ph_list2 >= N_PHONEME_LIST as i32 - 3 - 2 {
                break;
            }
            if ph_code == 255 {
                continue;
            }
            let Some(ph) = self.ph(ph_code) else { continue };
            let n = self.n_ph_list2 as usize;

            if ph_code == PHON_SWITCH {
                let tab = self.word_phonemes.get(p).copied().unwrap_or(0);
                let e = &mut self.ph_list2[n];
                e.phcode = ph_code;
                e.stresslevel = 0;
                e.sourceix = 0;
                e.synthflags = 0;
                e.tone_ph = tab;
                self.n_ph_list2 += 1;
                self.select_phoneme_table(tab as i32);
                p += 1;
            } else if ph.typ == PH_STRESS {
                if ph.program == 0 {
                    next_stress = ph.std_length as i32;
                } else if prev_vowel >= 0 {
                    self.ph_list2[prev_vowel as usize].tone_ph = ph_code;
                } else {
                    next_tone = ph_code;
                }
            } else if ph_code == PHON_SYLLABIC {
                prev_vowel = n as isize - 1;
                if prev_vowel >= 0 {
                    let e = &mut self.ph_list2[prev_vowel as usize];
                    e.synthflags |= SFLAG_SYLLABLE;
                    e.stresslevel = next_stress as u8;
                }
            } else if ph_code == PHON_LENGTHEN {
                if n > 0 {
                    self.ph_list2[n - 1].synthflags |= SFLAG_LENGTHEN;
                }
            } else if ph_code == PHON_END_WORD {
                srcix = source_ix.wrapping_add(1);
            } else if ph_code == PHON_X1 {
                flags |= FLAG_DOUBLING;
            } else {
                let ef = self.embedded_flag;
                {
                    let e = &mut self.ph_list2[n];
                    e.phcode = ph_code;
                    e.tone_ph = 0;
                    e.synthflags = ef | found_dict_flag;
                    e.sourceix = srcix;
                }
                self.embedded_flag = 0;
                srcix = 0;

                if ph.typ == PH_VOWEL {
                    stress = next_stress;
                    next_stress = 1;
                    if stress >= 4 {
                        self.any_stressed_words = true;
                    }
                    if prev_vowel >= 0 && (n as isize - 1) != prev_vowel && n > 0 {
                        self.ph_list2[n - 1].stresslevel = stress as u8;
                    }
                    self.ph_list2[n].synthflags |= SFLAG_SYLLABLE;
                    prev_vowel = n as isize;
                    if stress > max_stress {
                        max_stress = stress;
                        max_stress_ix = n;
                    }
                    if next_tone != 0 {
                        self.ph_list2[n].tone_ph = next_tone;
                        next_tone = 0;
                    }
                } else if first_phoneme && (self.t(tr).prev_dict_flags[0] & FLAG_DOUBLING) != 0 {
                    self.ph_list2[n].synthflags |= SFLAG_LENGTHEN;
                }
                self.ph_list2[n].stresslevel = stress as u8;
                self.n_ph_list2 += 1;
                first_phoneme = false;
            }
        }

        if word_flags & FLAG_COMMA_AFTER != 0 {
            let n = self.n_ph_list2 as usize;
            self.set_plist2(n, PHON_PAUSE_CLAUSE);
            self.n_ph_list2 += 1;
        }

        if (word_flags & FLAG_HYPHEN) == 0 {
            self.ph_list2[plist2].sourceix = source_ix;
        }

        self.tm(tr).end_stressed_vowel = 0;
        let last = (self.n_ph_list2 - 1).max(0) as usize;
        if stress >= 4 && self.phz(self.ph_list2[last].phcode).typ == PH_VOWEL {
            self.tm(tr).end_stressed_vowel = 1;
        }

        if switch_phonemes >= 0 {
            self.dictionary_name = old_dictionary_name;
            let ix = self.voice_phoneme_tab_ix;
            self.select_phoneme_table(ix);
            let n = self.n_ph_list2 as usize;
            self.set_plist2(n, PHON_SWITCH);
            self.ph_list2[n].tone_ph = self.voice_phoneme_tab_ix as u8;
            self.n_ph_list2 += 1;
        }

        if pitch_raised > 0 {
            if let Some(e) = self.embedded_list.get_mut(self.embedded_ix as usize) {
                *e = (EMBED_P + 0x60 + 0x80 + (pitch_raised << 8)) as u32;
            }
            self.embedded_ix += 1;
            let n = self.n_ph_list2 as usize;
            self.set_plist2(n, PHON_PAUSE_SHORT);
            self.ph_list2[n].synthflags = SFLAG_EMBEDDED;
            self.n_ph_list2 += 1;
        }

        if flags & FLAG_STRESS_END2 != 0 {
            self.ph_list2[max_stress_ix].synthflags |= SFLAG_PROMOTE_STRESS;
        }

        self.tm(tr).prev_dict_flags[0] = flags;
        flags
    }

    /// `EmbeddedCommand`: parses an embedded command at `source[*source_index]`.
    fn embedded_command(&mut self, source_index_out: &mut isize) -> i32 {
        const COMMANDS: &[u8] = b"PSARHTIVYMUBF";
        let mut value: i32 = -1;
        let mut sign = 0;
        let mut source_index = *source_index_out;
        let c = at(&self.source, source_index);
        if c == b'+' {
            sign = 0x40;
            source_index += 1;
        } else if c == b'-' {
            sign = 0x60;
            source_index += 1;
        }
        if is_digit09(at(&self.source, source_index) as i32) {
            value = atoi(&self.source, source_index);
            while is_digit09(at(&self.source, source_index) as i32) {
                source_index += 1;
            }
        }
        let c = at(&self.source, source_index);
        source_index += 1;
        if self.embedded_ix >= N_EMBEDDED_LIST as i32 - 2 {
            return 0;
        }
        let cmd = if c >= 0x80 {
            return 0;
        } else if c == 0 {
            COMMANDS.len() as i32 + 1
        } else {
            match COMMANDS.iter().position(|&x| x == c) {
                Some(p) => p as i32 + 1,
                None => return 0,
            }
        };
        if value == -1 {
            value = EMBEDDED_DEFAULT[cmd as usize];
            sign = 0;
        }
        if cmd == EMBED_Y {
            self.option_sayas2 = value;
            self.count_sayas_digits = 0;
        }
        if cmd == EMBED_F {
            self.word_emphasis = if value >= 3 { FLAG_EMPHASIZED } else { 0 };
        }
        self.embedded_list[self.embedded_ix as usize] = (cmd + sign + (value << 8)) as u32;
        self.embedded_ix += 1;
        *source_index_out = source_index;
        1
    }

    /// `FindReplacementChars`: searches the `.replace` list for `c` (and following chars).
    fn find_replacement_chars(&self, tr: Tr, from: usize, c: i32, next_pos: isize, ignore_next_n: &mut i32) -> Option<usize> {
        let dict = self.t(tr).dict.clone()?;
        let d = &dict.data;
        let mut from = from as isize;
        while !is_str_totally_null(d, from, 4) {
            if from as usize >= d.len() {
                return None;
            }
            let (fc, n) = utf8_in(d, from);
            from += n;
            if c == fc {
                if at(d, from) == 0 {
                    return Some(from as usize + 1);
                }
                let mut matched = true;
                let mut nmatched = 0;
                let mut match_next = next_pos;
                while at(d, from) != 0 {
                    let (fc, n) = utf8_in(d, from);
                    from += n;
                    let (nc, m) = utf8_in(&self.source, match_next);
                    match_next += m;
                    let nc = crate::readclause::towlower2(nc, self.t(tr));
                    if nc != fc {
                        matched = false;
                    } else {
                        nmatched += 1;
                    }
                }
                if matched {
                    *ignore_next_n = nmatched;
                    return Some(from as usize + 1);
                }
            }
            while at(d, from) != 0 {
                from += 1;
            }
            from += 1;
            while at(d, from) != 0 {
                from += 1;
            }
            from += 1;
        }
        None
    }

    /// `SubstituteChar`: the `.replace` rules in `*_rules`.
    fn substitute_char(&mut self, tr: Tr, c: i32, next_in: i32, next_pos: isize, insert: &mut i32, wordflags: &mut u32) -> i32 {
        if self.ignore_next_n > 0 {
            self.ignore_next_n -= 1;
            return 8;
        }
        if c == 0 {
            return 0;
        }
        let Some(from) = self.t(tr).langopts.replace_chars else { return c };
        let mut upper_case = false;
        let mut c_lower = c;
        if ucd::isupper(c) {
            c_lower = crate::readclause::towlower2(c, self.t(tr));
            upper_case = true;
        }
        let mut ignore = 0;
        let to = self.find_replacement_chars(tr, from, c_lower, next_pos, &mut ignore);
        self.ignore_next_n = ignore;
        let Some(to) = to else { return c };
        let dict = self.t(tr).dict.clone();
        let Some(dict) = dict else { return c };
        let d = &dict.data;
        let (mut new_c, n) = utf8_in(d, to as isize);
        let to2 = to as isize + n;
        if at(d, to2) != 0 {
            let mut c2 = utf8_in(d, to2).0;
            if upper_case && ucd::isupper(next_in) {
                c2 = ucd::toupper(c2);
            }
            *insert = c2;
        }
        if upper_case {
            new_c = ucd::toupper(new_c);
        }
        *wordflags |= FLAG_CHAR_REPLACED;
        new_c
    }

    /// `TranslateChar`: language-specific character changes (Korean jamo, Dutch 'n, .replace).
    #[allow(clippy::too_many_arguments)]
    fn translate_char(&mut self, tr: Tr, ptr: isize, prev_in: i32, c: i32, next_in: i32, insert: &mut i32, wordflags: &mut u32) -> i32 {
        let code = c - 0xac00;
        if code >= 0 && c <= 0xd7af {
            let initial = (code / 28) / 21;
            let medial = (code / 28) % 21;
            let fin = code % 28;
            if initial == 11 {
                if fin > 0 {
                    *insert = fin + 0x11a7;
                }
                return medial + 0x1161;
            }
            *insert = (11 * 28 * 21) + (medial * 28) + fin + 0xac00;
            return initial + 0x1100;
        }
        let code = c - 0x3130;
        if (0..0x34).contains(&code) {
            return HANGUL_COMPATIBILITY[code as usize] as i32 + 0x1100;
        }
        let name = self.t(tr).translator_name;
        if (name == l2(b'a', b'f') || name == l2(b'n', b'l')) && c == b'\'' as i32 && !ucd::isalpha(prev_in) {
            let next2 = utf8_in(&self.source, ptr + 1).0;
            if is_space(next2) {
                if next_in == b'n' as i32 && name == l2(b'a', b'f') {
                    put(&mut self.source, ptr, b' ');
                    return 0x0259;
                }
                if next_in == b'n' as i32 || next_in == b't' as i32 {
                    return 0x0259;
                }
            }
        }
        self.substitute_char(tr, c, next_in, ptr, insert, wordflags)
    }

    fn upper_case_in_word(&self, tr: Tr, sbuf: &[u8], word: isize, c: i32) -> bool {
        if self.t(tr).translator_name != l2(b'g', b'a') {
            return false;
        }
        for p in UCASE_GA {
            let len = p.len() as isize;
            if at(sbuf, word - len) == b' ' && (0..len - 1).all(|k| at(sbuf, word - len + 1 + k) == p[k as usize]) {
                let last = p[len as usize - 1] as i32;
                if c == last || (last == b'A' as i32 && self.is_vowel(tr, c)) {
                    return true;
                }
            }
        }
        false
    }

    fn calc_word_length(source_index: isize, charix_top: i32, charix: &[i16], words: &mut [WordTab], word_count: usize) -> u8 {
        let ch = |j: isize| if j < 0 { 0 } else { charix.get(j as usize).copied().unwrap_or(0) };
        let mut j = source_index;
        while j < charix_top as isize && ch(j) <= 0 {
            j += 1;
        }
        if let Some(w) = words.get_mut(word_count) {
            w.sourceix = ch(j) as u16;
        }
        let mut k: i32 = 0;
        while ch(j) != 0 {
            if ch(j) != -1 {
                k += 1;
            }
            j += 1;
        }
        k as u8
    }

    /// `CombineFlag`: joins a word marked `$combine` with the next one.
    #[allow(clippy::too_many_arguments)]
    fn combine_flag(&mut self, tr: Tr, w: &mut [u8], word: isize, words: &mut [WordTab], wix: usize, flags: &mut u32, in_clause: bool) {
        let sylimit = self.t(tr).langopts.param[LOPT_COMBINE_WORDS];
        let mut p2 = word;
        while at(w, p2) != b' ' && (p2 as usize) < w.len() {
            p2 += 1;
        }
        let mut ok = true;
        let c_word2 = utf8_in(w, p2 + 1).0;
        if !ucd::isalpha(c_word2) {
            ok = false;
        }
        let mut flags2_0 = 0;
        if ok {
            let ph_buf = self.word_phonemes;
            let nw = words.len();
            flags2_0 = self.translate_word(tr, w, p2 + 1, Some(&mut words[(wix + 1).min(nw)..]), in_clause, None);
            if (flags2_0 & FLAG_WAS_UNPRONOUNCABLE) != 0 || self.word_phonemes[0] == PHON_SWITCH {
                ok = false;
            }
            if (sylimit & 0x100) != 0 && (flags2_0 & FLAG_ALT_TRANS) == 0 {
                ok = false;
            }
            if (sylimit & 0x200) != 0 && (wt(words, wix as isize + 1).flags & FLAG_LAST_WORD) != 0 {
                ok = false;
            }
            if !ok {
                self.set_word_phonemes(&ph_buf);
            }
        }
        if ok {
            put(w, p2, b'-');
            if let Some(e) = wt_mut(words, wix as isize) {
                e.flags &= !FLAG_ALL_UPPER;
            }
            *flags = self.translate_word(Tr::Main, w, word, Some(&mut words[wix..]), in_clause, None);
            let wp = self.word_phonemes;
            if sylimit > 0 && self.count_syllables(&wp) > (sylimit & 0x1f) {
                put(w, p2, b' ');
                *flags = self.translate_word(Tr::Main, w, word, Some(&mut words[wix..]), in_clause, None);
            } else {
                if *flags == 0 {
                    *flags = flags2_0;
                }
                *flags |= FLAG_SKIPWORDS;
                self.dictionary_skipwords = 1;
            }
        }
    }

    /// `SwitchLanguage`: `[[_^_xx]]` in phoneme input switches the phoneme table.
    fn switch_language(&mut self, w: &[u8], word: isize) {
        let mut lang_name = Vec::new();
        let mut p = word + 3;
        loop {
            let c1 = sat(w, p);
            p += 1;
            if c1 == b' ' as i32 || c1 == 0 || lang_name.len() >= 11 {
                break;
            }
            lang_name.push(ucd::tolower(c1) as u8);
        }
        let ix = self.lookup_phoneme_table(&lang_name);
        if ix > 0 {
            self.select_phoneme_table(ix);
            self.word_phonemes[0] = PHON_SWITCH;
            self.word_phonemes[1] = ix as u8;
            self.word_phonemes[2] = 0;
        }
    }

    /// `TranslateClause`: reads one clause from the decoder and translates it into
    /// `phoneme_list`.
    pub(crate) fn translate_clause(&mut self, tr: Tr) {
        let mut c: i32;
        let mut cc: i32 = 0;
        let mut source_index: isize = 0;
        let mut source_index_word: isize = 0;
        let mut prev_in: i32 = b' ' as i32;
        let mut prev_out: i32 = b' ' as i32;
        let mut prev_in_save: i32 = 0;
        let mut next_in: i32;
        let mut next_in_nbytes: isize;
        let mut char_inserted: i32 = 0;
        let mut clause_pause: i32;
        let mut pre_pause_add = 0;
        let mut all_upper_case: u32 = FLAG_ALL_UPPER;
        let mut alpha_count = 0;
        let mut finished = false;
        let mut single_quoted = false;
        let mut phoneme_mode = false;
        let mut word_flags: u32;
        let mut next_word_flags: u32;
        let new_sentence2: bool;
        let mut embedded_count = 0;
        let mut letter_count = 0;
        let mut space_inserted = false;
        let mut syllable_marked = false;
        let mut decimal_sep_count = false;
        let mut charix_top: i32 = 0;
        let mut charix = std::mem::take(&mut self.clause_charix);
        let mut words = std::mem::take(&mut self.clause_words);
        let mut word_count: usize;
        let mut sbuf = std::mem::take(&mut self.clause_sbuf);
        let mut tone = 0;

        if !self.exists(tr) {
            self.clause_charix = charix;
            self.clause_words = words;
            self.clause_sbuf = sbuf;
            return;
        }

        self.embedded_ix = 0;
        self.embedded_read = 0;
        self.pre_pause = 0;
        self.any_stressed_words = false;

        self.clause_start_char = self.count_characters;
        if self.clause_start_char < 0 {
            self.clause_start_char = 0;
        }
        self.clause_start_word = self.count_words + 1;

        for x in charix.iter_mut().take(N_TR_SOURCE) {
            *x = 0;
        }
        let mut source = std::mem::take(&mut self.source);
        let mut voice_change_name = Vec::new();
        let terminator = self.read_clause(tr, &mut source, &mut charix, &mut charix_top, N_TR_SOURCE as i32, &mut tone, &mut voice_change_name);
        self.source = source;
        let mut terminator = terminator;

        let ct = charix_top.max(0) as usize;
        for (k, v) in [(1, 0), (2, 0x7fff), (3, 0)] {
            if let Some(x) = charix.get_mut(ct + k) {
                *x = v;
            }
        }

        clause_pause = (terminator & CLAUSE_PAUSE) * 10;
        if terminator & CLAUSE_PAUSE_LONG != 0 {
            clause_pause *= 32;
        }

        let mut p = 0isize;
        while at(&self.source, p) != 0 {
            if !isspace2(at(&self.source, p) as i32) {
                break;
            }
            p += 1;
        }
        if at(&self.source, p) == 0 {
            clause_pause -= self.max_clause_pause;
            if clause_pause < 0 {
                clause_pause = 0;
            }
            if self.new_sentence {
                terminator |= CLAUSE_TYPE_SENTENCE;
            }
            self.max_clause_pause += clause_pause;
            new_sentence2 = false;
        } else {
            self.max_clause_pause = clause_pause;
            new_sentence2 = self.new_sentence;
        }
        self.tm(tr).clause_terminator = terminator;

        if new_sentence2 {
            self.count_sentences += 1;
            if self.skip_sentences > 0 {
                self.skip_sentences -= 1;
                if self.skip_sentences == 0 {
                    self.skipping_text = false;
                }
            }
        }

        self.ph_list2[0] = PhonemeList2 { phcode: PHON_PAUSE_SHORT, ..Default::default() };
        self.n_ph_list2 = 1;
        {
            let t = self.tm(tr);
            t.prev_last_stress = 0;
            t.prepause_timeout = 0;
            t.expect_verb = 0;
            t.expect_noun = 0;
            t.expect_past = 0;
            t.expect_verb_s = 0;
            t.phonemes_repeat_count = 0;
            t.end_stressed_vowel = 0;
            t.prev_dict_flags = [0, 0];
        }

        word_count = 0;
        word_flags = 0;
        next_word_flags = 0;

        sbuf[0] = 0;
        sbuf[1] = b' ';
        sbuf[2] = b' ';
        let mut ix: isize = 3;

        words[0].start = ix as u16;
        words[0].flags = 0;
        words[0].length = Self::calc_word_length(source_index, charix_top, &charix, &mut words, 0);

        let mut prev_out2: i32;
        while !finished && ix < N_TR_SOURCE as isize - 1 {
            prev_out2 = prev_out;
            prev_out = utf8_in2(&sbuf, ix - 1, true).0;

            if self.t(tr).langopts.tone_numbers != 0 && is_digit09(prev_out) && is_alpha(prev_out2) {
                prev_out = b'a' as i32;
            }

            if prev_in_save != 0 {
                prev_in = prev_in_save;
                prev_in_save = 0;
            } else if source_index > 0 {
                prev_in = utf8_in2(&self.source, source_index - 1, true).0;
            }

            let prev_source_index = source_index;

            if char_inserted != 0 {
                c = char_inserted;
                char_inserted = 0;
            } else {
                let (v, n) = utf8_in(&self.source, source_index);
                cc = v;
                source_index += n;
                c = cc;
            }

            if c == 0 {
                finished = true;
                c = b' ' as i32;
                next_in = b' ' as i32;
                next_in_nbytes = 0;
            } else {
                let (v, n) = utf8_in(&self.source, source_index);
                next_in = v;
                next_in_nbytes = n;
            }

            if c == CTRL_EMBEDDED {
                let srcix = source_index - 1;
                if prev_in != b' ' as i32 {
                    c = b' ' as i32;
                    prev_in_save = c;
                    source_index -= 1;
                } else {
                    embedded_count += self.embedded_command(&mut source_index);
                    prev_in_save = prev_in;
                    for k in srcix..source_index {
                        put(&mut self.source, k, b' ');
                    }
                    source_index = srcix;
                    continue;
                }
            }

            if self.option_sayas2 == SAYAS_KEY && c != b' ' as i32 {
                if prev_in == b' ' as i32 && next_in == b' ' as i32 {
                    self.option_sayas2 = SAYAS_SINGLE_CHARS;
                }
                c = crate::readclause::towlower2(c, self.t(tr));
            }

            if phoneme_mode {
                all_upper_case = FLAG_PHONEMES;
                if c == b']' as i32 && next_in == b']' as i32 {
                    phoneme_mode = false;
                    source_index += 1;
                    c = b' ' as i32;
                }
            } else if (self.option_sayas2 & 0xf0) == SAYAS_DIGITS {
                if ucd::isdigit(c) {
                    self.count_sayas_digits += 1;
                    if self.count_sayas_digits > (self.option_sayas2 & 0xf) {
                        c = b' ' as i32;
                        space_inserted = true;
                        self.count_sayas_digits = 0;
                    }
                } else {
                    self.count_sayas_digits = 0;
                    if ucd::isdigit(prev_out) {
                        c = b' ' as i32;
                        space_inserted = true;
                    }
                }
            } else if (self.option_sayas2 & 0x10) == 0 {
                if c == 0x92 || c == 0xb4 || c == 0x2019 || c == 0x2032 {
                    c = b'\'' as i32;
                }
                if (c == 0x2018 || c == b'?' as i32) && is_alpha(prev_out) && is_alpha(next_in) {
                    c = b'\'' as i32;
                }
                if c == CHAR_EMPHASIS {
                    c = b' ' as i32;
                    word_flags |= FLAG_FOCUS;
                }
                if c == CHAR_COMMA_BREAK {
                    c = b' ' as i32;
                    word_flags |= FLAG_COMMA_AFTER;
                }
                c = self.translate_char(tr, source_index, prev_in, c, next_in, &mut char_inserted, &mut word_flags);
                if c == 8 {
                    continue;
                }
                if char_inserted != 0 {
                    next_in = char_inserted;
                }

                let punct_within_word = self.t(tr).punct_within_word;
                if !is_alpha(c) && !is_space(c) && !wcschr(punct_within_word, c) && is_alpha(prev_out) {
                    if self.t(tr).langopts.tone_numbers != 0 && is_digit09(c) && !is_digit09(next_in) {
                    } else {
                        c = b' ' as i32;
                        space_inserted = true;
                    }
                }

                if ucd::isdigit(prev_out) {
                    if !ucd::isdigit(c) && c != b'.' as i32 && c != b',' as i32 && c != b' ' as i32 {
                        c = b' ' as i32;
                        space_inserted = true;
                    }
                } else if prev_in == b',' as i32 {
                    if c == b',' as i32 {
                        c = b' ' as i32;
                    }
                } else {
                    decimal_sep_count = false;
                }

                if c == b'[' as i32 && (next_in == 0x02 || (next_in == b'[' as i32 && self.option_phoneme_input != 0)) {
                    phoneme_mode = true;
                    source_index += 1;
                    continue;
                }

                if is_alpha(c) {
                    alpha_count += 1;
                    let ideographs = self.t(tr).langopts.ideographs != 0;
                    if !is_alpha(prev_out) || (ideographs && (c > 0x3040 || prev_out > 0x3040)) {
                        if !wcschr(punct_within_word, prev_out) {
                            letter_count = 0;
                        }
                        if prev_out != b' ' as i32 && !wcschr(punct_within_word, prev_out) {
                            c = b' ' as i32;
                            space_inserted = true;
                            if !is_bracket(prev_out) {
                                next_word_flags |= FLAG_NOSPACE;
                            }
                        } else {
                            if ucd::isupper(c) {
                                word_flags |= FLAG_FIRST_UPPER;
                            }
                            if prev_out == b' ' as i32 && ucd::isdigit(sat(&sbuf, ix - 2)) && !ucd::isdigit(prev_in) {
                                put(&mut sbuf, ix, b' ');
                                ix += 1;
                                words[word_count].start += 1;
                            }
                        }
                    }

                    if c != b' ' as i32 {
                        letter_count += 1;
                        let lbo = self.t(tr).letter_bits_offset;
                        if lbo > 0 && ((c < 0x250 && prev_out >= lbo) || (c >= lbo && letter_count > 1 && prev_out < 0x250)) && is_alpha(prev_out) {
                            c = b' ' as i32;
                            space_inserted = true;
                            word_flags |= FLAG_HYPHEN_AFTER;
                            next_word_flags |= FLAG_HYPHEN;
                        }
                    }

                    if ucd::isupper(c) {
                        c = crate::readclause::towlower2(c, self.t(tr));
                        if self.t(tr).langopts.param[LOPT_CAPS_IN_WORD] != 0 {
                            if !syllable_marked {
                                char_inserted = c;
                                c = 0x2c8;
                                syllable_marked = true;
                            }
                        } else if ucd::islower(prev_in) {
                            if prev_out != b' ' as i32 && !self.upper_case_in_word(tr, &sbuf, ix, c) {
                                c = b' ' as i32;
                                space_inserted = true;
                                prev_in_save = c;
                            }
                        } else if c != b' ' as i32 && ucd::isupper(prev_in) && ucd::islower(next_in) {
                            let next2_in = utf8_in(&self.source, source_index + next_in_nbytes).0;
                            if self.t(tr).translator_name == l2(b'n', b'l') && letter_count == 2 && c == b'j' as i32 && prev_in == b'I' as i32 {
                            } else if prev_out != b' ' as i32 && is_alpha(next2_in) {
                                c = b' ' as i32;
                                space_inserted = true;
                                prev_in_save = c;
                                next_word_flags |= FLAG_NOSPACE;
                            }
                        }
                    } else if all_upper_case != 0 && letter_count > 2 {
                        if self.t(tr).translator_name == l2(b'e', b'n') && c == b's' as i32 && next_in == b' ' as i32 {
                            c = b' ' as i32;
                            all_upper_case |= FLAG_HAS_PLURAL;
                            if at(&sbuf, ix - 1) == b'\'' {
                                put(&mut sbuf, ix - 1, b' ');
                            }
                        } else {
                            all_upper_case = 0;
                        }
                    } else {
                        all_upper_case = 0;
                    }
                } else if c == b'-' as i32 {
                    if !is_space(prev_in) && is_alpha(next_in) {
                        if prev_out != b' ' as i32 {
                            c = b' ' as i32;
                            space_inserted = true;
                        } else {
                            word_flags |= FLAG_HYPHEN;
                            if word_count > 0 {
                                words[word_count - 1].flags |= FLAG_HYPHEN_AFTER;
                            }
                            c = b' ' as i32;
                        }
                    } else if prev_in == b' ' as i32 && next_in == b' ' as i32 {
                        c = b' ' as i32;
                        pre_pause_add = 4;
                    } else if next_in == b'-' as i32 {
                        source_index += 1;
                        c = b' ' as i32;
                        pre_pause_add = 4;
                    } else if prev_out == b' ' as i32 && is_alpha(prev_out2) && !is_alpha(prev_in) {
                        put(&mut sbuf, ix, b' ');
                        ix += 1;
                        words[word_count].start += 1;
                    }
                } else if c == b'.' as i32 {
                    if prev_out == b'.' as i32 {
                        c = b' ' as i32;
                        space_inserted = true;
                    } else if word_count > 0 && (words[word_count - 1].flags & FLAG_NOSPACE) == 0 && is_alpha(prev_in) {
                        words[word_count - 1].flags |= FLAG_HAS_DOT;
                        if is_space(next_in) || next_in == b'-' as i32 {
                            c = b' ' as i32;
                        }
                    }
                } else if c == b'\'' as i32 {
                    let apostrophe = self.t(tr).langopts.param[LOPT_APOSTROPHE];
                    if ((prev_in == b'.' as i32 && next_in == b's' as i32) || ucd::isalnum(prev_in)) && is_alpha(next_in) {
                        single_quoted = false;
                    } else if (apostrophe & 1) != 0 && is_alpha(next_in) {
                        single_quoted = false;
                    } else if (apostrophe & 2) != 0 && is_alpha(prev_in) {
                        single_quoted = false;
                    } else if wcschr(self.t(tr).char_plus_apostrophe, prev_in) && prev_out2 == b' ' as i32 {
                        single_quoted = false;
                        if next_in == b' ' as i32 {
                            source_index += 1;
                        }
                    } else if prev_out == b's' as i32 && !single_quoted {
                        c = b' ' as i32;
                    } else {
                        single_quoted = is_space(prev_out);
                        pre_pause_add = 4;
                        c = b' ' as i32;
                    }
                } else if lookupwchar(&BREAKS, c) != 0 {
                    c = b' ' as i32;
                } else if ucd::isdigit(c) {
                    let decimal_sep = self.t(tr).langopts.decimal_sep;
                    if self.t(tr).langopts.tone_numbers != 0 && is_alpha(prev_out) && !is_digit(next_in) {
                    } else if prev_out != b' ' as i32 && !ucd::isdigit(prev_out) {
                        if prev_out != decimal_sep || (decimal_sep_count && decimal_sep == b',' as i32) {
                            c = b' ' as i32;
                            space_inserted = true;
                        } else {
                            decimal_sep_count = true;
                        }
                    } else if prev_out == b' ' as i32 && is_alpha(prev_out2) && !is_alpha(prev_in) {
                        put(&mut sbuf, ix, b' ');
                        ix += 1;
                        words[word_count].start += 1;
                    }
                }
            }

            if is_space(c) {
                if prev_out == b' ' as i32 {
                    word_flags |= FLAG_MULTIPLE_SPACES;
                    continue;
                }
                if cc == 0x09 || cc == 0x0a {
                    next_word_flags |= FLAG_MULTIPLE_SPACES;
                }
                if space_inserted {
                    let mut j = 0;
                    let mut k = source_index - 1;
                    while k >= source_index_word && charix.get(k.max(0) as usize).copied().unwrap_or(0) != 0 && k >= 0 {
                        if charix[k as usize] > 0 {
                            j += 1;
                        }
                        k -= 1;
                    }
                    words[word_count].length = j as u8;
                }
                source_index_word = source_index;

                put(&mut sbuf, ix, b' ');
                ix += 1;

                if word_count < N_CLAUSE_WORDS - 1 && ix > words[word_count].start as isize {
                    if embedded_count > 0 {
                        if let Some(e) = self.embedded_list.get_mut((self.embedded_ix - 1).max(0) as usize) {
                            *e |= 0x80;
                        }
                        words[word_count].flags |= FLAG_EMBEDDED;
                        embedded_count = 0;
                    }
                    if alpha_count == 0 {
                        all_upper_case &= !FLAG_ALL_UPPER;
                    }
                    words[word_count].pre_pause = self.pre_pause as u8;
                    words[word_count].flags |= all_upper_case | word_flags | self.word_emphasis;

                    if self.pre_pause > 0 {
                        let start = words[word_count].start as isize;
                        let mut j = ix;
                        while j > start {
                            let v = at(&sbuf, j - 1);
                            put(&mut sbuf, j, v);
                            j -= 1;
                        }
                        put(&mut sbuf, j, b' ');
                        words[word_count].start += 1;
                        ix += 1;
                    }

                    word_count += 1;
                    words[word_count].start = ix as u16;
                    words[word_count].flags = 0;
                    words[word_count].length = Self::calc_word_length(source_index, charix_top, &charix, &mut words, word_count);

                    word_flags = next_word_flags;
                    next_word_flags = 0;
                    self.pre_pause = 0;
                    all_upper_case = FLAG_ALL_UPPER;
                    alpha_count = 0;
                    syllable_marked = false;
                }

                if space_inserted {
                    source_index = prev_source_index;
                    char_inserted = 0;
                    space_inserted = false;
                }
            } else if ix < N_TR_SOURCE as isize - 4 {
                ix += utf8_out(c, &mut sbuf, ix);
            }
            if pre_pause_add > self.pre_pause {
                self.pre_pause = pre_pause_add;
            }
            pre_pause_add = 0;
        }

        if word_count == 0 && embedded_count > 0 {
            if let Some(e) = self.embedded_list.get_mut((self.embedded_ix - 1).max(0) as usize) {
                *e |= 0x80;
            }
            words[word_count].flags |= FLAG_EMBEDDED;
            word_count = 1;
        }

        self.tm(tr).clause_end = ix - 1;
        put(&mut sbuf, ix, 0);
        words[0].pre_pause = 0;
        words[word_count].pre_pause = 8;
        if word_count > 0 {
            let mut k = word_count - 1;
            while k > 0 && is_bracket(at(&sbuf, words[k].start as isize) as i32) {
                k -= 1;
            }
            words[k].flags |= FLAG_LAST_WORD;
            if (terminator & CLAUSE_DOT_AFTER_LAST_WORD) != 0 && (words[word_count - 1].flags & FLAG_NOSPACE) == 0 {
                words[word_count - 1].flags |= FLAG_HAS_DOT;
            }
        }
        words[0].flags |= FLAG_FIRST_WORD;

        let mut number_buf = self.clause_number_buf;
        let mut num_wtab = self.clause_num_wtab;
        let mut wix = 0usize;
        while wix < word_count && self.n_ph_list2 < N_PHONEME_LIST as i32 - 7 - 2 {
            let mut dict_flags: u32 = 0;

            self.count_words += 1;
            if self.skip_words > 0 {
                self.skip_words -= 1;
                if self.skip_words == 0 {
                    self.skipping_text = false;
                }
            }
            if self.skipping_text {
                wix += 1;
                continue;
            }

            let word = words[wix].start as isize;
            let mut pw = word;

            if ucd::isdigit(sat(&sbuf, word)) && self.t(tr).langopts.break_numbers != BREAK_THOUSANDS {
                let mut pn = 0usize;
                let thousands_sep = self.t(tr).langopts.thousands_sep;
                while pn < number_buf.len() - 20 {
                    if ucd::isdigit(sat(&sbuf, pw)) {
                        number_buf[pn] = at(&sbuf, pw);
                        pn += 1;
                        pw += 1;
                    } else if sat(&sbuf, pw) == thousands_sep
                        && at(&sbuf, pw + 1) == b' '
                        && ucd::isdigit(sat(&sbuf, pw + 2))
                        && at(&sbuf, pw + 3) != b' '
                        && at(&sbuf, pw + 4) != b' '
                    {
                        pw += 2;
                        wix += 1;
                    } else {
                        for k in word..pw {
                            put(&mut sbuf, k, b' ');
                        }
                        for k in 0..pn {
                            put(&mut sbuf, word + k as isize, number_buf[k]);
                        }
                        break;
                    }
                }
                pw = word;
            }

            let mut n_digits = 0isize;
            while ucd::isdigit(sat(&sbuf, word + n_digits)) {
                n_digits += 1;
            }

            if n_digits > 4 && n_digits <= 32 {
                let mut nw = 0usize;
                number_buf[0] = b' ';
                number_buf[1] = b' ';
                number_buf[2] = b' ';
                let mut pn = 3usize;
                let mut nx = n_digits;
                let max_digits = self.t(tr).langopts.max_digits;
                if n_digits > max_digits as isize || at(&sbuf, word) == b'0' {
                    words[wix].flags |= FLAG_INDIVIDUAL_DIGITS;
                }
                let break_numbers = self.t(tr).langopts.break_numbers;
                let thousands_sep = self.t(tr).langopts.thousands_sep;
                let decimal_sep = self.t(tr).langopts.decimal_sep;
                while pn < number_buf.len() - 20 {
                    c = at(&sbuf, pw) as i8 as i32;
                    pw += 1;
                    if !is_digit09(c) && c != decimal_sep {
                        break;
                    }
                    number_buf[pn] = c as u8;
                    pn += 1;
                    nx -= 1;
                    if nx > 0 && (break_numbers & 1u32.wrapping_shl(nx as u32)) != 0 {
                        if nw < num_wtab.len() {
                            num_wtab[nw] = words[wix];
                        }
                        nw += 1;
                        if thousands_sep != b' ' as i32 {
                            number_buf[pn] = thousands_sep as u8;
                            pn += 1;
                        }
                        number_buf[pn] = b' ';
                        pn += 1;
                        if (words[wix].flags & FLAG_INDIVIDUAL_DIGITS) == 0 {
                            if (break_numbers & 1u32.wrapping_shl((nx - 1) as u32)) != 0 {
                                number_buf[pn] = b'0';
                                number_buf[pn + 1] = b'0';
                                pn += 2;
                            }
                            if (break_numbers & 1u32.wrapping_shl((nx - 2) as u32)) != 0 {
                                number_buf[pn] = b'0';
                                pn += 1;
                            }
                        }
                    }
                }
                pw -= 1;
                for k in 0..2 {
                    if nw + k < num_wtab.len() {
                        num_wtab[nw + k] = wt(&words, wix as isize + k as isize);
                    }
                }
                for j in 1..=nw.min(num_wtab.len() - 1) {
                    num_wtab[j].flags &= !(FLAG_MULTIPLE_SPACES | FLAG_EMBEDDED);
                }
                for k in 0..16 {
                    if pn + k < number_buf.len() {
                        number_buf[pn + k] = at(&sbuf, pw + k as isize);
                    }
                }
                if pn + 16 < number_buf.len() {
                    number_buf[pn + 16] = 0;
                }
                nw = 0;
                let mut q = 3usize;
                while q < pn {
                    let pp = words[wix].pre_pause as i32;
                    dict_flags = self.translate_word2(tr, &mut number_buf, q as isize, &mut num_wtab, nw.min(49), pp, false);
                    nw += 1;
                    loop {
                        let b = number_buf.get(q).copied().unwrap_or(b' ');
                        q += 1;
                        if b == b' ' || q >= number_buf.len() {
                            break;
                        }
                    }
                    words[wix].pre_pause = 0;
                }
            } else {
                self.pre_pause = 0;
                let pp = words[wix].pre_pause as i32;
                dict_flags = self.translate_word2(tr, &mut sbuf, word, &mut words, wix, pp, true);

                if wix + 1 < words.len() && self.pre_pause > words[wix + 1].pre_pause as i32 {
                    words[wix + 1].pre_pause = self.pre_pause as u8;
                    self.pre_pause = 0;
                }

                if dict_flags & FLAG_SPELLWORD != 0 {
                    let mut pw = word;
                    let mut guard = 0;
                    while at(&sbuf, pw) != b' ' && guard < N_TR_SOURCE {
                        guard += 1;
                        number_buf = [0u8; 150];
                        for x in number_buf[..9].iter_mut() {
                            *x = b' ';
                        }
                        let nx = utf8_in(&sbuf, pw).1;
                        for k in 0..nx {
                            number_buf[2 + k as usize] = at(&sbuf, pw + k);
                        }
                        self.translate_word2(tr, &mut number_buf, 2, &mut words, wix, 0, false);
                        pw += nx;
                    }
                }

                if (dict_flags & (FLAG_ALLOW_DOT | FLAG_NEEDS_DOT)) != 0
                    && wix as i32 == word_count as i32 - 1 - self.dictionary_skipwords
                    && (terminator & CLAUSE_DOT_AFTER_LAST_WORD) != 0
                {
                    clause_pause = 10;
                }
            }

            if dict_flags & FLAG_SKIPWORDS != 0 {
                while self.dictionary_skipwords > 0 {
                    if let Some(e) = words.get_mut(wix + self.dictionary_skipwords as usize) {
                        e.flags |= FLAG_DELETE_WORD;
                    }
                    self.dictionary_skipwords -= 1;
                }
            }
            wix += 1;
        }

        self.clause_number_buf = number_buf;
        self.clause_num_wtab = num_wtab;
        self.clause_charix = charix;
        self.clause_words = words;
        self.clause_sbuf = sbuf;

        if self.embedded_read < self.embedded_ix {
            self.word_embedded_cmd();
        }

        for k in 0..2 {
            let n = self.n_ph_list2 as usize + k;
            if let Some(p2) = self.ph_list2.get_mut(n) {
                p2.phcode = PHON_PAUSE;
                p2.stresslevel = 0;
                p2.sourceix = source_index as u16;
                p2.synthflags = 0;
            }
        }
        self.n_ph_list2 += 2;

        if self.eof() && (word_count == 0 || self.option_endpause == 0) {
            clause_pause = 10;
        }

        self.make_phoneme_list(tr, clause_pause, new_sentence2);

        if embedded_count != 0 {
            let n = (self.n_phoneme_list - 2).max(0) as usize;
            self.phoneme_list[n].synthflags = SFLAG_EMBEDDED;
        }

        self.new_sentence = (terminator & CLAUSE_TYPE_SENTENCE) != 0;
    }
}
