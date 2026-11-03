//! readclause.c: reading one clause of text from the decoder. SSML is never enabled through
//! `espeak_TextToPhonemes`, so the SSML branches (`option_ssml`) are not ported.

use crate::common::*;
use crate::defs::*;
use crate::engine::{Espeak, Tr};
use crate::ucd;

pub(crate) fn towlower2(c: i32, tr: &Translator) -> i32 {
    if c == b'I' as i32 && tr.langopts.dotless_i != 0 {
        return 0x131;
    }
    ucd::tolower(c)
}

fn is_roman_u(c: i32) -> bool {
    c == b'I' as i32 || c == b'V' as i32 || c == b'X' as i32 || c == b'L' as i32
}

fn lookupwchar2(list: &[u16], c: i32) -> i32 {
    let mut ix = 0;
    while ix + 1 < list.len() && list[ix] != 0 {
        if list[ix] as i32 == c {
            return list[ix + 1] as i32;
        }
        ix += 2;
    }
    0
}

fn utf8_push(buf: &mut [u8], ix: &mut isize, c: i32) {
    *ix += utf8_out(c, buf, *ix);
}

impl Espeak {
    pub(crate) fn eof(&self) -> bool {
        if self.ungot_char != 0 {
            return false;
        }
        self.decoder.eof()
    }

    fn get_c(&mut self) -> i32 {
        let c1 = self.ungot_char;
        if c1 != 0 {
            self.ungot_char = 0;
            return c1;
        }
        self.count_characters += 1;
        self.decoder.getc()
    }

    fn unget_c(&mut self, c: i32) {
        self.ungot_char = c;
    }

    fn terminate_buf_with_space_and_zero(&mut self, buf: &mut [u8], index: isize, ungetc: Option<i32>) {
        put(buf, index, b' ');
        put(buf, index + 1, 0);
        if let Some(c) = ungetc {
            self.unget_c(c);
        }
    }

    /// `DecodeWithPhonemeMode`: phonemes written back as `[\002...]]` phoneme-mode text.
    fn decode_with_phoneme_mode(&mut self, phonemes: &mut [u8], tr: Tr, tr2: Option<Tr>, flags: [u32; 2]) -> Vec<u8> {
        let mut out = vec![b'[', 2];
        match tr2 {
            None => {
                self.set_word_stress(tr, phonemes, 0, Some(flags), -1, 0);
                let d = self.decode_phonemes(phonemes);
                out.extend_from_slice(&d);
                out.extend_from_slice(b"]]");
            }
            Some(tr2) => {
                self.set_word_stress(tr2, phonemes, 0, Some(flags), -1, 0);
                let d = self.decode_phonemes(phonemes);
                out.extend_from_slice(b"_^_");
                out.extend_from_slice(ESPEAKNG_DEFAULT_VOICE);
                out.push(b' ');
                out.extend_from_slice(&d);
                out.extend_from_slice(b" _^_");
                out.extend_from_slice(&word_to_string2(self.t(tr).translator_name));
                out.extend_from_slice(b"]]");
            }
        }
        out
    }

    fn lookup_special(&mut self, tr: Tr, string: &[u8]) -> Option<Vec<u8>> {
        let mut flags = [0u32; 2];
        let mut phonemes = [0u8; N_WORD_PHONEMES];
        if self.lookup_dict_list(tr, string, 0, false, &mut phonemes, &mut flags, 0, None, None) {
            return Some(self.decode_with_phoneme_mode(&mut phonemes, tr, None, flags));
        }
        None
    }

    /// `LookupCharName`: phoneme-mode text speaking the name of character `c`.
    fn lookup_char_name(&mut self, tr: Tr, c: i32, only: bool) -> Vec<u8> {
        let mut flags = [0u32; 2];
        let mut single_letter = [0u8; 24];
        let mut phonemes = [0u8; N_WORD_PHONEMES];
        let mut lang_name = false;
        single_letter[1] = b'_';
        let ix = utf8_out(c, &mut single_letter, 2) as usize;
        single_letter[2 + ix] = 0;

        if only {
            self.lookup_dict_list(tr, &single_letter, 2, false, &mut phonemes, &mut flags, 0, None, None);
        } else {
            if !self.lookup_dict_list(tr, &single_letter, 1, false, &mut phonemes, &mut flags, 0, None, None)
                && !self.lookup_dict_list(tr, &single_letter, 2, false, &mut phonemes, &mut flags, 0, None, None)
            {
                single_letter[1] = b' ';
                let mut sl = single_letter;
                self.translate_rules(tr, &mut sl, 2, &mut phonemes, 60, None, 0, None);
            }
            if (phonemes[0] == 0 || phonemes[0] == PHON_SWITCH) && self.t(tr).translator_name != l2(b'e', b'n') {
                self.set_translator2(ESPEAKNG_DEFAULT_VOICE);
                single_letter[1] = b'_';
                if !self.lookup_dict_list(Tr::Second, &single_letter, 1, false, &mut phonemes, &mut flags, 0, None, None) {
                    self.lookup_dict_list(Tr::Second, &single_letter, 2, false, &mut phonemes, &mut flags, 0, None, None);
                }
                if phonemes[0] != 0 {
                    lang_name = true;
                } else {
                    let ix = self.voice_phoneme_tab_ix;
                    self.select_phoneme_table(ix);
                }
            }
        }

        if phonemes[0] != 0 {
            if lang_name {
                let out = self.decode_with_phoneme_mode(&mut phonemes, tr, Some(Tr::Second), flags);
                let ix = self.voice_phoneme_tab_ix;
                self.select_phoneme_table(ix);
                out
            } else {
                self.decode_with_phoneme_mode(&mut phonemes, tr, None, flags)
            }
        } else if !only {
            b"[\x02(X1)(X1)(X1)]]".to_vec()
        } else {
            Vec::new()
        }
    }

    /// `AnnouncePunctuation`: with punctuation announcement on (`\x01B` in the text).
    #[allow(clippy::too_many_arguments)]
    fn announce_punctuation(&mut self, tr: Tr, c1: i32, c2_ptr: &mut i32, output: &mut [u8], bufix: &mut isize, end_clause: bool) -> i32 {
        let mut c2 = *c2_ptr;
        let mut buf: Vec<u8> = Vec::new();
        let mut punctname: Option<Vec<u8>> = None;

        if c1 == b'.' as i32 && end_clause && c2 != b'.' as i32 {
            punctname = self.lookup_special(tr, b"_.p\0");
        }
        let punctname = match punctname {
            Some(p) => p,
            None => self.lookup_char_name(tr, c1, false),
        };

        if *bufix == 0 || !end_clause || (self.t(tr).langopts.param[LOPT_ANNOUNCE_PUNCT] & 2) != 0 {
            let mut punct_count = 1;
            while !self.eof() && c2 == c1 && c1 != b'<' as i32 {
                punct_count += 1;
                c2 = self.get_c();
            }
            *c2_ptr = c2;
            if end_clause {
                self.unget_c(c2);
            }
            if punct_count == 1 {
                buf.push(b' ');
                buf.extend_from_slice(&punctname);
            } else if punct_count < 4 {
                buf.extend_from_slice(b"\x01+10S");
                for _ in 0..punct_count {
                    buf.push(b' ');
                    buf.extend_from_slice(&punctname);
                }
                buf.extend_from_slice(b" \x01-10S");
            } else {
                buf.push(b' ');
                buf.extend_from_slice(&punctname);
                buf.extend_from_slice(format!(" {punct_count} ").as_bytes());
                buf.extend_from_slice(&punctname);
            }
        } else {
            self.ungot_char2 = c1;
            buf = b" ".to_vec();
            self.unget_c(c2);
        }

        let bufix1 = *bufix;
        buf.truncate(199);
        cstrcpy(output, *bufix, &buf);
        *bufix += buf.len() as isize;

        if !end_clause {
            return -1;
        }
        if c1 == b'-' as i32 {
            return CLAUSE_NONE;
        }
        let attributes = ucd::clause_type_from_codepoint(c1);
        let short_pause = if (attributes & CLAUSE_INTONATION_TYPE) == 0x1000 { CLAUSE_SHORTCOMMA } else { CLAUSE_SHORTFALL };
        if bufix1 > 0 && (self.t(tr).langopts.param[LOPT_ANNOUNCE_PUNCT] & 2) == 0 {
            if (attributes & !CLAUSE_OPTIONAL_SPACE_AFTER) == CLAUSE_SEMICOLON {
                return CLAUSE_SHORTFALL;
            }
            return short_pause;
        }
        if attributes & CLAUSE_TYPE_SENTENCE != 0 {
            return attributes;
        }
        short_pause
    }

    /// `ReadClause`: reads the next clause into `buf`, returning its terminator.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_clause(&mut self, tr: Tr, buf: &mut [u8], charix: &mut [i16], charix_top: &mut i32, n_buf: i32, tone_type: &mut i32, voice_change: &mut Vec<u8>) -> i32 {
        let mut c1: i32 = b' ' as i32;
        let mut c2: i32;
        let mut cprev: i32 = b' ' as i32;
        let mut c_next: i32 = 0;
        let mut ix: isize = 0;
        let mut linelength = 0;
        let mut phoneme_mode = 0;
        let mut any_alnum = false;
        let mut punct_data: i32;
        let mut is_end_clause: bool;
        let mut announced_punctuation: i32;
        let mut stressed_word = false;
        let end_clause_after_tag = 0;
        let n_buf = n_buf as isize;

        if self.clear_skipping_text {
            self.skipping_text = false;
            self.clear_skipping_text = false;
        }

        {
            let t = self.tm(tr);
            t.phonemes_repeat_count = 0;
            t.clause_upper_count = 0;
            t.clause_lower_count = 0;
        }
        *tone_type = 0;
        voice_change.clear();

        if self.ungot_char2 != 0 {
            c2 = self.ungot_char2;
        } else if self.eof() {
            c2 = 0;
        } else {
            c2 = self.get_c();
        }

        while !self.eof() || self.ungot_char != 0 || self.ungot_char2 != 0 || self.ungot_string_ix >= 0 {
            if !ucd::isalnum(c1) {
                if self.end_character_position > 0 && self.count_characters > self.end_character_position {
                    return CLAUSE_EOF;
                }
                if self.skip_characters > 0 && self.count_characters >= self.skip_characters {
                    self.clear_skipping_text = true;
                    self.skip_characters = 0;
                    self.unget_c(c2);
                    return CLAUSE_NONE;
                }
            }
            let cprev2 = cprev;
            cprev = c1;
            c1 = c2;

            if self.ungot_string_ix >= 0 && at(&self.ungot_string, self.ungot_string_ix as isize) == 0 {
                self.ungot_string_ix = -1;
            }
            if self.ungot_string_ix == 0 && self.ungot_char2 == 0 {
                c1 = at(&self.ungot_string, 0) as i32;
                self.ungot_string_ix += 1;
            }
            if self.ungot_string_ix >= 0 {
                c2 = at(&self.ungot_string, self.ungot_string_ix as isize) as i32;
                self.ungot_string_ix += 1;
            } else if self.eof() {
                c2 = b' ' as i32;
            } else {
                c2 = self.get_c();
            }

            self.ungot_char2 = 0;

            if self.ignore_text {
                continue;
            }

            if c2 == b'\n' as i32 && self.option_linelength == -1 {
                let mut terminator = ucd::clause_type_from_codepoint(c1);
                if terminator == CLAUSE_NONE {
                    if let Some(x) = charix.get_mut(ix as usize) {
                        *x = (self.count_characters - self.clause_start_char) as i16;
                    }
                    *charix_top = ix as i32;
                    utf8_push(buf, &mut ix, c1);
                    terminator = CLAUSE_PERIOD;
                }
                self.terminate_buf_with_space_and_zero(buf, ix, None);
                return terminator;
            }

            if c1 == CTRL_EMBEDDED {
                if c2 == b'V' as i32 {
                    put(buf, ix, 0);
                    ix += 1;
                    loop {
                        if self.eof() {
                            break;
                        }
                        c1 = self.get_c();
                        if ucd::isspace(c1) || ix >= n_buf - 1 {
                            break;
                        }
                        put(buf, ix, c1 as u8);
                        ix += 1;
                    }
                    put(buf, ix, 0);
                    return CLAUSE_VOICE;
                } else if c2 == b'B' as i32 {
                    cstrcpy(buf, ix, b"   ");
                    ix += 3;
                    if !self.eof() && {
                        c2 = self.get_c();
                        c2 == b'0' as i32
                    } {
                        self.option_punctuation = 0;
                    } else {
                        self.option_punctuation = 1;
                        self.option_punctlist[0] = 0;
                        if c2 != b'1' as i32 {
                            let mut j = 0;
                            while !self.eof() && !ucd::isspace(c2) && j < crate::engine::N_PUNCTLIST - 1 {
                                self.option_punctlist[j] = c2 as u32;
                                j += 1;
                                c2 = self.get_c();
                                put(buf, ix, b' ');
                                ix += 1;
                            }
                            self.option_punctlist[j] = 0;
                            self.option_punctuation = 2;
                        }
                    }
                    if !self.eof() {
                        c2 = self.get_c();
                    }
                    continue;
                }
            }

            linelength += 1;

            if let Some(i) = Some(lookupwchar2(self.t(tr).chars_ignore, c1)).filter(|&i| i != 0) {
                if i == 1 {
                    continue;
                }
                c1 = i;
            }

            if ucd::isalnum(c1) {
                any_alnum = true;
            } else {
                if stressed_word {
                    stressed_word = false;
                    c1 = CHAR_EMPHASIS;
                    self.unget_c(c2);
                    c2 = b' ' as i32;
                }
                if c1 == 0xf0b {
                    c1 = b' ' as i32;
                }
                if c1 == 0xd4d && c2 == 0x200d {
                    c1 = 0xd4e;
                }
            }

            if ucd::isupper(c1) {
                self.tm(tr).clause_upper_count += 1;
                if self.option_capitals == 2 && self.sayas_mode == 0 && !ucd::isupper(cprev) {
                    if let Some(text_buf) = self.lookup_special(tr, b"_cap\0") {
                        let j = text_buf.len().min(29) as isize;
                        if ix + j < n_buf {
                            cstrcpy(buf, ix, &text_buf[..j as usize]);
                            ix += j;
                        }
                    }
                }
            } else if ucd::isalpha(c1) {
                self.tm(tr).clause_lower_count += 1;
            }

            if self.option_phoneme_input != 0 {
                if phoneme_mode > 0 {
                    phoneme_mode -= 1;
                } else if c1 == b'[' as i32 && c2 == b'[' as i32 {
                    phoneme_mode = -1;
                } else if c1 == b']' as i32 && c2 == b']' as i32 {
                    phoneme_mode = 2;
                }
            }

            if c1 == b'\n' as i32 {
                let mut parag = 0;
                while !self.eof() && ucd::isspace(c2) {
                    if c2 == b'\n' as i32 {
                        parag += 1;
                    }
                    c2 = self.get_c();
                }
                if parag > 0 {
                    self.terminate_buf_with_space_and_zero(buf, ix, Some(c2));
                    if parag > 3 {
                        parag = 3;
                    }
                    return (CLAUSE_PARAGRAPH - 30) + 30 * parag;
                }
                if linelength <= self.option_linelength {
                    self.terminate_buf_with_space_and_zero(buf, ix, Some(c2));
                    return CLAUSE_COLON;
                }
                linelength = 0;
            }

            announced_punctuation = 0;

            if phoneme_mode == 0 && self.sayas_mode == 0 {
                is_end_clause = false;
                let _ = end_clause_after_tag;

                if c1 == b'.' as i32 && c2 == b'.' as i32 {
                    loop {
                        if self.eof() {
                            break;
                        }
                        c_next = self.get_c();
                        if c_next != b'.' as i32 {
                            break;
                        }
                        c1 = 0x2026;
                        c2 = b' ' as i32;
                    }
                    if c1 == 0x2026 {
                        c2 = c_next;
                    } else {
                        self.unget_c(c_next);
                    }
                }

                punct_data = ucd::clause_type_from_codepoint(c1);
                if punct_data != CLAUSE_NONE {
                    if punct_data & (CLAUSE_QUESTION | CLAUSE_EXCLAMATION) != 0 {
                        while !self.eof() && (ucd::clause_type_from_codepoint(c2) & (CLAUSE_QUESTION | CLAUSE_EXCLAMATION)) != 0 {
                            c_next = self.get_c();
                            c2 = c_next;
                        }
                    }
                    if punct_data & CLAUSE_PUNCTUATION_IN_WORD != 0 {
                        stressed_word = true;
                        *tone_type = (punct_data >> 12) & 0xf;
                        continue;
                    }
                    if ucd::isspace(c2) || (punct_data & CLAUSE_OPTIONAL_SPACE_AFTER) != 0 || is_bracket(c2) || c2 == b'?' as i32 || self.eof() || c2 == CTRL_EMBEDDED {
                        is_end_clause = true;
                    }
                }

                if c1 == 0xe000 + b'<' as i32 {
                    c1 = b'<' as i32;
                }
                if self.option_punctuation != 0
                    && ucd::ispunct(c1)
                    && !self.audio_text
                    && (self.option_punctuation == 1 || (c1 != 0 && wcschr(&self.option_punctlist, c1)))
                {
                    self.tm(tr).phonemes_repeat_count = 0;
                    let terminator = self.announce_punctuation(tr, c1, &mut c2, buf, &mut ix, is_end_clause);
                    if terminator >= 0 {
                        return terminator;
                    }
                    announced_punctuation = c1;
                }

                if (punct_data & CLAUSE_SPEAK_PUNCTUATION_NAME) != 0 && announced_punctuation == 0 {
                    let name = self.lookup_char_name(tr, c1, true);
                    cstrcpy(buf, ix, &name);
                    if at(buf, ix) != 0 {
                        ix += cstrlen(buf, ix) as isize;
                        announced_punctuation = c1;
                        punct_data &= !CLAUSE_INTONATION_TYPE;
                    }
                }

                if is_end_clause {
                    let mut nl_count = 0;
                    c_next = c2;
                    if ucd::isspace(c_next) {
                        while !self.eof() && ucd::isspace(c_next) {
                            if c_next == b'\n' as i32 {
                                nl_count += 1;
                            }
                            c_next = self.get_c();
                        }
                    }
                    if c1 == b'.' as i32 && nl_count < 2 {
                        punct_data |= CLAUSE_DOT_AFTER_LAST_WORD;
                    }
                    if nl_count == 0 {
                        if c1 == b',' as i32
                            && cprev == b'.' as i32
                            && self.t(tr).translator_name == l2(b'h', b'u')
                            && ucd::isdigit(cprev2)
                            && (ucd::isdigit(c_next) || ucd::islower(c_next))
                        {
                            c1 = CHAR_COMMA_BREAK;
                            is_end_clause = false;
                        }
                        if c1 == b'.' as i32 && c_next == b'\'' as i32 && self.decoder.peekc() == b's' as i32 {
                            is_end_clause = false;
                        }
                        if c1 == b'.' as i32 {
                            if (self.t(tr).langopts.numbers & NUM_ORDINAL_DOT) != 0
                                && (ucd::isdigit(cprev) || (is_roman_u(cprev) && (is_roman_u(cprev2) || ucd::isspace(cprev2))))
                            {
                                if !ucd::isdigit(cprev) || ucd::islower(c_next) || c_next == b'-' as i32 {
                                    is_end_clause = false;
                                }
                            }
                            if ucd::islower(c_next) && !self.t(tr).langopts.lowercase_sentence {
                                is_end_clause = false;
                            }
                            if !any_alnum {
                                c1 = b' ' as i32;
                                is_end_clause = false;
                            }
                        } else if !any_alnum {
                            is_end_clause = false;
                        }
                    }

                    if is_end_clause {
                        self.terminate_buf_with_space_and_zero(buf, ix, Some(c_next));
                        if ucd::isdigit(cprev) && !is_alpha(c_next) {
                            punct_data &= !CLAUSE_DOT_AFTER_LAST_WORD;
                        }
                        if nl_count > 1 {
                            if punct_data == CLAUSE_QUESTION || punct_data == CLAUSE_EXCLAMATION {
                                return punct_data + 35;
                            }
                            return CLAUSE_PARAGRAPH;
                        }
                        return punct_data;
                    } else if !self.eof() && ucd::isspace(c2) {
                        self.unget_c(c_next);
                    }
                }
            }

            if c1 == announced_punctuation {
                if is_bracket(c1) {
                    c1 = 0xe000 + b'(' as i32;
                } else if c1 != b'-' as i32 {
                    c1 = b' ' as i32;
                }
            }

            let mut j = ix + 1;
            if c1 == 0xe000 + b'<' as i32 {
                c1 = b'<' as i32;
            }
            utf8_push(buf, &mut ix, c1);
            if !ucd::isspace(c1) && !is_bracket(c1) {
                if let Some(x) = charix.get_mut(ix as usize) {
                    *x = (self.count_characters - self.clause_start_char) as i16;
                }
                while j < ix {
                    if let Some(x) = charix.get_mut(j as usize) {
                        *x = -1;
                    }
                    j += 1;
                }
            }
            *charix_top = ix as i32;

            if (ix > n_buf - 75 && !is_alpha(c1) && !ucd::isdigit(c1)) || ix >= n_buf - 4 {
                self.terminate_buf_with_space_and_zero(buf, ix, Some(c2));
                return CLAUSE_NONE;
            }
        }

        if stressed_word {
            utf8_push(buf, &mut ix, CHAR_EMPHASIS);
        }
        self.terminate_buf_with_space_and_zero(buf, ix, None);
        CLAUSE_EOF
    }
}
