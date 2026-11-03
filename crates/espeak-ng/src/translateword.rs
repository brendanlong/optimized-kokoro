//! translateword.c: translating one word, including spelling it letter by letter.

use crate::common::*;
use crate::defs::*;
use crate::dictionary::STRESS_PHONEMES;
use crate::engine::{Espeak, Tr};
use crate::numbers::is_superscript;
use crate::tr_languages::alphabet_from_char;
use crate::ucd;

const HEX_LETTERS: [&[u8]; 6] = [b"'e:j\0", b"b'i:\0", b"s'i:\0", b"d'i:\0", b"'i:\0", b"'ef\0"];
const MODIFIERS: [Option<&[u8]>; 4] = [None, Some(b"_sub\0"), Some(b"_sup\0"), None];
const NUMBER_RANGES: [i32; 16] = [0x660, 0x6f0, 0x966, 0x9e6, 0xa66, 0xae6, 0xb66, 0xbe6, 0xc66, 0xce6, 0xd66, 0xe50, 0xed0, 0xf20, 0x1040, 0x1090];

fn same_alphabet(a: Option<&'static Alphabet>, b: Option<&'static Alphabet>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => false,
    }
}

fn non_ascii_number(letter: i32) -> i32 {
    for &base in NUMBER_RANGES.iter() {
        if letter < base {
            break;
        }
        if letter < base + 10 {
            return letter - base + b'0' as i32;
        }
    }
    -1
}

/// `snprintf(out, N_WORD_PHONEMES, "%s%s%s", ...)`.
fn concat3(out: &mut [u8; N_WORD_PHONEMES], a: &[u8], b: &[u8], c: &[u8]) {
    let mut v = Vec::with_capacity(N_WORD_PHONEMES);
    v.extend_from_slice(cstr(a, 0));
    v.extend_from_slice(cstr(b, 0));
    v.extend_from_slice(cstr(c, 0));
    cstrcpy(out, 0, &v);
}

impl Espeak {
    /// `TranslateWord3`. The word starts at `w[word_start]` and ends at a space.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn translate_word3(&mut self, tr: Tr, w: &mut [u8], word_start: isize, wtab: &mut [WordTab], word_out: Option<&mut Vec<u8>>, in_clause: bool) -> u32 {
        let mut dictionary_flags = [0u32; 2];
        let mut dictionary_flags2 = [0u32; 2];
        let mut end_type: u32;
        let mut end_type1: u32 = 0;
        let mut prefix_type: u32 = 0;
        let mut phonemes = [0u8; N_WORD_PHONEMES];
        let mut phonemes2 = [0u8; N_WORD_PHONEMES];
        let mut prefix_phonemes = [0u8; N_WORD_PHONEMES];
        let mut unpron_phonemes = [0u8; N_WORD_PHONEMES];
        let mut end_phonemes = [0u8; N_WORD_PHONEMES];
        let mut word_copy = [0u8; N_WORD_BYTES];
        let mut word_copy2 = [0u8; N_WORD_BYTES];
        let mut prefix_chars = [0u8; 0x3f + 2];
        let mut found = false;
        let mut last_char: i32 = 0;
        let mut prefix_flags = 0;
        let mut emphasize_allcaps = 0;
        let mut was_unpronouncable = 0;
        let mut wtab_null = [WordTab::default(); 8];
        let wtab: &mut [WordTab] = if wtab.is_empty() { &mut wtab_null } else { wtab };
        let wt = |wtab: &[WordTab], k: usize| wtab.get(k).copied().unwrap_or_default();

        let mut wflags = wt(wtab, 0).flags;
        self.dictionary_skipwords = 0;

        if self.t(tr).dict.is_none() {
            self.word_phonemes[0] = 0;
            return 0;
        }

        let mut word1 = word_start;
        if at(w, word1) == b' ' {
            word1 += 1;
        }
        let mut wordx = word1;
        let first_char = utf8_in(w, wordx).0;
        let mut word_length = 0;
        while at(w, wordx) != 0 && at(w, wordx) != b' ' {
            let (c, n) = utf8_in(w, wordx);
            last_char = c;
            wordx += n;
            word_length += 1;
        }
        let mut word_copy_length = (wordx - word_start).max(0) as usize;
        if word_copy_length >= N_WORD_BYTES {
            word_copy_length = N_WORD_BYTES - 1;
        }
        memcpy(&mut word_copy2, 0, w, word_start, word_copy_length);

        let mut spell_word = 0;

        if word_length == 1 && (wflags & FLAG_TRANSLATOR2) != 0 {
            let c_temp = utf8_in(w, wordx + 1).0;
            if !is_alpha(c_temp) || !same_alphabet(alphabet_from_char(last_char), alphabet_from_char(c_temp)) {
                spell_word = 1;
            }
        }

        if self.option_sayas == SAYAS_KEY {
            if word_length == 1 {
                spell_word = 4;
            } else {
                word1 -= 1;
                put(w, word1, b'_');
                found = self.lookup_dict_list(tr, w, word1, in_clause, &mut phonemes, &mut dictionary_flags, 0, Some(wtab), None);
            }
        }

        if self.option_sayas & 0x10 != 0 {
            spell_word = self.option_sayas & 0xf;
        } else {
            let mut replacement = Vec::new();
            if !found {
                found = self.lookup_dict_list(tr, w, word1, in_clause, &mut phonemes, &mut dictionary_flags, FLAG_ALLOW_TEXTMODE, Some(wtab), Some(&mut replacement));
            }
            if (dictionary_flags[0] & (FLAG_ALLOW_DOT | FLAG_NEEDS_DOT)) != 0 && at(w, wordx + 1) == b'.' {
                put(w, wordx + 1, b' ');
            }
            if dictionary_flags[0] & FLAG_TEXTMODE != 0 {
                if let Some(out) = word_out {
                    out.clear();
                    if !replacement.is_empty() {
                        out.extend_from_slice(cstr(&replacement, 2));
                    } else {
                        out.extend_from_slice(cstr(w, word1));
                    }
                }
                return dictionary_flags[0];
            } else if !found && (dictionary_flags[0] & FLAG_SKIPWORDS) != 0 && (dictionary_flags[0] & FLAG_ABBREV) == 0 {
                wordx = word1;
                let mut ix = 0;
                while ix < self.dictionary_skipwords {
                    if at(w, wordx) == b' ' {
                        put(w, wordx, b'-');
                        ix += 1;
                    }
                    wordx += 1;
                    if wordx as usize >= w.len() {
                        break;
                    }
                }
            }

            if word_length == 1 && self.dictionary_skipwords == 0 && self.check_dotted_abbrev(w, word1) != 0 {
                dictionary_flags = [0, 0];
                spell_word = 1;
                if self.dictionary_skipwords != 0 {
                    dictionary_flags[0] = FLAG_SKIPWORDS;
                }
            }

            if phonemes[0] == PHON_SWITCH {
                self.set_word_phonemes(&phonemes);
                return 0;
            }

            if !found && (dictionary_flags[0] & FLAG_ABBREV) != 0 {
                spell_word = 1;
            }

            if !found && ucd::isdigit(first_char) {
                let mut wp = self.word_phonemes;
                self.lookup(tr, b"_0lang\0", &mut wp);
                self.word_phonemes = wp;
                if self.word_phonemes[0] == PHON_SWITCH {
                    return 0;
                }
                if (self.t(tr).langopts.numbers2 & NUM2_ENGLISH_NUMERALS) != 0 && (wt(wtab, 0).flags & FLAG_CHAR_REPLACED) == 0 {
                    self.word_phonemes[0] = PHON_SWITCH;
                    self.word_phonemes[1] = 0;
                    return 0;
                }
                found = self.translate_number(tr, w, word1, &mut phonemes, &mut dictionary_flags, wtab, 0, in_clause);
            }

            if !found && (wflags & FLAG_UPPERS) != FLAG_FIRST_UPPER {
                let numbers = self.t(tr).langopts.numbers;
                if ((numbers & NUM_ROMAN) != 0 || ((numbers & NUM_ROMAN_CAPITALS) != 0 && (wflags & FLAG_ALL_UPPER) != 0))
                    && ((wflags & FLAG_LAST_WORD) != 0 || (wt(wtab, 1).flags & FLAG_NOSPACE) == 0)
                {
                    found = self.translate_roman(tr, w, word1, &mut phonemes, wtab);
                    if found {
                        dictionary_flags[0] |= FLAG_ABBREV;
                    }
                }
            }

            if (wflags & FLAG_ALL_UPPER) != 0 && word_length > 1 && ucd::isalpha(first_char) {
                let t = self.t(tr);
                if (self.option_tone_flags & OPTION_EMPHASIZE_ALLCAPS) != 0 && (dictionary_flags[0] & FLAG_ABBREV) == 0 {
                    emphasize_allcaps = FLAG_EMPHASIZED;
                } else if !found && (dictionary_flags[0] & FLAG_SKIPWORDS) == 0 && word_length < 4 && t.clause_lower_count > 3 && t.clause_upper_count <= t.clause_lower_count {
                    spell_word = 1;
                }
            }
        }

        if spell_word > 0 {
            phonemes[0] = 0;
            if self.speak_individual_letters(tr, w, word1, &mut phonemes, spell_word).is_none() {
                if word_length > 1 {
                    return FLAG_SPELLWORD;
                }
                return 0;
            }
            cstrcpy(&mut self.word_phonemes, 0, cstr(&phonemes, 0));
            if wflags & FLAG_TRANSLATOR2 != 0 {
                return 0;
            }
            self.add_plural_suffixes(wflags, tr, last_char);
            return dictionary_flags[0] & FLAG_SKIPWORDS;
        } else if !found {
            let mut posn = 0;
            let mut non_initial = false;
            let mut length: i32 = 999;
            wordx = word1;
            while (length < 3 && length > 0) || (word_length > 1 && self.unpronouncable(tr, w, wordx, posn)) {
                was_unpronouncable = FLAG_WAS_UNPRONOUNCABLE;
                emphasize_allcaps = 0;
                if at(w, wordx) == b'\'' {
                    break;
                }
                if posn > 0 {
                    non_initial = true;
                }
                wordx += self.translate_letter(tr, w, wordx, &mut unpron_phonemes, non_initial as i32);
                posn += 1;
                if unpron_phonemes[0] == PHON_SWITCH {
                    self.set_word_phonemes(&unpron_phonemes);
                    if cstr(&unpron_phonemes, 1) == ESPEAKNG_DEFAULT_VOICE {
                        return FLAG_SPELLWORD;
                    }
                    return 0;
                }
                length = 0;
                while at(w, wordx + length as isize) != b' ' && (wordx + length as isize) < w.len() as isize {
                    length += 1;
                }
            }
            self.set_spelling_stress(tr, &mut unpron_phonemes, 0, posn);

            if at(w, wordx) != b' ' {
                if unpron_phonemes[0] != 0 && at(w, wordx) != b'\'' {
                    put(w, wordx - 1, b' ');
                }

                end_type = self.translate_rules(tr, w, wordx, &mut phonemes, N_WORD_PHONEMES, Some(&mut end_phonemes), wflags, Some(&mut dictionary_flags));

                if phonemes[0] == PHON_SWITCH {
                    self.set_word_phonemes(&phonemes);
                    return 0;
                }

                if phonemes[0] == 0 && end_phonemes[0] == 0 {
                    let wc = utf8_in(w, wordx).0;
                    if word_length == 1 && (is_alpha(wc) || is_superscript(wc) != 0) {
                        if self.speak_individual_letters(tr, w, wordx, &mut phonemes, spell_word).is_none() {
                            return 0;
                        }
                        cstrcpy(&mut self.word_phonemes, 0, cstr(&phonemes, 0));
                        return 0;
                    }
                }

                let mut c_temp = at(w, wordx - 1);
                found = false;
                let mut confirm_prefix = true;
                let mut loopcount = 0;
                while loopcount < 50 && (end_type & SUFX_P) != 0 {
                    loopcount += 1;
                    if confirm_prefix && (end_type & SUFX_B) == 0 {
                        let mut end_phonemes2b = [0u8; N_WORD_PHONEMES];
                        phonemes2[0] = 0;
                        let end2 = self.translate_rules(tr, w, wordx, &mut phonemes2, N_WORD_PHONEMES, Some(&mut end_phonemes2b), wflags | FLAG_NO_PREFIX | FLAG_NO_TRACE, Some(&mut dictionary_flags));
                        if end2 != 0 {
                            self.remove_ending(tr, w, wordx, end2, Some(&mut word_copy));
                            end_type = self.translate_rules(tr, w, wordx, &mut phonemes, N_WORD_PHONEMES, Some(&mut end_phonemes), wflags | FLAG_NO_TRACE, Some(&mut dictionary_flags));
                            let n = cstrlen(&word_copy, 0);
                            memcpy(w, wordx, &word_copy, 0, n);
                            if (end_type & SUFX_P) == 0 {
                                end_type = end2;
                                phonemes = phonemes2;
                                end_phonemes = end_phonemes2b;
                            }
                            confirm_prefix = false;
                            continue;
                        }
                    }

                    prefix_type = end_type;
                    if prefix_type & SUFX_V != 0 {
                        self.tm(tr).expect_verb = 1;
                    }
                    put(w, wordx - 1, c_temp);

                    if (prefix_type & SUFX_B) == 0 {
                        for _ in 0..(prefix_type & 0xf) {
                            wordx += 1;
                            while (at(w, wordx) & 0xc0) == 0x80 {
                                wordx += 1;
                            }
                        }
                    } else {
                        let mut pfix = 1usize;
                        prefix_chars[0] = 0;
                        let n_chars = (prefix_type & 0x3f) as usize;
                        for ix in 0..n_chars {
                            prefix_chars[pfix] = at(w, wordx);
                            pfix += 1;
                            wordx += 1;
                            if ix == n_chars - 1 {
                                prefix_chars[pfix - 1] = 0;
                            }
                        }
                        prefix_chars[pfix] = 0;
                    }
                    c_temp = at(w, wordx - 1);
                    put(w, wordx - 1, b' ');
                    confirm_prefix = true;
                    wflags |= FLAG_PREFIX_REMOVED;

                    if prefix_type & SUFX_B != 0 {
                        let mut prefix_phonemes2 = [0u8; 12];
                        strncpy0(&mut prefix_phonemes2, &end_phonemes, 12);
                        prefix_phonemes = phonemes;
                        found = self.lookup_dict_list(tr, &prefix_chars, 1, false, &mut phonemes, &mut dictionary_flags, 0, Some(wtab), None);
                        if found {
                            prefix_phonemes = phonemes;
                        }
                        if dictionary_flags[0] & FLAG_ABBREV != 0 {
                            prefix_phonemes[0] = 0;
                            let mut pc = prefix_chars;
                            self.speak_individual_letters(tr, &mut pc, 1, &mut prefix_phonemes, 1);
                        }
                    } else {
                        cstrcat(&mut prefix_phonemes, cstr(&end_phonemes, 0));
                    }
                    end_phonemes[0] = 0;

                    end_type = 0;
                    found = self.lookup_dict_list(tr, w, wordx, in_clause, &mut phonemes, &mut dictionary_flags2, SUFX_P, Some(wtab), None);
                    if dictionary_flags[0] == 0 {
                        dictionary_flags = dictionary_flags2;
                    } else {
                        prefix_flags = 1;
                    }
                    if !found {
                        end_type = self.translate_rules(tr, w, wordx, &mut phonemes, N_WORD_PHONEMES, Some(&mut end_phonemes), wflags & (FLAG_HYPHEN_AFTER | FLAG_PREFIX_REMOVED), Some(&mut dictionary_flags));
                        if phonemes[0] == PHON_SWITCH {
                            put(w, wordx - 1, c_temp);
                            self.set_word_phonemes(&phonemes);
                            return 0;
                        }
                    }
                }

                if end_type != 0 && (end_type & SUFX_P) == 0 {
                    end_type1 = end_type;
                    phonemes2 = phonemes;
                    let mut end_flags = self.remove_ending(tr, w, wordx, end_type, Some(&mut word_copy));
                    let mut more_suffixes = true;
                    let mut guard = 0;
                    while more_suffixes && guard < 100 {
                        guard += 1;
                        more_suffixes = false;
                        phonemes[0] = 0;
                        if prefix_phonemes[0] != 0 {
                            put(w, wordx - 1, c_temp);
                            found = self.lookup_dict_list(tr, w, word1, in_clause, &mut phonemes, &mut dictionary_flags2, end_flags, Some(wtab), None);
                            put(w, wordx - 1, b' ');
                            if phonemes[0] == PHON_SWITCH {
                                let n = cstrlen(&word_copy, 0);
                                memcpy(w, wordx, &word_copy, 0, n);
                                self.set_word_phonemes(&phonemes);
                                return 0;
                            }
                            if dictionary_flags[0] == 0 {
                                dictionary_flags = dictionary_flags2;
                            }
                            if found {
                                prefix_phonemes[0] = 0;
                            }
                            if !found && dictionary_flags2[0] != 0 {
                                prefix_flags = 1;
                            }
                        }
                        if !found {
                            found = self.lookup_dict_list(tr, w, wordx, in_clause, &mut phonemes, &mut dictionary_flags2, end_flags, Some(wtab), None);
                            if phonemes[0] == PHON_SWITCH {
                                let n = cstrlen(&word_copy, 0);
                                memcpy(w, wordx, &word_copy, 0, n);
                                self.set_word_phonemes(&phonemes);
                                return 0;
                            }
                            if dictionary_flags[0] == 0 {
                                dictionary_flags = dictionary_flags2;
                            }
                        }
                        if !found {
                            if end_type & SUFX_Q != 0 {
                                phonemes = phonemes2;
                            } else {
                                if end_flags & FLAG_SUFX != 0 {
                                    wflags |= FLAG_SUFFIX_REMOVED;
                                }
                                if end_type & SUFX_A != 0 {
                                    wflags |= FLAG_SUFFIX_VOWEL;
                                }
                                if end_type & SUFX_M != 0 {
                                    let end_phonemes2 = end_phonemes;
                                    end_type = self.translate_rules(tr, w, wordx, &mut phonemes, N_WORD_PHONEMES, Some(&mut end_phonemes), wflags, Some(&mut dictionary_flags));
                                    cstrcat(&mut end_phonemes, cstr(&end_phonemes2, 0));
                                    if end_type != 0 && (end_type & SUFX_P) == 0 {
                                        end_flags = self.remove_ending(tr, w, wordx, end_type, None);
                                        more_suffixes = true;
                                    }
                                } else {
                                    self.translate_rules(tr, w, wordx, &mut phonemes, N_WORD_PHONEMES, None, wflags, Some(&mut dictionary_flags));
                                    end_type = 0;
                                }
                                if phonemes[0] == PHON_SWITCH {
                                    self.set_word_phonemes(&phonemes);
                                    let n = cstrlen(&word_copy, 0);
                                    memcpy(w, wordx, &word_copy, 0, n);
                                    put(w, wordx - 1, c_temp);
                                    return 0;
                                }
                            }
                        }
                    }

                    if (end_type1 & SUFX_T) == 0 {
                        let ep = end_phonemes;
                        self.append_phonemes(tr, Some(&mut phonemes), N_WORD_PHONEMES, &ep);
                        end_phonemes[0] = 0;
                    }
                    let n = cstrlen(&word_copy, 0);
                    memcpy(w, wordx, &word_copy, 0, n);
                }
                put(w, wordx - 1, c_temp);
            }
        }

        self.add_plural_suffixes(wflags, tr, last_char);
        wflags |= emphasize_allcaps;

        let add_suffix_phonemes = if end_phonemes[0] != 0 { 2 } else { 0 };

        let mut prefix_stress = 0u8;
        for &p in cstr(&prefix_phonemes, 0) {
            if p == PHON_STRESS_P || p == PHON_STRESS_P2 {
                prefix_stress = p;
            }
        }
        if prefix_flags != 0 || prefix_stress != 0 {
            if self.t(tr).langopts.param[LOPT_PREFIXES] != 0 || (prefix_type & SUFX_T) != 0 {
                self.set_word_stress(tr, &mut phonemes, 0, Some(dictionary_flags), 3, 0);
                let mut ix = 0;
                let n = cstrlen(&prefix_phonemes, 0);
                for p in prefix_phonemes[..n].iter_mut() {
                    if *p == PHON_STRESS_P {
                        if ix == 0 {
                            ix = 1;
                        } else {
                            *p = PHON_STRESS_3;
                        }
                    }
                }
                let mut wp = self.word_phonemes;
                concat3(&mut wp, &unpron_phonemes, &prefix_phonemes, &phonemes);
                wp[N_WORD_PHONEMES - 1] = 0;
                self.set_word_stress(tr, &mut wp, 0, Some(dictionary_flags), -1, 0);
                self.word_phonemes = wp;
            } else {
                let mut wp = self.word_phonemes;
                concat3(&mut wp, &unpron_phonemes, &prefix_phonemes, &phonemes);
                wp[N_WORD_PHONEMES - 1] = 0;
                self.set_word_stress(tr, &mut wp, 0, Some(dictionary_flags), -1, 0);
                self.word_phonemes = wp;
            }
        } else {
            self.set_word_stress(tr, &mut phonemes, 0, Some(dictionary_flags), -1, add_suffix_phonemes);
            let mut wp = self.word_phonemes;
            concat3(&mut wp, &unpron_phonemes, &prefix_phonemes, &phonemes);
            wp[N_WORD_PHONEMES - 1] = 0;
            self.word_phonemes = wp;
        }

        if end_phonemes[0] != 0 {
            let ix = cstrlen(&self.word_phonemes, 0);
            let cut = N_WORD_PHONEMES - 1 - ix;
            end_phonemes[cut] = 0;
            let ep = end_phonemes;
            cstrcpy(&mut self.word_phonemes, ix as isize, cstr(&ep, 0));
        }

        if wflags & FLAG_LAST_WORD != 0 {
            dictionary_flags[0] &= !FLAG_PAUSE1;
        }

        if (wflags & FLAG_HYPHEN) != 0 && (self.t(tr).langopts.stress_flags & S_HYPEN_UNSTRESS) != 0 {
            self.change_word_stress(tr, 3);
        } else if wflags & FLAG_EMPHASIZED2 != 0 {
            self.change_word_stress(tr, 6);
            if wflags & FLAG_EMPHASIZED != 0 {
                dictionary_flags[0] |= FLAG_PAUSE1;
            }
        } else if wt(wtab, self.dictionary_skipwords.max(0) as usize).flags & FLAG_LAST_WORD != 0 {
            if dictionary_flags[0] & (FLAG_STRESS_END | FLAG_STRESS_END2) != 0 {
                self.change_word_stress(tr, 4);
            } else if dictionary_flags[0] & FLAG_UNSTRESS_END != 0 {
                // C tests the `any_stressed_words` pointer, not its value, so this is always true
                self.change_word_stress(tr, 3);
            }
        }

        if end_type1 & SUFX_F != 0 {
            self.tm(tr).expect_verb = 2;
            self.tm(tr).expect_verb_s = 2;
        }

        {
            let t = self.tm(tr);
            if dictionary_flags[1] & FLAG_PASTF != 0 {
                t.expect_past = 3;
                t.expect_verb = 0;
                t.expect_noun = 0;
            } else if dictionary_flags[1] & FLAG_VERBF != 0 {
                t.expect_verb = 2;
                t.expect_verb_s = 0;
                t.expect_noun = 0;
            } else if dictionary_flags[1] & FLAG_VERBSF != 0 {
                t.expect_verb = 0;
                t.expect_verb_s = 2;
                t.expect_past = 0;
                t.expect_noun = 0;
            } else if dictionary_flags[1] & FLAG_NOUNF != 0 {
                t.expect_noun = 2;
                t.expect_verb = 0;
                t.expect_verb_s = 0;
                t.expect_past = 0;
            }
        }

        if at(w, wordx) != 0 && (dictionary_flags[1] & FLAG_VERB_EXT) == 0 {
            let t = self.tm(tr);
            if t.expect_verb > 0 {
                t.expect_verb -= 1;
            }
            if t.expect_verb_s > 0 {
                t.expect_verb_s -= 1;
            }
            if t.expect_noun > 0 {
                t.expect_noun -= 1;
            }
            if t.expect_past > 0 {
                t.expect_past -= 1;
            }
        }

        if word_length == 1 && self.t(tr).translator_name == l2(b'e', b'n') && ucd::isalpha(first_char) && first_char != b'i' as i32 {
            dictionary_flags[0] |= FLAG_ALLOW_DOT;
        }

        if (self.t(tr).langopts.param[LOPT_ALT] & 2) != 0 && (dictionary_flags[0] & (FLAG_ALT_TRANS | FLAG_ALT2_TRANS)) != 0 {
            self.apply_special_attribute2(tr, dictionary_flags[0]);
        }

        dictionary_flags[0] |= was_unpronouncable;
        memcpy(w, word_start, &word_copy2, 0, word_copy_length);
        dictionary_flags[0]
    }

    /// `ApplySpecialAttribute2`, on `word_phonemes`.
    fn apply_special_attribute2(&mut self, tr: Tr, dict_flags: u32) {
        let len = cstrlen(&self.word_phonemes, 0);
        if self.t(tr).langopts.param[LOPT_ALT] & 2 == 0 {
            return;
        }
        let e = self.phoneme_code(b'E' as u32) as u8;
        let ee = self.phoneme_code(b'e' as u32) as u8;
        let o = self.phoneme_code(b'O' as u32) as u8;
        let oo = self.phoneme_code(b'o' as u32) as u8;
        for ix in 0..len.saturating_sub(1) {
            if self.word_phonemes[ix] == PHON_STRESS_P {
                let p = &mut self.word_phonemes[ix + 1];
                if dict_flags & FLAG_ALT2_TRANS != 0 {
                    if *p == e {
                        *p = ee;
                    }
                    if *p == o {
                        *p = oo;
                    }
                } else {
                    if *p == ee {
                        *p = e;
                    }
                    if *p == oo {
                        *p = o;
                    }
                }
                break;
            }
        }
    }

    /// `ChangeWordStress`, on `word_phonemes`.
    fn change_word_stress(&mut self, tr: Tr, new_stress: i32) {
        let mut phonetic = [0u8; N_WORD_PHONEMES];
        let mut vowel_stress = [0i8; N_WORD_PHONEMES / 2];
        let mut vowel_count = 0;
        let mut stressed_syllable = 0;
        cstrcpy(&mut phonetic, 0, cstr(&self.word_phonemes, 0));
        let max_stress = self.get_vowel_stress(tr, &mut phonetic, &mut vowel_stress, &mut vowel_count, &mut stressed_syllable, 0);

        if new_stress >= STRESS_IS_PRIMARY {
            for ix in 1..vowel_count.min(100) as usize {
                if vowel_stress[ix] as i32 >= max_stress {
                    vowel_stress[ix] = new_stress as i8;
                    break;
                }
            }
        } else {
            for ix in 1..vowel_count.min(100) as usize {
                if vowel_stress[ix] as i32 > new_stress {
                    vowel_stress[ix] = new_stress as i8;
                }
            }
        }

        let mut out = Vec::with_capacity(N_WORD_PHONEMES);
        let mut ix = 1usize;
        for &p in cstr(&phonetic, 0) {
            let ph = self.phz(p);
            if ph.typ == PH_VOWEL && (ph.phflags & PH_NONSYLLABIC) == 0 {
                let vs = vowel_stress.get(ix).copied().unwrap_or(0) as i32;
                if vs == STRESS_IS_DIMINISHED || vs > STRESS_IS_UNSTRESSED {
                    out.push(STRESS_PHONEMES.get(vs as usize).copied().unwrap_or(0));
                }
                ix += 1;
            }
            out.push(p);
        }
        cstrcpy(&mut self.word_phonemes, 0, &out);
    }

    /// `SpeakIndividualLetters`: appends the names of the letters at `w[word..]` to `phonemes`.
    /// Returns the position after the word, or `None` when a letter needs another language.
    pub(crate) fn speak_individual_letters(&mut self, tr: Tr, w: &mut [u8], mut word: isize, phonemes: &mut [u8], spell_word: i32) -> Option<isize> {
        let mut posn = 0;
        let mut capitals = 0;
        let mut non_initial = 0;
        if spell_word > 2 {
            capitals = 2;
        }
        if spell_word > 1 {
            capitals |= 4;
        }
        while at(w, word) != b' ' && at(w, word) != 0 {
            let n = self.translate_letter(tr, w, word, phonemes, capitals | non_initial);
            word += n;
            posn += 1;
            non_initial = 1;
            if phonemes[0] == PHON_SWITCH {
                cstrcpy(&mut self.word_phonemes, 0, cstr(phonemes, 0));
                return None;
            }
            if n == 0 {
                word += 1;
            }
        }
        self.set_spelling_stress(tr, phonemes, spell_word, posn);
        Some(word)
    }

    /// `TranslateLetter`: appends the name of the letter at `w[word]` to `phonemes`.
    /// Returns the number of bytes in the letter.
    pub(crate) fn translate_letter(&mut self, tr: Tr, w: &[u8], word: isize, phonemes: &mut [u8], control: i32) -> isize {
        let mut capital = [0u8; PH_BUF];
        let mut ph_buf = [0u8; PH_BUF];
        let mut ph_buf2 = [0u8; PH_BUF];
        let mut ph_alphabet = [0u8; PH_BUF];
        let mut phontab_1 = self.t(Tr::Main).phoneme_tab_ix;
        const PAUSE_STRING: [u8; 2] = [PHON_PAUSE, 0];

        let (mut letter, n_bytes) = utf8_in(w, word);
        if (letter & 0xfff00) == 0x0e000 {
            letter &= 0xff;
        }
        if (control & 2) != 0 && ucd::isupper(letter) {
            self.lookup(tr, b"_cap\0", &mut capital);
        }
        letter = crate::readclause::towlower2(letter, self.t(tr));
        let next = at(w, word + n_bytes) as i8 as i32;
        self.lookup_letter(tr, letter, next, &mut ph_buf, 0, control & 1);

        if ph_buf[0] == 0 {
            let c = is_superscript(letter);
            if c != 0 {
                letter = c & 0x3fff;
                if (control & 4) != 0 {
                    if let Some(modifier) = MODIFIERS[(c >> 14) as usize & 3] {
                        self.lookup(tr, modifier, &mut capital);
                        if capital[0] == 0 {
                            capital[2] = self.set_translator3(ESPEAKNG_DEFAULT_VOICE) as u8;
                            let mut tmp = [0u8; PH_BUF];
                            self.lookup(Tr::Third, modifier, &mut tmp);
                            cstrcpy(&mut capital, 3, cstr(&tmp, 0));
                            if capital[3] != 0 {
                                capital[0] = PHON_PAUSE;
                                capital[1] = PHON_SWITCH;
                                let len = cstrlen(&capital, 3);
                                if len + 5 < capital.len() {
                                    capital[len + 3] = PHON_SWITCH;
                                    capital[len + 4] = phontab_1 as u8;
                                    capital[len + 5] = 0;
                                }
                            }
                        }
                    }
                }
            }
            self.lookup_letter(tr, letter, next, &mut ph_buf, 0, control & 1);
        }

        if ph_buf[0] == PHON_SWITCH {
            cstrcpy(phonemes, 0, cstr(&ph_buf, 0));
            return 0;
        }

        if ph_buf[0] == 0 {
            let number = non_ascii_number(letter);
            if number > 0 {
                self.lookup_letter(tr, number, 0, &mut ph_buf, 0, control & 1);
            }
        }

        let mut al_offset = 0;
        let mut al_flags = 0;
        let alphabet = alphabet_from_char(letter);
        if let Some(a) = alphabet {
            al_offset = a.offset;
            al_flags = a.flags;
        }

        // C compares with its own copy of current_alphabet, which is always NULL here
        if let Some(a) = alphabet {
            if (al_flags & AL_DONT_NAME) == 0 && al_offset != self.t(Tr::Main).letter_bits_offset {
                let main = self.t(Tr::Main);
                if (al_flags & AL_DONT_NAME) != 0 || al_offset == main.langopts.alt_alphabet || al_offset == main.langopts.our_alphabet {
                } else {
                    ph_buf2[0] = 0;
                    let mut name = a.name.to_vec();
                    name.push(0);
                    if self.lookup(Tr::Main, &name, &mut ph_alphabet) == 0 {
                        ph_alphabet[2] = self.set_translator3(ESPEAKNG_DEFAULT_VOICE) as u8;
                        self.lookup(Tr::Third, &name, &mut ph_buf2);
                    } else if tr != Tr::Main {
                        phontab_1 = self.t(tr).phoneme_tab_ix;
                        let pa = ph_alphabet;
                        cstrcpy(&mut ph_buf2, 0, cstr(&pa, 0));
                        ph_alphabet[2] = self.t(Tr::Main).phoneme_tab_ix as u8;
                    }
                    if ph_buf2[0] != 0 {
                        ph_alphabet[0] = PHON_PAUSE;
                        ph_alphabet[1] = PHON_SWITCH;
                        let b2 = cstr(&ph_buf2, 0).to_vec();
                        cstrcpy(&mut ph_alphabet, 3, &b2);
                        let len = b2.len() + 3;
                        if len + 2 < ph_alphabet.len() {
                            ph_alphabet[len] = PHON_SWITCH;
                            ph_alphabet[len + 1] = phontab_1 as u8;
                            ph_alphabet[len + 2] = 0;
                        }
                    }
                }
            }
        }

        if ph_buf[0] == 0 {
            let main = self.t(Tr::Main);
            let language = if al_offset != 0 && al_offset == main.langopts.alt_alphabet {
                main.langopts.alt_alphabet_lang
            } else if let Some(a) = alphabet.filter(|a| a.language != 0 && (al_flags & AL_NOT_LETTERS) == 0) {
                a.language
            } else {
                l2(b'e', b'n')
            };

            if language != self.t(tr).translator_name || language == l2(b'k', b'o') {
                ph_buf[2] = self.set_translator3(&word_to_string2(language)) as u8;
                if self.exists(Tr::Third) {
                    let code = letter - 0xac00;
                    if code >= 0 && letter <= 0xd7af {
                        let mut hangul_buf = [0u8; 12];
                        hangul_buf[0] = b' ';
                        let mut p3 = 1isize;
                        let initial = (code / 28) / 21;
                        if initial != 11 {
                            p3 += utf8_out(initial + 0x1100, &mut hangul_buf, p3);
                        }
                        utf8_out(((code / 28) % 21) + 0x1161, &mut hangul_buf, p3);
                        utf8_out((code % 28) + 0x11a7, &mut hangul_buf, p3 + 3);
                        put(&mut hangul_buf, p3 + 6, b' ');
                        put(&mut hangul_buf, p3 + 7, 0);
                        ph_buf[3] = 0;
                        let mut tmp = [0u8; 77];
                        self.translate_rules(Tr::Third, &mut hangul_buf, 1, &mut tmp, 77, None, 0, None);
                        self.set_word_stress(Tr::Third, &mut tmp, 0, None, -1, 0);
                        cstrcpy(&mut ph_buf, 3, cstr(&tmp, 0));
                    } else {
                        self.lookup_letter(Tr::Third, letter, next, &mut ph_buf, 3, control & 1);
                    }

                    if ph_buf[3] == PHON_SWITCH {
                        let lang = cstr(&ph_buf, 4).to_vec();
                        ph_buf[2] = self.set_translator3(&lang) as u8;
                        self.lookup_letter(Tr::Third, letter, next, &mut ph_buf, 3, control & 1);
                    }

                    let ix = self.voice_phoneme_tab_ix;
                    self.select_phoneme_table(ix);

                    if ph_buf[3] != 0 {
                        ph_buf[0] = PHON_PAUSE;
                        ph_buf[1] = PHON_SWITCH;
                        let len = cstrlen(&ph_buf, 3) + 3;
                        if len + 2 < ph_buf.len() {
                            ph_buf[len] = PHON_SWITCH;
                            ph_buf[len + 1] = self.t(tr).phoneme_tab_ix as u8;
                            ph_buf[len + 2] = 0;
                        }
                    }
                }
            }
        }

        if ph_buf[0] == 0 {
            let mut speak_letter_number = true;
            if (al_flags & AL_NO_SYMBOL) == 0 {
                if ucd::isalpha(letter) {
                    self.lookup(Tr::Main, b"_?A\0", &mut ph_buf);
                }
                if ph_buf[0] == 0 && !ucd::isspace(letter) {
                    self.lookup(Tr::Main, b"_??\0", &mut ph_buf);
                }
                if ph_buf[0] == 0 {
                    self.encode_phonemes(b"l'et@\0", 0, &mut ph_buf, 0);
                }
            }
            if (control & 4) == 0 && (al_flags & AL_NOT_CODE) != 0 {
                speak_letter_number = false;
            }
            if speak_letter_number {
                let hexbuf: Vec<u8> = if al_offset == 0x2800 {
                    (0..8).filter(|ix| letter & (1 << ix) != 0).map(|ix| b'1' + ix as u8).collect()
                } else {
                    format!("{:x}", letter as u32).into_bytes()
                };
                let mut pbuf = 0usize;
                for &h in &hexbuf {
                    pbuf += cstrlen(&ph_buf, pbuf as isize);
                    if pbuf + 1 >= ph_buf.len() {
                        break;
                    }
                    ph_buf[pbuf] = PHON_PAUSE_VSHORT;
                    pbuf += 1;
                    self.lookup_letter(Tr::Main, h as i32, 0, &mut ph_buf, pbuf, 1);
                    if (ph_buf[pbuf] == 0 || ph_buf[pbuf] == PHON_SWITCH) && h >= b'a' {
                        self.encode_phonemes(HEX_LETTERS[(h - b'a') as usize], 0, &mut ph_buf[pbuf..], 0);
                    }
                }
                let n = cstrlen(&ph_buf, pbuf as isize);
                cstrcpy(&mut ph_buf, (pbuf + n) as isize, &PAUSE_STRING[..1]);
            }
        }

        let len = cstrlen(phonemes, 0);
        let mut v = vec![0xffu8];
        if self.t(tr).langopts.accents & 2 != 0 {
            v.extend_from_slice(cstr(&ph_alphabet, 0));
            v.extend_from_slice(cstr(&ph_buf, 0));
            v.extend_from_slice(cstr(&capital, 0));
        } else {
            v.extend_from_slice(cstr(&ph_alphabet, 0));
            v.extend_from_slice(cstr(&capital, 0));
            v.extend_from_slice(cstr(&ph_buf, 0));
        }
        if len + v.len() < N_WORD_PHONEMES {
            cstrcpy(phonemes, len as isize, &v);
        }
        n_bytes
    }

    /// `addPluralSuffixes`, appending to `word_phonemes`.
    fn add_plural_suffixes(&mut self, flags: u32, tr: Tr, last_char: i32) {
        if flags & FLAG_HAS_PLURAL == 0 {
            return;
        }
        let last_char = last_char as i8 as i32;
        let mut word: [u8; 4] = if last_char == b'f' as i32 {
            [b' ', b's', b's', 0]
        } else if last_char == 0 || !(last_char < 0x80 && last_char > 0 && b"hsx".contains(&(last_char as u8))) {
            [b' ', b'z', b'z', 0]
        } else {
            [b' ', b'i', b'z', 0]
        };
        let mut wp = self.word_phonemes;
        self.translate_rules(tr, &mut word, 1, &mut wp, N_WORD_PHONEMES, None, 0, None);
        self.word_phonemes = wp;
    }

    /// `CheckDottedAbbrev`: "u.s.a" spelled as letters.
    fn check_dotted_abbrev(&mut self, w: &mut [u8], word1: isize) -> i32 {
        let mut count = 0;
        let mut word = word1;
        let mut word_buf = Vec::with_capacity(80);
        loop {
            let mut ok = 0;
            let (wc, n) = utf8_in(w, word);
            let mut nbytes = n;
            if at(w, word + nbytes) == b' ' && is_alpha(wc) {
                if at(w, word + nbytes + 1) == b'.' {
                    if at(w, word + nbytes + 2) == b' ' {
                        ok = 1;
                    } else if at(w, word + nbytes + 2) == b'\'' && at(w, word + nbytes + 3) == b's' {
                        nbytes += 2;
                        ok = 2;
                    }
                } else if count > 0 {
                    ok = 2;
                }
            }
            if ok == 0 {
                break;
            }
            for ix in 0..nbytes {
                if word_buf.len() < 80 {
                    word_buf.push(at(w, word + ix));
                }
            }
            count += 1;
            if ok == 2 {
                word += nbytes;
                break;
            }
            word += nbytes + 3;
        }
        if count > 1 {
            let ix = word_buf.len();
            for (k, &b) in word_buf.iter().enumerate() {
                put(w, word1 + k as isize, b);
            }
            let mut k = ix as isize;
            while word1 + k < word {
                put(w, word1 + k, b' ');
                k += 1;
            }
            self.dictionary_skipwords = (count - 1) * 2;
        }
        count
    }

    /// `Unpronouncable`: should the start of this word be spelled?
    fn unpronouncable(&mut self, tr: Tr, w: &mut [u8], word: isize, posn: i32) -> bool {
        let mut c1 = 0;
        let mut vowel_posn = 9;
        let c = utf8_in(w, word).0;
        let t = self.t(tr);
        if t.letter_bits_offset > 0 && c < 0x241 {
            return false;
        }
        if let Some(a) = alphabet_from_char(c) {
            if a.offset != t.letter_bits_offset {
                return false;
            }
        }
        let unpron = t.langopts.param[LOPT_UNPRONOUNCABLE];
        if unpron == 1 {
            return false;
        }
        let c0 = at(w, word);
        if c0 == b' ' || c0 == 0 || c0 == b'\'' {
            return false;
        }
        let mut index = 0isize;
        let mut count = 0;
        let mut c;
        loop {
            let (cc, n) = utf8_in(w, word + index);
            c = cc;
            index += n;
            if c == 0 || c == b' ' as i32 {
                break;
            }
            if c == b'\'' as i32 && (count > 1 || posn > 0) {
                break;
            }
            if count == 0 {
                c1 = c;
            }
            if !(c == b'\'' as i32 && unpron == 3) {
                count += 1;
            }
            if self.is_vowel(tr, c) {
                vowel_posn = count;
                break;
            }
            if c != b'\'' as i32 && !ucd::isalpha(c) {
                return false;
            }
        }
        if vowel_posn > 2 && unpron == 2 {
            return self.unpronouncable2(tr, w, word);
        }
        if c1 == unpron {
            vowel_posn -= 1;
        }
        vowel_posn > self.t(tr).langopts.max_initial_consonants + 1
    }

    fn unpronouncable2(&mut self, tr: Tr, w: &mut [u8], word: isize) -> bool {
        let mut ph_buf = [0u8; N_WORD_PHONEMES];
        let c = at(w, word - 1);
        put(w, word - 1, b' ');
        let end_flags = self.translate_rules(tr, w, word, &mut ph_buf, N_WORD_PHONEMES, None, FLAG_UNPRON_TEST, None);
        put(w, word - 1, c);
        end_flags == 0 || (end_flags & SUFX_UNPRON) != 0
    }
}
