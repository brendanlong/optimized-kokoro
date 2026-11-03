//! numbers.c: letter names, accented letters, Roman numerals and numbers.

use crate::common::*;
use crate::defs::*;
use crate::engine::{Espeak, Tr};
use crate::ucd;

const M_LIGATURE: u16 = 0x8000;
const M_NAME: u16 = 0;
const M_SMALLCAP: u16 = 1;
const M_TURNED: u16 = 2;
const M_REVERSED: u16 = 3;
const M_CURL: u16 = 4;
const M_ACUTE: u16 = 5;
const M_BREVE: u16 = 6;
const M_CARON: u16 = 7;
const M_CEDILLA: u16 = 8;
const M_CIRCUMFLEX: u16 = 9;
const M_DIAERESIS: u16 = 10;
const M_DOUBLE_ACUTE: u16 = 11;
const M_DOT_ABOVE: u16 = 12;
const M_GRAVE: u16 = 13;
const M_MACRON: u16 = 14;
const M_OGONEK: u16 = 15;
const M_RING: u16 = 16;
const M_STROKE: u16 = 17;
const M_TILDE: u16 = 18;
const M_BAR: u16 = 19;
const M_RETROFLEX: u16 = 20;
const M_HOOK: u16 = 21;
const M_MIDDLE_DOT: u16 = M_DOT_ABOVE;
const M_IMPLOSIVE: u16 = M_HOOK;

/// `accents_tab`: names to look up in `*_list`, and whether to say them before the letter.
const ACCENTS_TAB: [(&[u8], i32); 22] = [
    (b"_lig", 1),
    (b"_smc", 0),
    (b"_tur", 0),
    (b"_rev", 0),
    (b"_crl", 0),
    (b"_acu", 0),
    (b"_brv", 0),
    (b"_hac", 0),
    (b"_ced", 0),
    (b"_cir", 0),
    (b"_dia", 0),
    (b"_ac2", 0),
    (b"_dot", 0),
    (b"_grv", 0),
    (b"_mcn", 0),
    (b"_ogo", 0),
    (b"_rng", 0),
    (b"_stk", 0),
    (b"_tld", 0),
    (b"_bar", 0),
    (b"_rfx", 0),
    (b"_hok", 0),
];

const CAPITAL: u16 = 0;

const fn letter(ch: u16, mod1: u16, mod2: u16) -> u16 {
    (ch - 59) + (mod1 << 6) + (mod2 << 11)
}

const fn ligature(ch1: u16, ch2: u16, mod1: u16) -> u16 {
    (ch1 - 59) + ((ch2 - 59) << 6) + (mod1 << 12) + M_LIGATURE
}

const L_ALPHA: u16 = 60;
const L_SCHWA: u16 = 61;
const L_OPEN_E: u16 = 62;
const L_GAMMA: u16 = 63;
const L_IOTA: u16 = 64;
const L_PHI: u16 = 67;
const L_ESH: u16 = 68;
const L_UPSILON: u16 = 69;
const L_EZH: u16 = 70;
const L_GLOTTAL: u16 = 71;
const L_RTAP: u16 = 72;
const L_RLONG: u16 = 73;

const NON_ASCII_TAB: [i32; 15] = [0, 0x3b1, 0x259, 0x25b, 0x3b3, 0x3b9, 0x153, 0x3c9, 0x3c6, 0x283, 0x3c5, 0x292, 0x294, 0x27e, 0x27c];

const fn c(ch: u8) -> u16 {
    ch as u16
}

#[rustfmt::skip]
static LETTER_ACCENTS_0E0: [u16; 0x17f - 0xe0 + 1] = [
    letter(c(b'a'), M_GRAVE, 0), letter(c(b'a'), M_ACUTE, 0), letter(c(b'a'), M_CIRCUMFLEX, 0), letter(c(b'a'), M_TILDE, 0),
    letter(c(b'a'), M_DIAERESIS, 0), letter(c(b'a'), M_RING, 0), ligature(c(b'a'), c(b'e'), 0), letter(c(b'c'), M_CEDILLA, 0),
    letter(c(b'e'), M_GRAVE, 0), letter(c(b'e'), M_ACUTE, 0), letter(c(b'e'), M_CIRCUMFLEX, 0), letter(c(b'e'), M_DIAERESIS, 0),
    letter(c(b'i'), M_GRAVE, 0), letter(c(b'i'), M_ACUTE, 0), letter(c(b'i'), M_CIRCUMFLEX, 0), letter(c(b'i'), M_DIAERESIS, 0),
    letter(c(b'd'), M_NAME, 0), letter(c(b'n'), M_TILDE, 0), letter(c(b'o'), M_GRAVE, 0), letter(c(b'o'), M_ACUTE, 0),
    letter(c(b'o'), M_CIRCUMFLEX, 0), letter(c(b'o'), M_TILDE, 0), letter(c(b'o'), M_DIAERESIS, 0), 0,
    letter(c(b'o'), M_STROKE, 0), letter(c(b'u'), M_GRAVE, 0), letter(c(b'u'), M_ACUTE, 0), letter(c(b'u'), M_CIRCUMFLEX, 0),
    letter(c(b'u'), M_DIAERESIS, 0), letter(c(b'y'), M_ACUTE, 0), letter(c(b't'), M_NAME, 0), letter(c(b'y'), M_DIAERESIS, 0),
    CAPITAL, letter(c(b'a'), M_MACRON, 0), CAPITAL, letter(c(b'a'), M_BREVE, 0), CAPITAL, letter(c(b'a'), M_OGONEK, 0), CAPITAL, letter(c(b'c'), M_ACUTE, 0),
    CAPITAL, letter(c(b'c'), M_CIRCUMFLEX, 0), CAPITAL, letter(c(b'c'), M_DOT_ABOVE, 0), CAPITAL, letter(c(b'c'), M_CARON, 0), CAPITAL, letter(c(b'd'), M_CARON, 0),
    CAPITAL, letter(c(b'd'), M_STROKE, 0), CAPITAL, letter(c(b'e'), M_MACRON, 0), CAPITAL, letter(c(b'e'), M_BREVE, 0), CAPITAL, letter(c(b'e'), M_DOT_ABOVE, 0),
    CAPITAL, letter(c(b'e'), M_OGONEK, 0), CAPITAL, letter(c(b'e'), M_CARON, 0), CAPITAL, letter(c(b'g'), M_CIRCUMFLEX, 0), CAPITAL, letter(c(b'g'), M_BREVE, 0),
    CAPITAL, letter(c(b'g'), M_DOT_ABOVE, 0), CAPITAL, letter(c(b'g'), M_CEDILLA, 0), CAPITAL, letter(c(b'h'), M_CIRCUMFLEX, 0), CAPITAL, letter(c(b'h'), M_STROKE, 0),
    CAPITAL, letter(c(b'i'), M_TILDE, 0), CAPITAL, letter(c(b'i'), M_MACRON, 0), CAPITAL, letter(c(b'i'), M_BREVE, 0), CAPITAL, letter(c(b'i'), M_OGONEK, 0),
    CAPITAL, letter(c(b'i'), M_NAME, 0), CAPITAL, ligature(c(b'i'), c(b'j'), 0), CAPITAL, letter(c(b'j'), M_CIRCUMFLEX, 0), CAPITAL, letter(c(b'k'), M_CEDILLA, 0),
    letter(c(b'k'), M_NAME, 0), CAPITAL, letter(c(b'l'), M_ACUTE, 0), CAPITAL, letter(c(b'l'), M_CEDILLA, 0), CAPITAL, letter(c(b'l'), M_CARON, 0), CAPITAL,
    letter(c(b'l'), M_MIDDLE_DOT, 0), CAPITAL, letter(c(b'l'), M_STROKE, 0), CAPITAL, letter(c(b'n'), M_ACUTE, 0), CAPITAL, letter(c(b'n'), M_CEDILLA, 0), CAPITAL,
    letter(c(b'n'), M_CARON, 0), letter(c(b'n'), M_NAME, 0), CAPITAL, letter(c(b'n'), M_NAME, 0), CAPITAL, letter(c(b'o'), M_MACRON, 0), CAPITAL, letter(c(b'o'), M_BREVE, 0),
    CAPITAL, letter(c(b'o'), M_DOUBLE_ACUTE, 0), CAPITAL, ligature(c(b'o'), c(b'e'), 0), CAPITAL, letter(c(b'r'), M_ACUTE, 0), CAPITAL, letter(c(b'r'), M_CEDILLA, 0),
    CAPITAL, letter(c(b'r'), M_CARON, 0), CAPITAL, letter(c(b's'), M_ACUTE, 0), CAPITAL, letter(c(b's'), M_CIRCUMFLEX, 0), CAPITAL, letter(c(b's'), M_CEDILLA, 0),
    CAPITAL, letter(c(b's'), M_CARON, 0), CAPITAL, letter(c(b't'), M_CEDILLA, 0), CAPITAL, letter(c(b't'), M_CARON, 0), CAPITAL, letter(c(b't'), M_STROKE, 0),
    CAPITAL, letter(c(b'u'), M_TILDE, 0), CAPITAL, letter(c(b'u'), M_MACRON, 0), CAPITAL, letter(c(b'u'), M_BREVE, 0), CAPITAL, letter(c(b'u'), M_RING, 0),
    CAPITAL, letter(c(b'u'), M_DOUBLE_ACUTE, 0), CAPITAL, letter(c(b'u'), M_OGONEK, 0), CAPITAL, letter(c(b'w'), M_CIRCUMFLEX, 0), CAPITAL, letter(c(b'y'), M_CIRCUMFLEX, 0),
    CAPITAL, CAPITAL, letter(c(b'z'), M_ACUTE, 0), CAPITAL, letter(c(b'z'), M_DOT_ABOVE, 0), CAPITAL, letter(c(b'z'), M_CARON, 0), letter(c(b's'), M_NAME, 0),
];

#[rustfmt::skip]
static LETTER_ACCENTS_250: [u16; 0x2a8 - 0x250 + 1] = [
    letter(c(b'a'), M_TURNED, 0), letter(L_ALPHA, 0, 0), letter(L_ALPHA, M_TURNED, 0), letter(c(b'b'), M_IMPLOSIVE, 0),
    0, letter(c(b'c'), M_CURL, 0), letter(c(b'd'), M_RETROFLEX, 0), letter(c(b'd'), M_IMPLOSIVE, 0),
    letter(c(b'e'), M_REVERSED, 0), 0, letter(L_SCHWA, M_HOOK, 0), 0,
    letter(L_OPEN_E, M_REVERSED, 0), letter(L_OPEN_E, M_HOOK, M_REVERSED), 0, letter(c(b'j'), M_BAR, 0),
    letter(c(b'g'), M_IMPLOSIVE, 0), letter(c(b'g'), 0, 0), letter(c(b'g'), M_SMALLCAP, 0), letter(L_GAMMA, 0, 0),
    0, letter(c(b'h'), M_TURNED, 0), letter(c(b'h'), M_HOOK, 0), 0,
    letter(c(b'i'), M_BAR, 0), letter(L_IOTA, 0, 0), letter(c(b'i'), M_SMALLCAP, 0), letter(c(b'l'), M_TILDE, 0),
    letter(c(b'l'), M_BAR, 0), letter(c(b'l'), M_RETROFLEX, 0), ligature(c(b'l'), c(b'z'), 0), letter(c(b'm'), M_TURNED, 0),
    0, letter(c(b'm'), M_HOOK, 0), 0, letter(c(b'n'), M_RETROFLEX, 0),
    letter(c(b'n'), M_SMALLCAP, 0), letter(c(b'o'), M_BAR, 0), ligature(c(b'o'), c(b'e'), M_SMALLCAP), 0,
    letter(L_PHI, 0, 0), letter(c(b'r'), M_TURNED, 0), letter(L_RLONG, M_TURNED, 0), letter(c(b'r'), M_RETROFLEX, M_TURNED),
    0, letter(c(b'r'), M_RETROFLEX, 0), 0, letter(L_RTAP, M_REVERSED, 0),
    letter(c(b'r'), M_SMALLCAP, 0), letter(c(b'r'), M_TURNED, M_SMALLCAP), letter(c(b's'), M_RETROFLEX, 0), 0,
    letter(c(b'j'), M_HOOK, 0), letter(L_ESH, M_REVERSED, 0), letter(L_ESH, M_CURL, 0), letter(c(b't'), M_TURNED, 0),
    letter(c(b't'), M_RETROFLEX, 0), letter(c(b'u'), M_BAR, 0), letter(L_UPSILON, 0, 0), letter(c(b'v'), M_HOOK, 0),
    letter(c(b'v'), M_TURNED, 0), letter(c(b'w'), M_TURNED, 0), letter(c(b'y'), M_TURNED, 0), letter(c(b'y'), M_SMALLCAP, 0),
    letter(c(b'z'), M_RETROFLEX, 0), letter(c(b'z'), M_CURL, 0), 0, letter(L_EZH, M_CURL, 0),
    0, letter(L_GLOTTAL, M_REVERSED, 0), letter(L_GLOTTAL, M_TURNED, 0), 0,
    0, letter(c(b'b'), M_SMALLCAP, 0), 0, letter(c(b'g'), M_IMPLOSIVE, M_SMALLCAP),
    letter(c(b'h'), M_SMALLCAP, 0), letter(c(b'j'), M_CURL, 0), letter(c(b'k'), M_TURNED, 0), letter(c(b'l'), M_SMALLCAP, 0),
    letter(c(b'q'), M_HOOK, 0), letter(L_GLOTTAL, M_STROKE, 0), letter(L_GLOTTAL, M_STROKE, M_REVERSED), ligature(c(b'd'), c(b'z'), 0),
    0, ligature(c(b'd'), c(b'z'), M_CURL), ligature(c(b't'), c(b's'), 0), 0,
    ligature(c(b't'), c(b's'), M_CURL),
];

const L_SUB: u16 = 0x4000;
const L_SUP: u16 = 0x8000;

#[rustfmt::skip]
static DERIVED_LETTERS: [(u16, u16); 62] = [
    (0x00aa, b'a' as u16 + L_SUP), (0x00b2, b'2' as u16 + L_SUP), (0x00b3, b'3' as u16 + L_SUP), (0x00b9, b'1' as u16 + L_SUP),
    (0x00ba, b'o' as u16 + L_SUP), (0x02b0, b'h' as u16 + L_SUP), (0x02b1, 0x266 + L_SUP), (0x02b2, b'j' as u16 + L_SUP),
    (0x02b3, b'r' as u16 + L_SUP), (0x02b4, 0x279 + L_SUP), (0x02b5, 0x27b + L_SUP), (0x02b6, 0x281 + L_SUP),
    (0x02b7, b'w' as u16 + L_SUP), (0x02b8, b'y' as u16 + L_SUP), (0x02c0, 0x294 + L_SUP), (0x02c1, 0x295 + L_SUP),
    (0x02e0, 0x263 + L_SUP), (0x02e1, b'l' as u16 + L_SUP), (0x02e2, b's' as u16 + L_SUP), (0x02e3, b'x' as u16 + L_SUP),
    (0x2070, b'0' as u16 + L_SUP), (0x2071, b'i' as u16 + L_SUP), (0x2074, b'4' as u16 + L_SUP), (0x2075, b'5' as u16 + L_SUP),
    (0x2076, b'6' as u16 + L_SUP), (0x2077, b'7' as u16 + L_SUP), (0x2078, b'8' as u16 + L_SUP), (0x2079, b'9' as u16 + L_SUP),
    (0x207a, b'+' as u16 + L_SUP), (0x207b, b'-' as u16 + L_SUP), (0x207c, b'=' as u16 + L_SUP), (0x207d, b'(' as u16 + L_SUP),
    (0x207e, b')' as u16 + L_SUP), (0x207f, b'n' as u16 + L_SUP), (0x2080, b'0' as u16 + L_SUB), (0x2081, b'1' as u16 + L_SUB),
    (0x2082, b'2' as u16 + L_SUB), (0x2083, b'3' as u16 + L_SUB), (0x2084, b'4' as u16 + L_SUB), (0x2085, b'5' as u16 + L_SUB),
    (0x2086, b'6' as u16 + L_SUB), (0x2087, b'7' as u16 + L_SUB), (0x2088, b'8' as u16 + L_SUB), (0x2089, b'9' as u16 + L_SUB),
    (0x208a, b'+' as u16 + L_SUB), (0x208b, b'-' as u16 + L_SUB), (0x208c, b'=' as u16 + L_SUB), (0x208d, b'(' as u16 + L_SUB),
    (0x208e, b')' as u16 + L_SUB), (0x2090, b'a' as u16 + L_SUB), (0x2091, b'e' as u16 + L_SUB), (0x2092, b'o' as u16 + L_SUB),
    (0x2093, b'x' as u16 + L_SUB), (0x2094, 0x259 + L_SUB), (0x2095, b'h' as u16 + L_SUB), (0x2096, b'k' as u16 + L_SUB),
    (0x2097, b'l' as u16 + L_SUB), (0x2098, b'm' as u16 + L_SUB), (0x2099, b'n' as u16 + L_SUB), (0x209a, b'p' as u16 + L_SUB),
    (0x209b, b's' as u16 + L_SUB), (0x209c, b't' as u16 + L_SUB),
];

pub(crate) fn is_superscript(letter: i32) -> i32 {
    for &(c, v) in DERIVED_LETTERS.iter() {
        if c as i32 > letter {
            break;
        }
        if c as i32 == letter {
            return v as i32;
        }
    }
    0
}

/// `sprintf` helpers: concatenations of C strings.
fn cat(parts: &[&[u8]]) -> Vec<u8> {
    let mut v = Vec::new();
    for p in parts {
        v.extend_from_slice(cstr(p, 0));
    }
    v
}

impl Espeak {
    /// numbers.c's `ph_ordinal2`, which C's `Lookup` may overflow into `ph_ordinal2x`.
    fn ph_ordinal2(&self) -> Vec<u8> {
        cstr(&self.ph_ordinal, 0).to_vec()
    }

    fn ph_ordinal2x(&self) -> Vec<u8> {
        cstr(&self.ph_ordinal, 12).to_vec()
    }

    fn lookup_letter2(&mut self, tr: Tr, letter: i32, ph_buf: &mut [u8]) -> u8 {
        let mut single_letter = [0u8; 10];
        single_letter[1] = b'_';
        let len = utf8_out(letter, &mut single_letter, 2) as usize;
        single_letter[len + 2] = b' ';
        single_letter[len + 3] = 0;
        if self.lookup(tr, &single_letter[1..], ph_buf) == 0 {
            single_letter[1] = b' ';
            if self.lookup(tr, &single_letter[2..], ph_buf) == 0 {
                let mut sl = single_letter;
                self.translate_rules(tr, &mut sl, 2, ph_buf, 20, None, 0, None);
            }
        }
        at(ph_buf, 0)
    }

    /// `LookupAccentedLetter`: the name of an accented letter, e.g. "e acute".
    pub(crate) fn lookup_accented_letter(&mut self, tr: Tr, letter: i32, ph_buf: &mut [u8], pos: usize) {
        let mut accent_data: u16 = 0;
        let mut accent1 = 0usize;
        let accent2;
        let mut letter2 = 0i32;
        let mut ph_letter1 = [0u8; PH_BUF];
        let mut ph_letter2 = [0u8; PH_BUF];
        let mut ph_accent1 = [0u8; PH_BUF];
        let mut ph_accent2 = [0u8; PH_BUF];

        if (0xe0..0x17f).contains(&letter) {
            accent_data = LETTER_ACCENTS_0E0[(letter - 0xe0) as usize];
        } else if (0x250..=0x2a8).contains(&letter) {
            accent_data = LETTER_ACCENTS_250[(letter - 0x250) as usize];
        }
        if accent_data == 0 {
            return;
        }
        let mut basic_letter = (accent_data & 0x3f) as i32 + 59;
        if basic_letter < b'a' as i32 {
            basic_letter = NON_ASCII_TAB.get((basic_letter - 59) as usize).copied().unwrap_or(0);
        }
        if accent_data & M_LIGATURE != 0 {
            letter2 = ((accent_data >> 6) & 0x3f) as i32 + 59;
            accent2 = ((accent_data >> 12) & 0x7) as usize;
        } else {
            accent1 = ((accent_data >> 6) & 0x1f) as usize;
            accent2 = ((accent_data >> 11) & 0xf) as usize;
        }
        if accent1 == 0 && accent_data & M_LIGATURE == 0 {
            return;
        }
        let mut name1 = ACCENTS_TAB[accent1.min(21)].0.to_vec();
        name1.push(0);
        let flags1 = self.lookup(tr, &name1, &mut ph_accent1);
        if flags1 == 0 {
            return;
        }
        if self.lookup_letter2(tr, basic_letter, &mut ph_letter1) == 0 {
            return;
        }
        let mut out = pos;
        if accent2 != 0 {
            let mut name2 = ACCENTS_TAB[accent2.min(21)].0.to_vec();
            name2.push(0);
            let flags2 = self.lookup(tr, &name2, &mut ph_accent2);
            if flags2 & FLAG_ACCENT_BEFORE != 0 {
                cstrcpy(ph_buf, out as isize, cstr(&ph_accent2, 0));
                out += cstrlen(ph_buf, out as isize);
                ph_accent2[0] = 0;
            }
        }
        let v = if letter2 != 0 {
            self.lookup_letter2(tr, letter2, &mut ph_letter2);
            cat(&[&ph_accent1, &[PHON_PAUSE_VSHORT, 0], &ph_letter1, &[PHON_STRESS_P, 0], &ph_letter2, &ph_accent2])
        } else if accent1 == 0 {
            cat(&[&ph_letter1])
        } else if (self.t(tr).langopts.accents & 1) != 0 || (flags1 & FLAG_ACCENT_BEFORE) != 0 || (ACCENTS_TAB[accent1].1 & 1) != 0 {
            cat(&[&ph_accent1, &[PHON_PAUSE_VSHORT, PHON_STRESS_P, 0], &ph_letter1])
        } else {
            cat(&[&[PHON_STRESS_2, 0], &ph_letter1, &[PHON_PAUSE_VSHORT, 0], &ph_accent1, &[PHON_PAUSE_VSHORT, 0]])
        };
        cstrcpy(ph_buf, out as isize, &v);
    }

    /// `LookupLetter`: the name of a letter, into `ph_buf1[pos..]`. `next_byte` is -1 when
    /// reading normal text rather than spelling.
    pub(crate) fn lookup_letter(&mut self, tr: Tr, letter: i32, next_byte: i32, ph_buf1: &mut [u8], pos: usize, control: i32) {
        let mut single_letter = [0u8; 10];
        let mut ph_buf3 = [0u8; PH_BUF];
        put(ph_buf1, pos as isize, 0);
        let len = utf8_out(letter, &mut single_letter, 2) as usize;
        single_letter[len + 2] = b' ';

        if next_byte == -1 {
            let mut tmp = [0u8; PH_BUF];
            let r = self.lookup(tr, &single_letter[2..], &mut tmp);
            cstrcpy(ph_buf1, pos as isize, cstr(&tmp, 0));
            if r != 0 {
                return;
            }
            single_letter[1] = b'_';
            if self.lookup(tr, &single_letter[1..], &mut ph_buf3) != 0 {
                return;
            }
            if self.t(tr).translator_name == l2(b'e', b'n') {
                return;
            }
            self.set_translator3(ESPEAKNG_DEFAULT_VOICE);
            if self.lookup(Tr::Third, &single_letter[2..], &mut ph_buf3) != 0 {
                cstrcpy(ph_buf1, pos as isize, &[PHON_SWITCH]);
            }
            let ix = self.voice_phoneme_tab_ix;
            self.select_phoneme_table(ix);
            return;
        }

        if letter <= 32 || ucd::isspace(letter) {
            let s = format!("_#{letter} ");
            let mut sb = s.into_bytes();
            sb.push(0);
            let mut tmp = [0u8; PH_BUF];
            self.lookup(tr, &sb, &mut tmp);
            cstrcpy(ph_buf1, pos as isize, cstr(&tmp, 0));
            return;
        }

        let next_byte = if next_byte != b' ' as i32 { RULE_SPELLING as i32 } else { next_byte };
        single_letter[3 + len] = next_byte as u8;
        single_letter[1] = b'_';

        if self.lookup(tr, &single_letter[1..], &mut ph_buf3) == 0 {
            single_letter[1] = b' ';
            if self.lookup(tr, &single_letter[2..], &mut ph_buf3) == 0 {
                let mut sl = single_letter;
                self.translate_rules(tr, &mut sl, 2, &mut ph_buf3, 40, None, FLAG_NO_TRACE, None);
            }
        }
        if ph_buf3[0] == 0 {
            self.lookup_accented_letter(tr, letter, &mut ph_buf3, 0);
        }
        cstrcpy(ph_buf1, pos as isize, cstr(&ph_buf3, 0));
        let first = at(ph_buf1, pos as isize);
        if first == 0 || first == PHON_SWITCH {
            return;
        }
        let mut tmp = [0u8; N_WORD_PHONEMES];
        cstrcpy(&mut tmp, 0, cstr(ph_buf1, pos as isize));
        self.set_word_stress(tr, &mut tmp, 0, Some([0, 0]), -1, control & 1);
        cstrcpy(ph_buf1, pos as isize, cstr(&tmp, 0));
    }

    /// `SetSpellingStress`: reduces the stress of most letter names when spelling a word.
    pub(crate) fn set_spelling_stress(&self, tr: Tr, phonemes: &mut [u8], control: i32, n_chars: i32) {
        let mut n_stress = 0;
        let mut prev = 0u8;
        let mut buf = Vec::with_capacity(N_WORD_PHONEMES);
        for &c in cstr(phonemes, 0) {
            if c == PHON_STRESS_P && prev != PHON_SWITCH {
                n_stress += 1;
            }
            prev = c;
            buf.push(c);
        }
        let mut count = 0;
        prev = 0;
        let mut out = Vec::with_capacity(buf.len() + 2);
        for (ix, &c0) in buf.iter().enumerate() {
            let mut c = c0;
            if c == PHON_STRESS_P && n_chars > 1 && prev != PHON_SWITCH {
                count += 1;
                if self.t(tr).langopts.spelling_stress {
                    if count > 1 {
                        c = PHON_STRESS_3;
                    }
                } else if count != n_stress && ((count % 3) != 0 || count == n_stress - 1) {
                    c = PHON_STRESS_3;
                }
            } else if c == 0xff {
                if control < 2 || ix == 0 {
                    continue;
                }
                c = if (count % 3) == 0 || control > 2 { PHON_PAUSE_NOLINK } else { PHON_PAUSE_VSHORT };
            }
            prev = c;
            out.push(c);
        }
        if control >= 2 {
            out.push(PHON_PAUSE_NOLINK);
        }
        cstrcpy(phonemes, 0, &out);
    }

    fn check_dot_ordinal(&mut self, tr: Tr, w: &mut [u8], word: isize, word_end: isize, wtab: &[WordTab], roman: bool, in_clause: bool) -> i32 {
        let mut ordinal = 0;
        let wt = |k: usize| wtab.get(k).copied().unwrap_or_default();
        if (self.t(tr).langopts.numbers & NUM_ORDINAL_DOT) != 0
            && (at(w, word_end) == b'.' || (wt(0).flags & FLAG_HAS_DOT) != 0)
            && (wt(1).flags & FLAG_NOSPACE) == 0
            && (roman || (wt(1).flags & FLAG_FIRST_UPPER) == 0)
        {
            let c2 = if at(w, word_end) == b'.' { utf8_in(w, word_end + 2).0 } else { utf8_in(w, word_end).0 };
            if at(w, word_end) != 0 && at(w, word_end + 1) != 0 && (c2 == 0 || (wt(0).flags & FLAG_COMMA_AFTER) != 0 || is_alpha(c2)) {
                ordinal = 2;
                if at(w, word_end) == b'.' {
                    put(w, word_end, b' ');
                }
                if !roman && self.t(tr).translator_name == l2(b'h', b'u') {
                    let mut nextflags = 0;
                    if is_alpha(c2) {
                        nextflags = self.translate_word(tr, w, word_end + 2, None, in_clause, None);
                    }
                    if (self.t(tr).prev_dict_flags[0] & FLAG_ALT_TRANS) != 0 && (c2 == 0 || (wt(0).flags & FLAG_COMMA_AFTER) != 0 || ucd::isdigit(c2)) {
                        ordinal = 0;
                    }
                    if nextflags & FLAG_ALT_TRANS != 0 {
                        ordinal = 0;
                    }
                    if nextflags & FLAG_ALT3_TRANS != 0 {
                        if at(w, word - 2) == b'-' {
                            ordinal = 0;
                        }
                        if self.t(tr).prev_dict_flags[0] & (FLAG_ALT_TRANS | FLAG_ALT3_TRANS) != 0 {
                            ordinal = 0x22;
                        }
                    }
                }
            }
        }
        ordinal
    }

    fn hu_number_e(w: &[u8], word: isize, thousandplex: i32, value: i32) -> bool {
        let w0 = at(w, word);
        let w1 = at(w, word + 1);
        if w0 == b'a' || w0 == b'e' {
            if w1 == b' ' || w1 == b'z' || (w1 == b't' && at(w, word + 2) == b't') {
                return false;
            }
            if (thousandplex == 1 || (value % 1000) == 0) && w1 == b'l' {
                return false;
            }
            return true;
        }
        false
    }

    /// `TranslateRoman`.
    pub(crate) fn translate_roman(&mut self, tr: Tr, w: &mut [u8], word: isize, ph_out: &mut [u8], wtab: &mut [WordTab]) -> bool {
        const ROMAN_NUMBERS: &[u8] = b"ixcmvld";
        const ROMAN_VALUES: [i32; 7] = [1, 10, 100, 1000, 5, 50, 500];
        let mut acc = 0;
        let mut prev = 0;
        let mut subtract = 0x7fff;
        let mut repeat = 0;
        let mut num_control = 0;
        let mut flags = [0u32; 2];
        let mut ph_roman = [0u8; PH_BUF];
        put(ph_out, 0, 0);
        let w0flags = wtab.first().map_or(0, |w| w.flags);
        let numbers = self.t(tr).langopts.numbers;

        if ((numbers & NUM_ROMAN_CAPITALS) != 0 && (w0flags & FLAG_ALL_UPPER) == 0) || is_digit09(sat(w, word - 2)) {
            return false;
        }
        if at(w, word + 1) == b' ' && !((numbers & (NUM_ROMAN_CAPITALS | NUM_ROMAN_ORDINAL | NUM_ORDINAL_DOT)) != 0 && (w0flags & FLAG_HAS_DOT) != 0) {
            return false;
        }
        let word_start = word;
        let mut p = word;
        loop {
            let c = at(w, p);
            p += 1;
            if c == b' ' {
                break;
            }
            let Some(ix) = ROMAN_NUMBERS.iter().position(|&r| r == c && c != 0) else { return false };
            let mut value = ROMAN_VALUES[ix];
            if value == prev {
                repeat += 1;
                if repeat >= 3 {
                    return false;
                }
            } else {
                repeat = 0;
            }
            if prev > 1 && prev != 10 && prev != 100 && value >= prev {
                return false;
            }
            if prev != 0 && prev < value {
                if (acc % 10) != 0 || (prev * 10) < value {
                    return false;
                }
                subtract = prev;
                value -= subtract;
            } else if value >= subtract {
                return false;
            } else {
                acc += prev;
            }
            prev = value;
        }
        if is_digit09(at(w, p) as i32) {
            return false;
        }
        acc += prev;
        if acc < self.t(tr).langopts.min_roman || acc > self.t(tr).langopts.max_roman {
            return false;
        }
        self.lookup(tr, b"_roman\0", &mut ph_roman);
        let mut out_pos = 0usize;
        if numbers & NUM_ROMAN_AFTER == 0 {
            cstrcpy(ph_out, 0, cstr(&ph_roman, 0));
            out_pos = cstrlen(&ph_roman, 0);
        }
        let mut number_chars = format!("  {acc} ").into_bytes();
        number_chars.extend_from_slice(self.t(tr).langopts.roman_suffix);
        number_chars.extend_from_slice(b"    ");
        number_chars.push(0);
        number_chars.resize(N_WORD_BYTES.max(number_chars.len()), 0);

        if at(w, p) == b'.' {
            return false;
        }
        if self.check_dot_ordinal(tr, w, word_start, p, wtab, true, false) != 0 {
            if let Some(w0) = wtab.first_mut() {
                w0.flags |= FLAG_ORDINAL;
            }
        }
        if numbers & NUM_ROMAN_ORDINAL != 0 {
            if self.t(tr).translator_name == l2(b'h', b'u') {
                let f = wtab.first().map_or(0, |w| w.flags);
                if f & FLAG_ORDINAL == 0 {
                    if (f & FLAG_HYPHEN_AFTER) != 0 && Self::hu_number_e(w, p, 0, acc) {
                        num_control |= 1;
                    } else {
                        return false;
                    }
                }
            } else if let Some(w0) = wtab.first_mut() {
                w0.flags |= FLAG_ORDINAL;
            }
        }
        self.tm(tr).prev_dict_flags = [0, 0];
        let mut tmp = [0u8; N_WORD_PHONEMES];
        self.translate_number(tr, &mut number_chars, 2, &mut tmp, &mut flags, wtab, num_control, false);
        cstrcpy(ph_out, out_pos as isize, cstr(&tmp, 0));
        if numbers & NUM_ROMAN_AFTER != 0 {
            cstrcat(ph_out, cstr(&ph_roman, 0));
        }
        true
    }

    fn m_variant(&self, value: i32) -> &'static [u8] {
        let teens = (value % 100) > 10 && (value % 100) < 20;
        match self.t(Tr::Main).langopts.numbers2 & NUM2_THOUSANDS_VAR_BITS {
            NUM2_THOUSANDS_VAR1 => {
                if !teens {
                    if value % 10 == 1 {
                        return b"1MA";
                    }
                    if (2..=4).contains(&(value % 10)) {
                        return b"0MA";
                    }
                }
            }
            NUM2_THOUSANDS_VAR2 => {
                if (2..=4).contains(&value) {
                    return b"0MA";
                }
            }
            NUM2_THOUSANDS_VAR3 => {
                if !teens && (2..=4).contains(&(value % 10)) {
                    return b"0MA";
                }
            }
            NUM2_THOUSANDS_VAR4 => {
                if teens || value % 10 == 0 {
                    return b"0MB";
                }
                if value % 10 == 1 {
                    return b"0MA";
                }
            }
            NUM2_THOUSANDS_VAR5 => {
                if !teens {
                    if value % 10 == 1 {
                        return b"1M";
                    }
                    if (2..=4).contains(&(value % 10)) {
                        return b"0MA";
                    }
                }
            }
            _ => {}
        }
        b"0M"
    }

    fn lookup_s(&mut self, tr: Tr, s: String, out: &mut [u8]) -> u32 {
        let mut b = s.into_bytes();
        b.push(0);
        self.lookup(tr, &b, out)
    }

    fn lookup_thousands(&mut self, tr: Tr, value: i32, thousandplex: i32, thousands_exact: i32, ph_out: &mut [u8]) -> bool {
        let mut found_value = 0u32;
        let mut ph_of = [0u8; PH_BUF];
        let mut ph_thousands = [0u8; PH_BUF];
        let mut ph_buf = [0u8; PH_BUF];

        if value > 0 {
            if thousands_exact & 1 != 0 {
                if thousands_exact & 2 != 0 {
                    found_value = self.lookup_s(tr, format!("_{value}M{thousandplex}o"), &mut ph_thousands);
                }
                if found_value == 0 && (self.number_control & 1) != 0 {
                    found_value = self.lookup_s(tr, format!("_{value}M{thousandplex}e"), &mut ph_thousands);
                }
                if found_value == 0 {
                    found_value = self.lookup_s(tr, format!("_{value}M{thousandplex}x"), &mut ph_thousands);
                }
            }
            if found_value == 0 {
                found_value = self.lookup_s(tr, format!("_{value}M{thousandplex}"), &mut ph_thousands);
            }
        }

        if found_value == 0 {
            if (value % 100) >= 20 {
                self.lookup(tr, b"_0of\0", &mut ph_of);
            }
            let mv = String::from_utf8_lossy(self.m_variant(value)).into_owned();
            let mut found = 0;
            if thousands_exact & 1 != 0 {
                if thousands_exact & 2 != 0 {
                    found = self.lookup_s(tr, format!("_{mv}{thousandplex}o"), &mut ph_thousands);
                }
                if found == 0 && (self.number_control & 1) != 0 {
                    found = self.lookup_s(tr, format!("_{mv}{thousandplex}e"), &mut ph_thousands);
                }
                if found == 0 {
                    found = self.lookup_s(tr, format!("_{mv}{thousandplex}x"), &mut ph_thousands);
                }
            }
            if found == 0 && self.lookup_s(tr, format!("_{mv}{thousandplex}"), &mut ph_thousands) == 0 {
                if thousandplex > 3 && self.lookup_s(tr, format!("_0M{}", thousandplex - 1), &mut ph_buf) == 0 {
                    self.lookup(tr, b"_0M2\0", &mut ph_thousands);
                    self.speak_missing_thousands = 3;
                }
                if ph_thousands[0] == 0 {
                    found_value = self.lookup_s(tr, format!("_{value}M1"), &mut ph_thousands);
                    if found_value == 0 {
                        self.lookup(tr, b"_0M1\0", &mut ph_thousands);
                    }
                    self.speak_missing_thousands = 2;
                }
            }
        }
        cstrcpy(ph_out, 0, &cat(&[&ph_of, &ph_thousands]));
        if value == 1 && thousandplex == 1 && (self.t(tr).langopts.numbers & NUM_OMIT_1_THOUSAND) != 0 {
            return true;
        }
        found_value != 0
    }

    fn lookup_num2(&mut self, tr: Tr, value: i32, thousandplex: i32, control: i32, ph_out: &mut [u8]) -> bool {
        let mut found: u32;
        let mut units = value % 10;
        let tens = value / 10;
        let mut used_and = false;
        let mut found_ordinal = 0u32;
        let mut ord_type = b'o';
        let mut ph_ordinal = [0u8; PH_BUF];
        let mut ph_tens = [0u8; PH_BUF];
        let mut ph_digits = [0u8; PH_BUF];
        let mut ph_and = [0u8; PH_BUF];
        let numbers = self.t(tr).langopts.numbers;
        let numbers2 = self.t(tr).langopts.numbers2;

        if control & 0x20 != 0 {
            ord_type = b'q';
        }
        let ord_type = ord_type as char;
        let is_ordinal = control & 1 != 0;
        found = 0;

        if (control & 2) != 0 && self.n_digit_lookup == 2 {
            let dl = self.digit_lookup.clone();
            cstrcpy(ph_out, 0, cstr(&dl, 0));
        } else {
            if at(&self.digit_lookup, 0) == 0 {
                if control & 8 != 0 {
                    found = self.lookup_s(tr, format!("_{value}fx"), &mut ph_digits);
                    if found == 0 {
                        found = self.lookup_s(tr, format!("_{value}f"), &mut ph_digits);
                    }
                } else if is_ordinal {
                    let o = self.ph_ordinal2();
                    cstrcpy(&mut ph_ordinal, 0, &o);
                    if control & 4 != 0 {
                        found = self.lookup_s(tr, format!("_{value}{ord_type}x"), &mut ph_digits);
                        if found != 0 && self.ph_ordinal[12] != 0 {
                            let x = self.ph_ordinal2x();
                            cstrcpy(&mut ph_ordinal, 0, &x);
                        }
                    }
                    if found == 0 {
                        found = self.lookup_s(tr, format!("_{value}{ord_type}"), &mut ph_digits);
                    }
                    found_ordinal = found;
                }
                if found == 0 {
                    if control & 2 != 0 {
                        if self.number_control & 1 != 0 {
                            found = self.lookup_s(tr, format!("_{value}e"), &mut ph_digits);
                        }
                    } else {
                        let s = if (numbers2 & NUM2_ORDINAL_AND_THOUSANDS) != 0 && thousandplex <= 1 { format!("_{value}o") } else { format!("_{value}a") };
                        found = self.lookup_s(tr, s, &mut ph_digits);
                    }
                    if found == 0 && !(is_ordinal && (numbers2 & NUM2_NO_TEEN_ORDINALS) != 0) {
                        found = self.lookup_s(tr, format!("_{value}"), &mut ph_digits);
                    }
                }
            }

            if value < 10 && (control & 0x10) != 0 {
                self.lookup(tr, b"_0\0", &mut ph_tens);
            } else if found != 0 {
                ph_tens[0] = 0;
            } else {
                if is_ordinal && self.lookup_s(tr, format!("_{tens}X{ord_type}"), &mut ph_tens) != 0 {
                    found_ordinal = 1;
                    if units != 0 && (numbers2 & NUM2_MULTIPLE_ORDINAL) != 0 {
                        let o2 = self.ph_ordinal2();
                        cstrcat(&mut ph_tens, cstr(&o2, 0));
                    }
                }
                if found_ordinal == 0 {
                    let s = if control & 0x200 != 0 { format!("_{tens}Xf") } else { format!("_{tens}X") };
                    self.lookup_s(tr, s, &mut ph_tens);
                }
                if ph_tens[0] == 0 && (numbers & NUM_VIGESIMAL) != 0 {
                    units = value % 20;
                    self.lookup_s(tr, format!("_{}X", tens & 0xfe), &mut ph_tens);
                }
                ph_digits[0] = 0;
                if units > 0 {
                    found = 0;
                    if (control & 2) != 0 && at(&self.digit_lookup, 0) != 0 {
                        let dl = self.digit_lookup.clone();
                        cstrcpy(&mut ph_digits, 0, cstr(&dl, 0));
                        found_ordinal = 1;
                        ph_ordinal[0] = 0;
                    } else {
                        if control & 8 != 0 {
                            found = self.lookup_s(tr, format!("_{units}f"), &mut ph_digits);
                        }
                        if is_ordinal && (numbers & NUM_SWAP_TENS) == 0 {
                            found = self.lookup_s(tr, format!("_{units}{ord_type}"), &mut ph_digits);
                            if found != 0 {
                                found_ordinal = 1;
                            }
                        }
                        if found == 0 {
                            if (self.number_control & 1) != 0 && (control & 2) != 0 {
                                found = self.lookup_s(tr, format!("_{units}e"), &mut ph_digits);
                            } else if (control & 2) == 0 || (numbers & NUM_SWAP_TENS) != 0 {
                                let s = if (numbers2 & NUM2_ORDINAL_AND_THOUSANDS) != 0 && thousandplex <= 1 { format!("_{units}o") } else { format!("_{units}a") };
                                found = self.lookup_s(tr, s, &mut ph_digits);
                            }
                        }
                        if found == 0 {
                            self.lookup_s(tr, format!("_{units}"), &mut ph_digits);
                        }
                    }
                }
            }

            if is_ordinal && found_ordinal == 0 && ph_ordinal[0] == 0 {
                if value >= 20 && ((value % 10) == 0 || (numbers & NUM_SWAP_TENS) != 0) {
                    self.lookup(tr, b"_ord20\0", &mut ph_ordinal);
                }
                if ph_ordinal[0] == 0 {
                    self.lookup(tr, b"_ord\0", &mut ph_ordinal);
                }
            }

            if (numbers & (NUM_SWAP_TENS | NUM_AND_UNITS)) != 0 && ph_tens[0] != 0 && ph_digits[0] != 0 {
                self.lookup(tr, b"_0and\0", &mut ph_and);
                if is_ordinal && (numbers2 & NUM2_ORDINAL_NO_AND) != 0 {
                    ph_and[0] = 0;
                }
                let v = if numbers & NUM_SWAP_TENS != 0 {
                    cat(&[&ph_digits, &ph_and, &ph_tens, &ph_ordinal])
                } else {
                    cat(&[&ph_tens, &ph_and, &ph_digits, &ph_ordinal])
                };
                cstrcpy(ph_out, 0, &v);
                used_and = true;
            } else {
                if numbers & NUM_SINGLE_VOWEL != 0 {
                    let ix = cstrlen(&ph_tens, 0) as isize - 1;
                    if ix >= 0 && ph_digits[0] != 0 {
                        let mut next_phtype = self.phz(ph_digits[0]).typ;
                        if next_phtype == PH_STRESS {
                            next_phtype = self.phz(ph_digits[1]).typ;
                        }
                        if self.phz(ph_tens[ix as usize]).typ == PH_VOWEL && next_phtype == PH_VOWEL {
                            ph_tens[ix as usize] = 0;
                        }
                    }
                }
                if (numbers2 & NUM2_ORDINAL_DROP_VOWEL) != 0 && ph_ordinal[0] != 0 {
                    let mut v = cat(&[&ph_tens, &ph_digits]);
                    if let Some(&last) = v.last() {
                        if self.phz(last).typ == PH_VOWEL {
                            v.pop();
                        }
                    }
                    v.extend_from_slice(cstr(&ph_ordinal, 0));
                    cstrcpy(ph_out, 0, &v);
                } else {
                    cstrcpy(ph_out, 0, &cat(&[&ph_tens, &ph_digits, &ph_ordinal]));
                }
            }
        }

        let n = cstrlen(ph_out, 0);
        if numbers & NUM_SINGLE_STRESS_L != 0 {
            let mut f = false;
            for ix in 0..n {
                if ph_out[ix] == PHON_STRESS_P {
                    if f {
                        ph_out[ix] = PHON_STRESS_3;
                    } else {
                        f = true;
                    }
                }
            }
        } else if numbers & NUM_SINGLE_STRESS != 0 {
            let mut f = false;
            for ix in (0..n).rev() {
                if ph_out[ix] == PHON_STRESS_P {
                    if f {
                        ph_out[ix] = PHON_STRESS_3;
                    } else {
                        f = true;
                    }
                }
            }
        }
        used_and
    }

    fn lookup_num3(&mut self, tr: Tr, value: i32, ph_out: &mut [u8], mut suppress_null: bool, thousandplex: i32, mut control: i32) {
        let ordinal = control & 0x22;
        let mut hundreds = value / 100;
        let tensunits = value % 100;
        let mut buf1 = [0u8; PH_BUF];
        let mut buf2 = [0u8; PH_BUF];
        let mut ph_100 = [0u8; PH_BUF];
        let mut ph_10t = [0u8; PH_BUF];
        let mut ph_digits = [0u8; PH_BUF];
        let mut ph_thousands = [0u8; PH_BUF];
        let mut ph_hundred_and = [0u8; PH_BUF];
        let mut ph_thousand_and = [0u8; PH_BUF];
        let numbers = self.t(tr).langopts.numbers;
        let numbers2 = self.t(tr).langopts.numbers2;
        let name = self.t(tr).translator_name;

        let say_zero_hundred = (numbers & NUM_ZERO_HUNDRED) != 0 && ((control & 1) != 0 || hundreds >= 10);

        if hundreds > 0 || say_zero_hundred {
            let mut found = 0;
            if ordinal != 0 && tensunits == 0 {
                found = self.lookup(tr, b"_0Co\0", &mut ph_100);
            }
            if found == 0 {
                if tensunits == 0 {
                    found = self.lookup(tr, b"_0C0\0", &mut ph_100);
                }
                if found == 0 {
                    self.lookup(tr, b"_0C\0", &mut ph_100);
                }
            }

            if (numbers & NUM_1900) != 0 && hundreds == 19 {
            } else if hundreds >= 10 {
                ph_digits[0] = 0;
                let exact = if value % 1000 == 0 { 1 } else { 0 };
                let mut tplex = thousandplex + 1;
                if numbers2 & NUM2_MYRIADS != 0 {
                    tplex = 0;
                }
                if !self.lookup_thousands(tr, hundreds / 10, tplex, exact | ordinal, &mut ph_10t) {
                    let mut x = 0;
                    if (numbers2 & (1i32.wrapping_shl(tplex as u32))) != 0 && tplex <= 3 {
                        x = 8;
                    }
                    if name == l2(b'm', b'l') {
                        x = 0x208;
                    }
                    self.lookup_num2(tr, hundreds / 10, thousandplex, x, &mut ph_digits);
                }
                let v = if numbers2 & NUM2_SWAP_THOUSANDS != 0 {
                    cat(&[&ph_10t, &[PHON_END_WORD, 0], &ph_digits, &[PHON_END_WORD, 0]])
                } else {
                    cat(&[&ph_digits, &[PHON_END_WORD, 0], &ph_10t, &[PHON_END_WORD, 0]])
                };
                cstrcpy(&mut ph_thousands, 0, &v);
                hundreds %= 10;
                if hundreds == 0 && !say_zero_hundred {
                    ph_100[0] = 0;
                }
                suppress_null = true;
                control |= 1;
            }

            ph_digits[0] = 0;
            if hundreds > 0 || say_zero_hundred {
                if (numbers & NUM_AND_HUNDRED) != 0 && ((control & 1) != 0 || ph_thousands[0] != 0) {
                    self.lookup(tr, b"_0and\0", &mut ph_thousand_and);
                }
                suppress_null = true;
                let mut found = 0;
                if ordinal != 0 && (tensunits == 0 || (numbers2 & NUM2_MULTIPLE_ORDINAL) != 0) {
                    found = self.lookup_s(tr, format!("_{hundreds}Co"), &mut ph_digits);
                    if (numbers2 & NUM2_MULTIPLE_ORDINAL) != 0 && tensunits > 0 {
                        let o2 = self.ph_ordinal2();
                        cstrcat(&mut ph_digits, cstr(&o2, 0));
                    }
                }
                if hundreds == 0 && say_zero_hundred {
                    self.lookup(tr, b"_0\0", &mut ph_digits);
                } else {
                    if hundreds == 1 && (numbers2 & NUM2_OMIT_1_HUNDRED_ONLY) != 0 && (control & 1) == 0 {
                    } else {
                        if found == 0 && tensunits == 0 {
                            found = self.lookup_s(tr, format!("_{hundreds}C0"), &mut ph_digits);
                        }
                        if found == 0 {
                            found = self.lookup_s(tr, format!("_{hundreds}C"), &mut ph_digits);
                        }
                    }
                    if found != 0 {
                        ph_100[0] = 0;
                    } else {
                        let mut say_one_hundred = true;
                        if hundreds == 1 && (numbers & NUM_OMIT_1_HUNDRED) != 0 {
                            say_one_hundred = false;
                        }
                        if say_one_hundred {
                            self.lookup_num2(tr, hundreds, thousandplex, 0, &mut ph_digits);
                        }
                    }
                }
            }
            cstrcpy(&mut buf1, 0, &cat(&[&ph_thousands, &ph_thousand_and, &ph_digits, &ph_100]));
        }

        ph_hundred_and[0] = 0;
        if tensunits > 0 {
            if (control & 2) != 0 && (numbers2 & NUM2_MULTIPLE_ORDINAL) != 0 {
            } else {
                if (value > 100 || ((control & 1) != 0 && thousandplex == 0))
                    && ((numbers & NUM_HUNDRED_AND) != 0 || ((numbers & NUM_HUNDRED_AND_DIGIT) != 0 && tensunits < 10))
                {
                    self.lookup(tr, b"_0and\0", &mut ph_hundred_and);
                }
                if (numbers & NUM_THOUSAND_AND) != 0 && hundreds == 0 && ((control & 1) != 0 || ph_thousands[0] != 0) {
                    self.lookup(tr, b"_0and\0", &mut ph_hundred_and);
                }
            }
        }

        buf2[0] = 0;
        if tensunits != 0 || !suppress_null {
            let mut x = 0;
            if thousandplex == 0 {
                x = 2;
                if ordinal != 0 {
                    x = 3;
                }
                if value < 100 && (control & 1) == 0 {
                    x |= 4;
                }
                if ordinal & 0x20 != 0 {
                    x |= 0x20;
                }
            } else if (numbers2 & (1i32.wrapping_shl(thousandplex as u32))) != 0 && thousandplex <= 3 {
                x = 8;
            }
            if name == l2(b'm', b'l') && thousandplex == 1 {
                x |= 0x208;
            }
            if (numbers2 & NUM2_ZERO_TENS) != 0 && ((control & 1) != 0 || hundreds > 0) {
                x |= 0x10;
            }
            if self.lookup_num2(tr, tensunits, thousandplex, x | (control & 0x100), &mut buf2) && (numbers & NUM_SINGLE_AND) != 0 {
                ph_hundred_and[0] = 0;
            }
        } else if self.ph_ordinal[0] != 0 {
            let ix = cstrlen(&buf1, 0);
            if ix > 0 && buf1[ix - 1] == PHON_PAUSE_SHORT {
                buf1[ix - 1] = 0;
            }
            let o2 = self.ph_ordinal2();
            cstrcpy(&mut buf2, 0, cstr(&o2, 0));
        }

        cstrcpy(ph_out, 0, &cat(&[&buf1, &ph_hundred_and, &[PHON_END_WORD, 0], &buf2]));
    }

    fn check_thousands_group(w: &[u8], word: isize, group_len: isize) -> bool {
        for ix in 0..group_len {
            if !is_digit09(at(w, word + ix) as i32) {
                return false;
            }
        }
        !(is_digit09(at(w, word + group_len) as i32) || is_digit09(sat(w, word - 1)))
    }

    #[allow(clippy::too_many_arguments)]
    fn translate_number_1(&mut self, tr: Tr, w: &mut [u8], word: isize, ph_out: &mut [u8], flags: &mut [u32; 2], wtab: &mut [WordTab], control: i32, in_clause: bool) -> bool {
        let mut suppress_null = false;
        let mut decimal_point = 0;
        let mut thousandplex: i32 = 0;
        let mut thousands_exact = 1;
        let mut thousands_inc = 0;
        let mut prev_thousands = 0;
        let mut ordinal = 0;
        let mut skipwords = 0;
        let mut buf1 = [0u8; PH_BUF];
        let mut ph_append = [0u8; PH_BUF];
        let mut ph_buf = [0u8; PH_BUF];
        let ph_buf2 = [0u8; PH_BUF];
        let mut ph_zeros = [0u8; PH_BUF];
        let mut suffix = [0u8; 30];
        let mut buf_digit_lookup = [0u8; PH_BUF];
        const STR_PAUSE: [u8; 2] = [PHON_PAUSE_NOLINK, 0];
        let wt = |wtab: &[WordTab], k: usize| wtab.get(k).copied().unwrap_or_default();

        flags[0] = 0;
        self.n_digit_lookup = 0;
        self.digit_lookup = vec![0u8; PH_BUF];
        self.number_control = control;

        let mut ix: isize = 0;
        while is_digit09(at(w, word + ix) as i32) {
            ix += 1;
        }
        let mut n_digits = ix;

        // strtol
        let mut value: i64 = 0;
        {
            let mut p = word;
            while c_isspace(sat(w, p)) {
                p += 1;
            }
            let mut neg = false;
            if at(w, p) == b'-' || at(w, p) == b'+' {
                neg = at(w, p) == b'-';
                p += 1;
            }
            let start = p;
            let mut overflow = false;
            while at(w, p).is_ascii_digit() {
                value = match value.checked_mul(10).and_then(|v| v.checked_add((at(w, p) - b'0') as i64)) {
                    Some(v) => v,
                    None => {
                        overflow = true;
                        i64::MAX
                    }
                };
                p += 1;
            }
            if neg {
                value = -value;
            }
            if overflow || p == start || value > i32::MAX as i64 {
                return false;
            }
        }
        let this_value = value;
        let _ = this_value;

        let group_len: isize = if self.t(tr).langopts.numbers2 & NUM2_MYRIADS != 0 { 4 } else { 3 };
        let thousands_sep = self.t(tr).langopts.thousands_sep;
        let decimal_sep = self.t(tr).langopts.decimal_sep;
        let numbers = self.t(tr).langopts.numbers;

        if n_digits == group_len && sat(w, word - 2) == thousands_sep && is_digit09(at(w, word - 3) as i32) {
            prev_thousands = 1;
        } else if (thousands_sep == b' ' as i32 || (numbers & NUM_ALLOW_SPACE) != 0)
            && n_digits == 3
            && (wt(wtab, 0).flags & FLAG_MULTIPLE_SPACES) == 0
            && is_digit09(at(w, word - 2) as i32)
        {
            prev_thousands = 1;
        }
        if prev_thousands == 0 {
            self.speak_missing_thousands = 0;
        }

        self.ph_ordinal[0] = 0;
        ph_zeros[0] = 0;

        if prev_thousands != 0 || at(w, word) != b'0' {
            ordinal = self.check_dot_ordinal(tr, w, word, word + ix, wtab, false, in_clause);
        }

        if at(w, word + ix) == b'.' && !is_digit09(at(w, word + ix + 1) as i32) && !is_digit09(at(w, word + ix + 2) as i32) && (wt(wtab, 1).flags & FLAG_NOSPACE) == 0 {
            put(w, word + ix, 0);
        }

        if ordinal == 0 || self.t(tr).translator_name == l2(b'h', b'u') {
            ix += 1;
            let mut p = 0usize;
            if wt(wtab, 0).flags & FLAG_HYPHEN_AFTER != 0 {
                suffix[p] = b'-';
                p += 1;
                ix += 1;
            }
            while at(w, word + ix) != 0 && at(w, word + ix) != b' ' && ix < suffix.len() as isize - 1 {
                if p < suffix.len() - 1 {
                    suffix[p] = at(w, word + ix);
                    p += 1;
                }
                ix += 1;
            }
            suffix[p] = 0;

            if suffix[0] != 0 {
                let ord_ind = self.t(tr).langopts.ordinal_indicator;
                if ord_ind.is_some_and(|o| cstr(&suffix, 0) == o) {
                    ordinal = 2;
                } else if !is_digit09(suffix[0] as i32) {
                    let mut s = b"_#".to_vec();
                    s.extend_from_slice(cstr(&suffix, 0));
                    s.push(0);
                    let mut region = self.ph_ordinal;
                    if self.lookup(tr, &s, &mut region) != 0 {
                        self.ph_ordinal = region;
                        ordinal = 2;
                        flags[0] |= FLAG_SKIPWORDS;
                        skipwords = 1;
                        let mut s = b"_x#".to_vec();
                        s.extend_from_slice(cstr(&suffix, 0));
                        s.push(0);
                        let mut region = self.ph_ordinal;
                        self.lookup(tr, &s, &mut region[12..]);
                        self.ph_ordinal = region;
                    } else {
                        self.ph_ordinal = region;
                    }
                }
            }
        }

        if wt(wtab, 0).flags & FLAG_ORDINAL != 0 {
            ordinal = 2;
        }

        ph_append[0] = 0;

        if at(w, word) == b'0' && prev_thousands == 0 && at(w, word + 1) != b' ' && at(w, word + 1) as i32 != decimal_sep {
            if n_digits == 2 && at(w, word + 3) == b':' && is_digit09(at(w, word + 5) as i32) && c_isspace(sat(w, word + 7)) {
            } else {
                if n_digits > 3 {
                    flags[0] &= !FLAG_SKIPWORDS;
                    return false;
                }
                let mut k = 0;
                while at(w, word + k) == b'0' && k < n_digits - 1 {
                    let mut tmp = [0u8; PH_BUF];
                    self.lookup(tr, b"_0\0", &mut tmp);
                    cstrcat(&mut ph_zeros, cstr(&tmp, 0));
                    k += 1;
                }
            }
        }

        if (numbers & NUM_ALLOW_SPACE) != 0 && at(w, word + n_digits) == b' ' {
            thousands_inc = 1;
        } else if sat(w, word + n_digits) == thousands_sep {
            thousands_inc = 2;
        }

        let mut suffix_ix = n_digits + 2;
        if thousands_inc > 0 {
            let mut digix = n_digits + thousands_inc;
            while (wt(wtab, thousandplex as usize + 1).flags & FLAG_MULTIPLE_SPACES) == 0 && Self::check_thousands_group(w, word + digix, group_len) {
                for k in 0..group_len {
                    if at(w, word + digix + k) != b'0' {
                        thousands_exact = 0;
                        break;
                    }
                }
                thousandplex += 1;
                digix += group_len;
                if sat(w, word + digix) == thousands_sep || ((numbers & NUM_ALLOW_SPACE) != 0 && at(w, word + digix) == b' ') {
                    suffix_ix = digix + 2;
                    digix += thousands_inc;
                } else {
                    break;
                }
            }
        }

        let mut value = value as i32;
        if value == 0 && prev_thousands != 0 {
            suppress_null = true;
        }

        if self.t(tr).translator_name == l2(b'h', b'u')
            && (wt(wtab, thousandplex as usize).flags & FLAG_HYPHEN_AFTER) != 0
            && thousands_exact == 1
            && Self::hu_number_e(w, word + suffix_ix, thousandplex, value)
        {
            self.number_control |= 1;
        }

        if sat(w, word + n_digits) == decimal_sep && is_digit09(at(w, word + n_digits + 1) as i32) {
            self.lookup(tr, b"_dpt\0", &mut ph_append);
            decimal_point = 0x100;
        } else if !suppress_null {
            if thousands_inc > 0 && thousandplex > 0 && self.lookup_thousands(tr, value, thousandplex, thousands_exact, &mut ph_append) {
                value = 0;
                suppress_null = true;
            }
        } else if self.speak_missing_thousands == 1 && self.lookup_s(tr, format!("_0M{}", thousandplex + 1), &mut buf1) == 0 {
            self.lookup_s(tr, format!("_0M{thousandplex}"), &mut ph_append);
        }

        if ph_append[0] == 0 && at(w, word + n_digits) == b'.' && thousandplex == 0 {
            self.lookup(tr, b"_.\0", &mut ph_append);
        }

        let mut out_pos = 0usize;
        if thousandplex == 0 {
            let mut p = word;
            while is_digit09(at(w, p + 1) as i32) {
                p += 1;
            }
            if is_digit09(at(w, p - 1) as i32) {
                let mut f = *flags;
                if self.lookup_dict_list(tr, w, p - 1, in_clause, &mut buf_digit_lookup, &mut f, FLAG_SUFX, Some(wtab), None) {
                    self.n_digit_lookup = 2;
                }
                *flags = f;
            }
            if buf_digit_lookup[0] == 0 && at(w, p) != b'0' {
                let mut f = *flags;
                if self.lookup_dict_list(tr, w, p, in_clause, &mut buf_digit_lookup, &mut f, FLAG_SUFX, Some(wtab), None) {
                    self.n_digit_lookup = 1;
                }
                *flags = f;
            }
            self.digit_lookup = buf_digit_lookup.to_vec();

            if prev_thousands == 0 {
                if decimal_point == 0 && ordinal == 0 && self.lookup_s(tr, format!("_{value}n"), ph_out) != 0 {
                    return true;
                }
                if self.t(tr).langopts.numbers2 & NUM2_PERCENT_BEFORE != 0 {
                    let mut p2 = word;
                    while at(w, p2) != b' ' && at(w, p2) != 0 {
                        p2 += 1;
                    }
                    if at(w, p2 + 1) == b'%' {
                        self.lookup(tr, b"%\0", ph_out);
                        out_pos = cstrlen(ph_out, 0);
                        put(w, p2 + 1, b' ');
                    }
                }
            }
        }

        self.lookup_num3(tr, value, &mut ph_buf, suppress_null, thousandplex, prev_thousands | ordinal | decimal_point);
        let v = if thousandplex > 0 && (self.t(tr).langopts.numbers2 & NUM2_SWAP_THOUSANDS) != 0 {
            cat(&[&ph_zeros, &ph_append, &[PHON_END_WORD, 0], &ph_buf2, &ph_buf])
        } else {
            cat(&[&ph_zeros, &ph_buf2, &ph_buf, &[PHON_END_WORD, 0], &ph_append])
        };
        let out_len = ph_out.len();
        let ph_out = &mut ph_out[out_pos.min(out_len)..];
        cstrcpy(ph_out, 0, &v);

        while decimal_point != 0 {
            n_digits += 1;
            let mut decimal_count: i32 = 0;
            while is_digit09(at(w, word + n_digits + decimal_count as isize) as i32) {
                decimal_count += 1;
            }
            let mut max_decimal_count = 2;
            let decimal_mode = numbers & NUM_DFRACTION_BITS;
            match decimal_mode {
                NUM_DFRACTION_4 | NUM_DFRACTION_2 => {
                    if decimal_mode == NUM_DFRACTION_4 {
                        max_decimal_count = 5;
                    }
                    while at(w, word + n_digits) == b'0' {
                        self.lookup(tr, b"_0\0", &mut buf1);
                        cstrcat(ph_out, cstr(&buf1, 0));
                        decimal_count -= 1;
                        n_digits += 1;
                    }
                    if decimal_count <= max_decimal_count && is_digit09(at(w, word + n_digits) as i32) {
                        let v = atoi(w, word + n_digits);
                        self.lookup_num3(tr, v, &mut buf1, false, 0, 0);
                        cstrcat(ph_out, cstr(&buf1, 0));
                        n_digits += decimal_count as isize;
                    }
                }
                NUM_DFRACTION_1 | NUM_DFRACTION_5 | NUM_DFRACTION_6 => {
                    let v = atoi(w, word + n_digits);
                    self.lookup_num3(tr, v, &mut ph_buf, false, 0, 0);
                    let mut broke = false;
                    if at(w, word + n_digits) == b'0' || decimal_mode != NUM_DFRACTION_1 {
                        if self.lookup_s(tr, format!("_0Z{decimal_count}"), &mut buf1) == 0 {
                            broke = true;
                        } else if decimal_mode == NUM_DFRACTION_6 {
                            cstrcat(ph_out, cstr(&buf1, 0));
                        } else {
                            cstrcat(&mut ph_buf, cstr(&buf1, 0));
                        }
                    }
                    if !broke {
                        cstrcat(ph_out, cstr(&ph_buf, 0));
                        n_digits += decimal_count as isize;
                    }
                }
                NUM_DFRACTION_3 => {
                    if decimal_count <= 4 && at(w, word + n_digits) != b'0' {
                        let v = atoi(w, word + n_digits);
                        self.lookup_num3(tr, v, &mut buf1, false, 0, 0);
                        cstrcat(ph_out, cstr(&buf1, 0));
                        n_digits += decimal_count as isize;
                    }
                }
                NUM_DFRACTION_7 => {
                    while decimal_count > 1 {
                        decimal_count -= 1;
                        let d = at(w, word + n_digits) as char;
                        if self.lookup_s(tr, format!("_{d}d"), &mut buf1) == 0 {
                            break;
                        }
                        n_digits += 1;
                        cstrcat(ph_out, cstr(&buf1, 0));
                    }
                }
                _ => {}
            }

            let mut c;
            loop {
                c = at(w, word + n_digits);
                if !(is_digit09(c as i32) && cstrlen(ph_out, 0) < N_WORD_PHONEMES - 10) {
                    break;
                }
                let v = (at(w, word + n_digits) - b'0') as i32;
                n_digits += 1;
                self.lookup_num2(tr, v, 0, 2, &mut buf1);
                let mut s = vec![PHON_END_WORD];
                s.extend_from_slice(cstr(&buf1, 0));
                cstrcat(ph_out, &s);
            }
            if self.lookup(tr, b"_dpt2\0", &mut buf1) != 0 {
                cstrcat(ph_out, cstr(&buf1, 0));
            }
            if c as i32 == decimal_sep && is_digit09(at(w, word + n_digits + 1) as i32) {
                self.lookup(tr, b"_dpt\0", &mut buf1);
                cstrcat(ph_out, cstr(&buf1, 0));
            } else {
                decimal_point = 0;
            }
        }

        if ph_out[0] != 0 && ph_out[0] != PHON_SWITCH {
            let mut p = word + n_digits + 1;
            let (mut next_char, n) = utf8_in(w, p);
            p += n;
            if (numbers & NUM_NOPAUSE) != 0 && next_char == b' ' as i32 {
                next_char = utf8_in(w, p).0;
            }
            if !ucd::isalpha(next_char) && thousands_exact == 0 {
                cstrcat(ph_out, &STR_PAUSE[..1]);
            }
        }

        flags[0] |= FLAG_FOUND;
        self.speak_missing_thousands -= 1;
        if skipwords != 0 {
            self.dictionary_skipwords = skipwords;
        }
        true
    }

    /// `TranslateNumber`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn translate_number(&mut self, tr: Tr, w: &mut [u8], word: isize, ph_out: &mut [u8], flags: &mut [u32; 2], wtab: &mut [WordTab], control: i32, in_clause: bool) -> bool {
        if self.option_sayas == SAYAS_DIGITS1 || (wtab.first().map_or(0, |w| w.flags) & FLAG_INDIVIDUAL_DIGITS) != 0 {
            return false;
        }
        if self.t(tr).langopts.numbers != 0 {
            return self.translate_number_1(tr, w, word, ph_out, flags, wtab, control, in_clause);
        }
        false
    }
}
