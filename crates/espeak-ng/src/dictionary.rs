//! dictionary.c: dictionary lookup, spelling rules, word stress, and the phoneme string output.

use crate::common::*;
use crate::data::{Dict, PhonemeTab};
use crate::defs::*;
use crate::engine::{Espeak, Tr};
use crate::tr_languages::alphabet_from_char;
use crate::ucd;

const DIERESES_LIST: [u16; 7] = [0xe4, 0xeb, 0xef, 0xf6, 0xfc, 0xff, 0];

pub(crate) const N_REMOVE_ACCENT: i32 = 0x25e;
#[rustfmt::skip]
pub(crate) static REMOVE_ACCENT: [u8; (N_REMOVE_ACCENT - 0xc0) as usize] = [
    b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'c', b'e', b'e', b'e', b'e', b'i', b'i', b'i', b'i', // 0c0
    b'd', b'n', b'o', b'o', b'o', b'o', b'o', 0, b'o', b'u', b'u', b'u', b'u', b'y', b't', b's', // 0d0
    b'a', b'a', b'a', b'a', b'a', b'a', b'a', b'c', b'e', b'e', b'e', b'e', b'i', b'i', b'i', b'i', // 0e0
    b'd', b'n', b'o', b'o', b'o', b'o', b'o', 0, b'o', b'u', b'u', b'u', b'u', b'y', b't', b'y', // 0f0
    b'a', b'a', b'a', b'a', b'a', b'a', b'c', b'c', b'c', b'c', b'c', b'c', b'c', b'c', b'd', b'd', // 100
    b'd', b'd', b'e', b'e', b'e', b'e', b'e', b'e', b'e', b'e', b'e', b'e', b'g', b'g', b'g', b'g', // 110
    b'g', b'g', b'g', b'g', b'h', b'h', b'h', b'h', b'i', b'i', b'i', b'i', b'i', b'i', b'i', b'i', // 120
    b'i', b'i', b'i', b'i', b'j', b'j', b'k', b'k', b'k', b'l', b'l', b'l', b'l', b'l', b'l', b'l', // 130
    b'l', b'l', b'l', b'n', b'n', b'n', b'n', b'n', b'n', b'n', b'n', b'n', b'o', b'o', b'o', b'o', // 140
    b'o', b'o', b'o', b'o', b'r', b'r', b'r', b'r', b'r', b'r', b's', b's', b's', b's', b's', b's', // 150
    b's', b's', b't', b't', b't', b't', b't', b't', b'u', b'u', b'u', b'u', b'u', b'u', b'u', b'u', // 160
    b'u', b'u', b'u', b'u', b'w', b'w', b'y', b'y', b'y', b'z', b'z', b'z', b'z', b'z', b'z', b's', // 170
    b'b', b'b', b'b', b'b', 0, 0, b'o', b'c', b'c', b'd', b'd', b'd', b'd', b'd', b'e', b'e', // 180
    b'e', b'f', b'f', b'g', b'g', b'h', b'i', b'i', b'k', b'k', b'l', b'l', b'm', b'n', b'n', b'o', // 190
    b'o', b'o', b'o', b'o', b'p', b'p', b'y', 0, 0, b's', b's', b't', b't', b't', b't', b'u', // 1a0
    b'u', b'u', b'v', b'y', b'y', b'z', b'z', b'z', b'z', b'z', b'z', b'z', 0, 0, 0, b'w', // 1b0
    b't', b't', b't', b'k', b'd', b'd', b'd', b'l', b'l', b'l', b'n', b'n', b'n', b'a', b'a', b'i', // 1c0
    b'i', b'o', b'o', b'u', b'u', b'u', b'u', b'u', b'u', b'u', b'u', b'u', b'u', b'e', b'a', b'a', // 1d0
    b'a', b'a', b'a', b'a', b'g', b'g', b'g', b'g', b'k', b'k', b'o', b'o', b'o', b'o', b'z', b'z', // 1e0
    b'j', b'd', b'd', b'd', b'g', b'g', b'w', b'w', b'n', b'n', b'a', b'a', b'a', b'a', b'o', b'o', // 1f0
    b'a', b'a', b'a', b'a', b'e', b'e', b'e', b'e', b'i', b'i', b'i', b'i', b'o', b'o', b'o', b'o', // 200
    b'r', b'r', b'r', b'r', b'u', b'u', b'u', b'u', b's', b's', b't', b't', b'y', b'y', b'h', b'h', // 210
    b'n', b'd', b'o', b'o', b'z', b'z', b'a', b'a', b'e', b'e', b'o', b'o', b'o', b'o', b'o', b'o', // 220
    b'o', b'o', b'y', b'y', b'l', b'n', b't', b'j', b'd', b'q', b'a', b'c', b'c', b'l', b't', b's', // 230
    b'z', 0, 0, b'b', b'u', b'v', b'e', b'e', b'j', b'j', b'q', b'q', b'r', b'r', b'y', b'y', // 240
    b'a', b'a', b'a', b'b', b'o', b'c', b'd', b'd', b'e', b'e', b'e', b'e', b'e', b'e',
];

#[rustfmt::skip]
static IPA1: [u16; 96] = [
    0x20, 0x21, 0x22, 0x2b0, 0x24, 0x25, 0x0e6, 0x2c8, 0x28, 0x29, 0x27e, 0x2b, 0x2cc, 0x2d, 0x2e, 0x2f,
    0x252, 0x31, 0x32, 0x25c, 0x34, 0x35, 0x36, 0x37, 0x275, 0x39, 0x2d0, 0x2b2, 0x3c, 0x3d, 0x3e, 0x294,
    0x259, 0x251, 0x3b2, 0xe7, 0xf0, 0x25b, 0x46, 0x262, 0x127, 0x26a, 0x25f, 0x4b, 0x26b, 0x271, 0x14b, 0x254,
    0x3a6, 0x263, 0x280, 0x283, 0x3b8, 0x28a, 0x28c, 0x153, 0x3c7, 0xf8, 0x292, 0x32a, 0x5c, 0x5d, 0x5e, 0x5f,
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x261, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f,
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x7b, 0x7c, 0x7d, 0x303, 0x7f,
];

pub(crate) const STRESS_PHONEMES: [u8; 7] = [
    PHON_STRESS_D,
    PHON_STRESS_U,
    PHON_STRESS_2,
    PHON_STRESS_3,
    PHON_STRESS_P,
    PHON_STRESS_P2,
    PHON_STRESS_TONIC,
];

/// `MatchRecord`. `phonemes` is an offset into the dictionary, `None` for `""`.
#[derive(Clone, Copy, Default)]
struct MatchRecord {
    points: i32,
    phonemes: Option<usize>,
    end_type: u32,
    del_fwd: Option<isize>,
}

/// The result of `LookupDict2`: where the matched words end in the source buffer.
type Found = Option<isize>;

pub(crate) fn hash_dictionary(s: &[u8]) -> usize {
    let mut hash: i32 = 0;
    let mut chars = 0;
    for &c in s {
        if c == 0 {
            break;
        }
        hash = hash.wrapping_mul(8).wrapping_add(c as i32);
        hash = (hash & 0x3ff) ^ (hash >> 8);
        chars += 1;
    }
    ((hash + chars) & 0x3ff) as usize
}

fn letter_group_no(b: u8) -> usize {
    (b as i32 - b'A' as i32).rem_euclid(256) as usize
}

impl Espeak {
    /// `EncodePhonemes`: ASCII phoneme mnemonics at `src[p..]` into phoneme codes at `out[o..]`.
    /// Returns the position after the input consumed, and the unrecognised character, if any.
    pub(crate) fn encode_phonemes(&self, src: &[u8], mut p: isize, out: &mut [u8], mut o: isize) -> (isize, i32) {
        let mut bad_phoneme = 0;
        while at(src, p) < 0x80 && c_isspace(at(src, p) as i32) {
            p += 1;
        }
        loop {
            let mut c = at(src, p);
            if c == 0 || c_isspace(c as i32) {
                break;
            }
            if c == b'|' {
                c = at(src, p + 1);
                if c != b'|' {
                    p += 1;
                    continue;
                }
            }
            let mut max: i32 = -1;
            let mut max_ph: u8 = 0;
            for ix in 1..self.n_phoneme_tab.min(256) {
                let Some(ph) = self.phoneme_tab[ix as usize] else { continue };
                if ph.typ == PH_INVALID {
                    continue;
                }
                let mut count = 0;
                let mnemonic_word = ph.mnemonic;
                loop {
                    let c = at(src, p + count as isize);
                    if !(c > b' ' && count < 4 && c as u32 == (mnemonic_word >> (count * 8)) & 0xff) {
                        break;
                    }
                    count += 1;
                }
                if (count as i32) > max && (count == 4 || ((mnemonic_word >> (count * 8)) & 0xff) == 0) {
                    max = count as i32;
                    max_ph = ph.code;
                }
            }
            if max_ph == 0 {
                bad_phoneme = utf8_in(src, p).0;
                put(out, o, 0);
                return (p + 1, bad_phoneme);
            }
            if max <= 0 {
                max = 1;
            }
            p += max as isize;
            put(out, o, max_ph);
            o += 1;
            if max_ph == PHON_SWITCH {
                let p_lang = o;
                let mut c;
                loop {
                    c = at(src, p);
                    if c_isspace(c as i32) || c == 0 {
                        break;
                    }
                    p += 1;
                    put(out, o, ucd::tolower(c as i32) as u8);
                    o += 1;
                }
                put(out, o, 0);
                if c == 0 {
                    if cstr(out, p_lang) == ESPEAKNG_DEFAULT_VOICE {
                        put(out, p_lang, 0);
                        return (p, bad_phoneme);
                    }
                } else {
                    put(out, o, b'|');
                    o += 1;
                }
            }
        }
        put(out, o, 0);
        (p, bad_phoneme)
    }

    /// `DecodePhonemes`: phoneme codes back to mnemonics.
    pub(crate) fn decode_phonemes(&self, input: &[u8]) -> Vec<u8> {
        const STRESS_CHARS: &[u8] = b"==,,'*  ";
        let mut out = b"* ".to_vec();
        let mut i = 0isize;
        loop {
            let phcode = at(input, i);
            i += 1;
            if phcode == 0 {
                break;
            }
            if phcode == 255 {
                continue;
            }
            let Some(ph) = self.ph(phcode) else { continue };
            if ph.typ == PH_STRESS && ph.std_length <= 4 && ph.program == 0 {
                if ph.std_length > 1 {
                    out.push(STRESS_CHARS[ph.std_length as usize]);
                }
            } else {
                let mut mnem = ph.mnemonic;
                while mnem & 0xff != 0 {
                    out.push((mnem & 0xff) as u8);
                    mnem >>= 8;
                }
                if phcode == PHON_SWITCH {
                    while c_isalpha(sat(input, i)) {
                        out.push(at(input, i));
                        i += 1;
                    }
                }
            }
        }
        out
    }

    /// `WritePhMnemonic`, appending to `out`. `plist` selects `phoneme_list[ix]`.
    pub(crate) fn write_ph_mnemonic(&mut self, out: &mut Vec<u8>, ph: &PhonemeTab, plist: Option<usize>, use_ipa: bool, flags: Option<&mut i32>) {
        if ph.code == PHON_END_WORD {
            return;
        }
        if ph.code == PHON_SWITCH {
            let tone_ph = plist.map_or(0, |ix| self.phoneme_list.get(ix).map_or(0, |p| p.tone_ph));
            out.push(b'(');
            if let Some(t) = self.data.phoneme_tab_list.get(tone_ph as usize) {
                out.extend_from_slice(&t.name);
            }
            out.push(b')');
            return;
        }
        if use_ipa {
            let mut phdata = PhonemeData::default();
            match plist {
                None => self.interpret_phoneme2(ph.code, &mut phdata),
                Some(ix) => {
                    let mut list = std::mem::take(&mut self.phoneme_list);
                    self.interpret_phoneme(None, 0, &mut list, ix as isize, &mut phdata, None);
                    self.phoneme_list = list;
                }
            }
            let ipa = &phdata.ipa_string;
            let mut p = 0;
            if ipa[0] == 0x20 {
                return;
            }
            if ipa[0] != 0 && ipa[0] < 0x20 {
                if let Some(f) = flags {
                    *f = ipa[0] as i32;
                }
                p = 1;
            }
            let s = cstr(ipa, p as isize);
            if !s.is_empty() {
                out.extend_from_slice(s);
                return;
            }
        }
        let mut first = true;
        let mut mnem = ph.mnemonic;
        while mnem & 0xff != 0 {
            let mut c = (mnem & 0xff) as i32;
            mnem >>= 8;
            if c == b'/' as i32 {
                break;
            }
            if use_ipa {
                if first && c == b'_' as i32 {
                    break;
                }
                if c == b'#' as i32 && ph.typ == PH_VOWEL {
                    break;
                }
                if !first && is_digit09(c) {
                    continue;
                }
                if (0x20..128).contains(&c) {
                    c = IPA1[(c - 0x20) as usize] as i32;
                }
                out.extend_from_slice(&utf8_vec(c));
            } else {
                out.push(c as u8);
            }
            first = false;
        }
    }

    /// `GetTranslatedPhonemeString`.
    pub(crate) fn get_translated_phoneme_string(&mut self, phoneme_mode: i32) -> Vec<u8> {
        const STRESS_CHARS: &[u8] = b"==,,''";
        let use_ipa = phoneme_mode & ESPEAK_PHONEMES_IPA != 0;
        let (use_tie, separate_phonemes) =
            if phoneme_mode & ESPEAK_PHONEMES_TIE != 0 { (phoneme_mode >> 8, 0) } else { (0, phoneme_mode >> 8) };
        let mut out = std::mem::take(&mut self.phon_out);
        out.clear();
        let mut buf = Vec::with_capacity(32);
        let mut phon_buf2 = Vec::with_capacity(32);
        for ix in 1..(self.n_phoneme_list - 2).max(1) as usize {
            buf.clear();
            phon_buf2.clear();
            let plist = self.phoneme_list[ix];
            let ph = plist.ph.unwrap_or_default();
            let mut flags = 0;
            self.write_ph_mnemonic(&mut phon_buf2, &ph, Some(ix), use_ipa, Some(&mut flags));
            if plist.newword & PHLIST_START_OF_WORD != 0 && plist.newword & (PHLIST_START_OF_SENTENCE | PHLIST_START_OF_CLAUSE) == 0 {
                buf.push(b' ');
            }
            if (plist.newword == 0 || separate_phonemes == b' ' as i32) && separate_phonemes != 0 && ix > 1 {
                let c = utf8_in(&phon_buf2, 0).0;
                if !(0x2b0..=0x36f).contains(&c) {
                    buf.extend_from_slice(&utf8_vec(separate_phonemes));
                }
            }
            if plist.synthflags & SFLAG_SYLLABLE != 0 {
                let mut stress = plist.stresslevel as i32;
                if stress > 1 {
                    if stress > STRESS_IS_PRIORITY {
                        stress = STRESS_IS_PRIORITY;
                    }
                    let c = if use_ipa {
                        if stress > STRESS_IS_SECONDARY { 0x2c8 } else { 0x2cc }
                    } else {
                        STRESS_CHARS[stress as usize] as i32
                    };
                    if c != 0 {
                        buf.extend_from_slice(&utf8_vec(c));
                    }
                }
            }
            let flags = 0;
            let mut count = 0;
            let mut p = 0isize;
            while at(&phon_buf2, p) != 0 {
                let (c, n) = utf8_in(&phon_buf2, p);
                p += n;
                if use_tie != 0 && count > 0 && (flags & (1 << (count - 1))) == 0 && !(0x2b0..=0x36f).contains(&c) && ucd::isalpha(c) {
                    buf.extend_from_slice(&utf8_vec(use_tie));
                }
                buf.extend_from_slice(&utf8_vec(c));
                count += 1;
            }
            if ph.code != PHON_SWITCH {
                if plist.synthflags & SFLAG_LENGTHEN != 0 {
                    let l = self.phz(PHON_LENGTHEN);
                    self.write_ph_mnemonic(&mut buf, &l, Some(ix), use_ipa, None);
                }
                if plist.synthflags & SFLAG_SYLLABLE != 0 && plist.typ != PH_VOWEL {
                    let s = self.phz(PHON_SYLLABIC);
                    self.write_ph_mnemonic(&mut buf, &s, Some(ix), use_ipa, None);
                }
                if plist.tone_ph > 0 {
                    let t = self.phz(plist.tone_ph);
                    self.write_ph_mnemonic(&mut buf, &t, Some(ix), use_ipa, None);
                }
            }
            if let Some(z) = buf.iter().position(|&c| c == 0) {
                buf.truncate(z);
            }
            out.extend_from_slice(&buf);
        }
        let result = out.clone();
        self.phon_out = out;
        result
    }

    /// `IsLetterGroup`: the length of the matching string from `.L` group `group`, or -1.
    fn is_letter_group(&self, dict: &Dict, w: &[u8], word: isize, group: usize, pre: bool) -> i32 {
        let Some(Some(start)) = dict.letter_groups.get(group) else { return -1 };
        let d = &dict.data;
        let mut p = *start as isize;
        let mut len = 0i32;
        while at(d, p) != RULE_GROUP_END {
            if p as usize >= d.len() {
                return -1;
            }
            if at(d, p) == b'~' {
                return 0;
            }
            let mut wp;
            let mut skip = false;
            if pre {
                len = cstrlen(d, p) as i32;
                wp = word;
                for _ in 0..(len - 1).max(0) {
                    wp -= 1;
                    if at(w, wp) == 0 {
                        skip = true;
                        break;
                    }
                }
            } else {
                wp = word;
            }
            if !skip {
                while at(d, p) == at(w, wp) && at(w, wp) != 0 {
                    wp += 1;
                    p += 1;
                }
                if at(d, p) == 0 {
                    if pre {
                        return len;
                    }
                    return (wp - word) as i32;
                }
            }
            loop {
                let c = at(d, p);
                p += 1;
                if c == 0 {
                    break;
                }
            }
        }
        -1
    }

    pub(crate) fn is_letter(&self, tr: Tr, letter: i32, group: i32) -> bool {
        let t = self.t(tr);
        if let Some(Some(list)) = t.letter_groups.get(group as usize) {
            return wcschr(list, letter);
        }
        if group > 7 || group < 0 {
            return false;
        }
        let mut letter = letter;
        if t.letter_bits_offset > 0 {
            let letter2 = letter.wrapping_sub(t.letter_bits_offset);
            if letter2 > 0 && letter2 < 0x100 {
                letter = letter2;
            } else {
                return false;
            }
        } else if letter >= 0xc0 && letter < N_REMOVE_ACCENT {
            return t.letter_bits[REMOVE_ACCENT[(letter - 0xc0) as usize] as usize] & (1u8 << group) != 0;
        }
        if (0..0x100).contains(&letter) {
            return t.letter_bits[letter as usize] & (1u8 << group) != 0;
        }
        false
    }

    pub(crate) fn is_vowel(&self, tr: Tr, letter: i32) -> bool {
        self.is_letter(tr, letter, LETTERGP_VOWEL2)
    }

    /// `GetVowelStress`. `phonemes` is rewritten in place without its stress marks.
    pub(crate) fn get_vowel_stress(&self, tr: Tr, phonemes: &mut [u8], vowel_stress: &mut [i8; N_WORD_PHONEMES / 2], vowel_count: &mut i32, stressed_syllable: &mut i32, control: i32) -> i32 {
        let mut count: i32 = 1;
        let mut max_stress: i32 = -1;
        let mut stress: i32 = -1;
        let mut primary_posn: i32 = 0;
        let mut rd = 0isize;
        let mut wr = 0isize;
        vowel_stress[0] = STRESS_IS_UNSTRESSED as i8;
        loop {
            let phcode = at(phonemes, rd);
            rd += 1;
            if phcode == 0 || count >= (N_WORD_PHONEMES / 2) as i32 - 1 {
                break;
            }
            let Some(ph) = self.ph(phcode) else { continue };
            if ph.typ == PH_STRESS && ph.program == 0 {
                if phcode == PHON_STRESS_PREV {
                    let mut j = count - 1;
                    while j > 0 && *stressed_syllable == 0 && (vowel_stress[j as usize] as i32) < STRESS_IS_PRIMARY {
                        let vs = vowel_stress[j as usize] as i32;
                        if vs != STRESS_IS_DIMINISHED && vs != STRESS_IS_UNSTRESSED {
                            vowel_stress[j as usize] = STRESS_IS_PRIMARY as i8;
                            if max_stress < STRESS_IS_PRIMARY {
                                max_stress = STRESS_IS_PRIMARY;
                                primary_posn = j;
                            }
                            for ix in 1..j {
                                if vowel_stress[ix as usize] as i32 == STRESS_IS_PRIMARY {
                                    vowel_stress[ix as usize] = STRESS_IS_SECONDARY as i8;
                                }
                            }
                            break;
                        }
                        j -= 1;
                    }
                } else if (ph.std_length as i32) < 4 || *stressed_syllable == 0 {
                    stress = ph.std_length as i32;
                    if stress > max_stress {
                        max_stress = stress;
                    }
                }
                continue;
            }
            if ph.typ == PH_VOWEL && (ph.phflags & PH_NONSYLLABIC) == 0 {
                vowel_stress[count as usize] = stress as i8;
                if stress >= STRESS_IS_PRIMARY && stress >= max_stress {
                    primary_posn = count;
                    max_stress = stress;
                }
                if stress < 0 && (control & 1) != 0 && (ph.phflags & PH_UNSTRESSED) != 0 {
                    vowel_stress[count as usize] = STRESS_IS_UNSTRESSED as i8;
                }
                count += 1;
                stress = -1;
            } else if phcode == PHON_SYLLABIC {
                vowel_stress[count as usize] = stress as i8;
                if stress < 0 && (control & 1) != 0 {
                    vowel_stress[count as usize] = STRESS_IS_UNSTRESSED as i8;
                }
                count += 1;
            }
            put(phonemes, wr, phcode);
            wr += 1;
        }
        vowel_stress[count as usize] = STRESS_IS_UNSTRESSED as i8;
        put(phonemes, wr, 0);

        if *stressed_syllable > 0 {
            if *stressed_syllable >= count {
                *stressed_syllable = count - 1;
            }
            vowel_stress[*stressed_syllable as usize] = STRESS_IS_PRIMARY as i8;
            max_stress = STRESS_IS_PRIMARY;
            primary_posn = *stressed_syllable;
        }

        if max_stress == STRESS_IS_PRIORITY {
            for ix in 1..count as usize {
                if vowel_stress[ix] as i32 == STRESS_IS_PRIMARY {
                    if self.t(tr).langopts.stress_flags & S_PRIORITY_STRESS != 0 {
                        vowel_stress[ix] = STRESS_IS_UNSTRESSED as i8;
                    } else {
                        vowel_stress[ix] = STRESS_IS_SECONDARY as i8;
                    }
                }
                if vowel_stress[ix] as i32 == STRESS_IS_PRIORITY {
                    vowel_stress[ix] = STRESS_IS_PRIMARY as i8;
                    primary_posn = ix as i32;
                }
            }
            max_stress = STRESS_IS_PRIMARY;
        }

        *stressed_syllable = primary_posn;
        *vowel_count = count;
        max_stress
    }

    /// `SetWordStress`: guesses the word's stress pattern; `output` is input and output.
    pub(crate) fn set_word_stress(&mut self, tr: Tr, output: &mut [u8], out_pos: isize, dictionary_flags: Option<[u32; 2]>, tonic: i32, control: i32) {
        const CONSONANT_TYPES: [u8; 16] = [0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0];
        let mut tonic = tonic;
        let mut vowel_stress = [0i8; N_WORD_PHONEMES / 2];
        let mut syllable_weight = [0i8; N_WORD_PHONEMES / 2];
        let mut vowel_length = [0i8; N_WORD_PHONEMES / 2];
        let mut phonetic = [0u8; N_WORD_PHONEMES];

        let stressflags = self.t(tr).langopts.stress_flags;
        let dflags = dictionary_flags.map_or(0, |f| f[0]);

        let mut ix = 0usize;
        while ix < N_WORD_PHONEMES {
            phonetic[ix] = at(output, out_pos + ix as isize);
            if phonetic[ix] as i32 >= self.n_phoneme_tab {
                phonetic[ix] = PHON_SCHWA;
            }
            if phonetic[ix] == 0 {
                break;
            }
            ix += 1;
        }
        if ix == 0 {
            return;
        }
        let final_ph = phonetic[ix.min(N_WORD_PHONEMES) - 1];
        let final_ph2 = phonetic[if ix > 1 { ix - 2 } else { ix - 1 }];

        let mut unstressed_word = false;
        let mut stressed_syllable = (dflags & 0x7) as i32;
        if dflags & 0x8 != 0 {
            stressed_syllable = (dflags & 0x3) as i32;
            unstressed_word = true;
        }

        let mut vowel_count = 0;
        let max_stress_input = self.get_vowel_stress(tr, &mut phonetic, &mut vowel_stress, &mut vowel_count, &mut stressed_syllable, 1);
        let mut max_stress = max_stress_input;
        if max_stress < 0 && dictionary_flags.is_some() {
            max_stress = STRESS_IS_DIMINISHED;
        }

        let vs = |a: &[i8; 100], i: i32| -> i32 { if (0..100).contains(&i) { a[i as usize] as i32 } else { 0 } };

        // heavy or light syllables
        let mut ix = 1usize;
        let mut p = 0usize;
        while phonetic[p] != 0 {
            let ph0 = self.phz(phonetic[p]);
            if ph0.typ == PH_VOWEL && (ph0.phflags & PH_NONSYLLABIC) == 0 {
                let mut weight = 0;
                let lengthened = self.phz(phonetic.get(p + 1).copied().unwrap_or(0)).code == PHON_LENGTHEN;
                if lengthened || (ph0.phflags & PH_LONG) != 0 {
                    weight += 1;
                }
                if ix < 100 {
                    vowel_length[ix] = weight;
                }
                if lengthened {
                    p += 1;
                }
                let p1 = self.phz(phonetic.get(p + 1).copied().unwrap_or(0));
                let p2 = self.phz(phonetic.get(p + 2).copied().unwrap_or(0));
                if CONSONANT_TYPES[(p1.typ & 15) as usize] != 0 && (p2.typ != PH_VOWEL || (p1.phflags & PH_LONG) != 0) {
                    weight += 1;
                }
                if ix < 100 {
                    syllable_weight[ix] = weight;
                }
                ix += 1;
            }
            p += 1;
            if p >= N_WORD_PHONEMES {
                break;
            }
        }

        let stress_rule = self.t(tr).langopts.stress_rule;
        let translator_name = self.t(tr).translator_name;
        let set = |a: &mut [i8; 100], i: i32, v: i32| {
            if (0..100).contains(&i) {
                a[i as usize] = v as i8;
            }
        };
        let mut rule = stress_rule;
        if rule == STRESSPOSN_2LLH {
            if syllable_weight[1] > 0 || syllable_weight[2] == 0 {
                rule = -1;
            } else {
                rule = STRESSPOSN_2L;
            }
        }
        match rule {
            STRESSPOSN_2L => {
                if stressed_syllable == 0 && vowel_count > 2 {
                    stressed_syllable = 2;
                    if max_stress == STRESS_IS_DIMINISHED {
                        set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    }
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_2R => {
                if stressed_syllable == 0 {
                    max_stress = STRESS_IS_PRIMARY;
                    if vowel_count > 2 {
                        stressed_syllable = vowel_count - 2;
                        if stressflags & S_FINAL_SPANISH != 0 {
                            let fph = self.phz(final_ph);
                            if fph.typ != PH_VOWEL {
                                let mnem = fph.mnemonic;
                                let fph2 = self.phz(final_ph2);
                                if translator_name == l2(b'a', b'n') || translator_name == l2(b'c', b'a') {
                                    if (mnem != b's' as u32 && mnem != b'n' as u32) || fph2.typ != PH_VOWEL {
                                        stressed_syllable = vowel_count - 1;
                                    }
                                } else if translator_name == l2(b'i', b'a') {
                                    if mnem != b's' as u32 || fph2.typ != PH_VOWEL {
                                        stressed_syllable = vowel_count - 1;
                                    }
                                } else if mnem == b's' as u32 && fph2.typ == PH_NASAL {
                                } else if (fph.typ != PH_NASAL && mnem != b's' as u32) || fph2.typ != PH_VOWEL {
                                    stressed_syllable = vowel_count - 1;
                                }
                            }
                        }
                        if stressflags & S_FINAL_LONG != 0 && vs(&vowel_length, vowel_count - 1) > vs(&vowel_length, vowel_count - 2) {
                            stressed_syllable = vowel_count - 1;
                        }
                        let v = vs(&vowel_stress, stressed_syllable);
                        if v == STRESS_IS_DIMINISHED || v == STRESS_IS_UNSTRESSED {
                            if stressed_syllable > 1 {
                                stressed_syllable -= 1;
                            } else {
                                stressed_syllable += 1;
                            }
                        }
                    } else {
                        stressed_syllable = 1;
                    }
                    if vs(&vowel_stress, stressed_syllable) < 0
                        && (vs(&vowel_stress, stressed_syllable - 1) < STRESS_IS_PRIMARY || vs(&vowel_stress, stressed_syllable + 1) < STRESS_IS_PRIMARY)
                    {
                        set(&mut vowel_stress, stressed_syllable, max_stress);
                    }
                }
            }
            STRESSPOSN_1R => {
                if stressed_syllable == 0 {
                    stressed_syllable = vowel_count - 1;
                    while stressed_syllable > 0 {
                        if vs(&vowel_stress, stressed_syllable) < STRESS_IS_DIMINISHED {
                            set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                            break;
                        }
                        stressed_syllable -= 1;
                    }
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_3R => {
                if stressed_syllable == 0 {
                    stressed_syllable = vowel_count - 3;
                    if stressed_syllable < 1 {
                        stressed_syllable = 1;
                    }
                    if max_stress == STRESS_IS_DIMINISHED {
                        set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    }
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_SYLCOUNT => {
                if stressed_syllable == 0 {
                    const GUESS_RU: [i32; 16] = [0, 0, 1, 1, 2, 3, 3, 4, 5, 6, 7, 7, 8, 9, 10, 11];
                    const GUESS_RU_V: [i32; 16] = [0, 0, 1, 1, 2, 2, 3, 3, 4, 5, 6, 7, 7, 8, 9, 10];
                    const GUESS_RU_T: [i32; 16] = [0, 0, 1, 2, 3, 3, 3, 4, 5, 6, 7, 7, 7, 8, 9, 10];
                    stressed_syllable = vowel_count - 3;
                    if (0..16).contains(&vowel_count) {
                        let t = self.phz(final_ph).typ;
                        stressed_syllable = if t == PH_VOWEL {
                            GUESS_RU_V[vowel_count as usize]
                        } else if t == PH_STOP {
                            GUESS_RU_T[vowel_count as usize]
                        } else {
                            GUESS_RU[vowel_count as usize]
                        };
                    }
                    set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_1RH => {
                if stressed_syllable == 0 {
                    let mut max_weight = -1;
                    for ix in 1..(vowel_count - 1).max(1) {
                        if vs(&vowel_stress, ix) < STRESS_IS_DIMINISHED {
                            let wt = vs(&syllable_weight, ix);
                            if wt >= max_weight {
                                max_weight = wt;
                                stressed_syllable = ix;
                            }
                        }
                    }
                    if vs(&syllable_weight, vowel_count - 1) == 2 && max_weight < 2 {
                        stressed_syllable = vowel_count - 1;
                    } else if max_weight <= 0 {
                        stressed_syllable = 1;
                    }
                    set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_1RU => {
                if stressed_syllable == 0 {
                    stressed_syllable = vowel_count - 1;
                    for ix in 1..vowel_count {
                        if vs(&vowel_stress, ix) == STRESS_IS_UNSTRESSED {
                            stressed_syllable = ix - 1;
                            break;
                        }
                    }
                    set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_ALL => {
                for ix in 1..vowel_count {
                    if vs(&vowel_stress, ix) < STRESS_IS_DIMINISHED {
                        set(&mut vowel_stress, ix, STRESS_IS_PRIMARY);
                    }
                }
            }
            STRESSPOSN_GREENLANDIC => {
                let mut long_vowel = 0;
                for ix in 1..vowel_count {
                    if vs(&vowel_stress, ix) == STRESS_IS_PRIMARY {
                        set(&mut vowel_stress, ix, STRESS_IS_SECONDARY);
                    }
                    if vs(&vowel_length, ix) > 0 {
                        long_vowel = ix;
                        set(&mut vowel_stress, ix, STRESS_IS_SECONDARY);
                    }
                }
                if stressed_syllable == 0 {
                    if long_vowel > 0 {
                        stressed_syllable = long_vowel;
                    } else if vowel_count > 5 {
                        stressed_syllable = vowel_count - 3;
                    } else {
                        stressed_syllable = vowel_count - 1;
                    }
                }
                set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                max_stress = STRESS_IS_PRIMARY;
            }
            STRESSPOSN_1SL => {
                if stressed_syllable == 0 {
                    stressed_syllable = 1;
                    if vowel_length[1] == 0 && vowel_count > 2 && vowel_length[2] > 0 {
                        stressed_syllable = 2;
                    }
                    set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    max_stress = STRESS_IS_PRIMARY;
                }
            }
            STRESSPOSN_EU => {
                if stressed_syllable == 0 && vowel_count > 2 {
                    for ix in 1..vowel_count {
                        set(&mut vowel_stress, ix, STRESS_IS_DIMINISHED);
                    }
                    stressed_syllable = 2;
                    if max_stress == STRESS_IS_DIMINISHED {
                        set(&mut vowel_stress, stressed_syllable, STRESS_IS_PRIMARY);
                    }
                    max_stress = STRESS_IS_PRIMARY;
                    if vowel_count > 3 {
                        set(&mut vowel_stress, vowel_count - 1, STRESS_IS_SECONDARY);
                    }
                }
            }
            _ => {}
        }

        if stressflags & S_FINAL_VOWEL_UNSTRESSED != 0
            && (control & 2) == 0
            && vowel_count > 2
            && max_stress_input < STRESS_IS_SECONDARY
            && vs(&vowel_stress, vowel_count - 1) == STRESS_IS_PRIMARY
            && self.phz(final_ph).typ == PH_VOWEL
        {
            set(&mut vowel_stress, vowel_count - 1, STRESS_IS_UNSTRESSED);
            set(&mut vowel_stress, vowel_count - 2, STRESS_IS_PRIMARY);
        }

        let mut stress = if max_stress < STRESS_IS_PRIMARY { STRESS_IS_PRIMARY } else { STRESS_IS_SECONDARY };

        if !unstressed_word {
            if stressflags & S_2_SYL_2 != 0 && vowel_count == 3 {
                if vowel_stress[1] as i32 == STRESS_IS_PRIMARY {
                    vowel_stress[2] = STRESS_IS_SECONDARY as i8;
                }
                if vowel_stress[2] as i32 == STRESS_IS_PRIMARY {
                    vowel_stress[1] = STRESS_IS_SECONDARY as i8;
                }
            }
            if stressflags & S_INITIAL_2 != 0 && (vowel_stress[1] as i32) < STRESS_IS_DIMINISHED && vowel_count > 3 && vowel_stress[2] as i32 >= STRESS_IS_PRIMARY {
                vowel_stress[1] = STRESS_IS_SECONDARY as i8;
            }
        }

        let mut done = false;
        let mut first_primary = 0;
        for v in 1..vowel_count {
            if vs(&vowel_stress, v) < STRESS_IS_DIMINISHED {
                if stressflags & S_FINAL_NO_2 != 0 && stress < STRESS_IS_PRIMARY && v == vowel_count - 1 {
                } else if stressflags & 0x8000 != 0 && !done {
                    set(&mut vowel_stress, v, stress);
                    done = true;
                    stress = STRESS_IS_SECONDARY;
                } else if vs(&vowel_stress, v - 1) <= STRESS_IS_UNSTRESSED
                    && (vs(&vowel_stress, v + 1) <= STRESS_IS_UNSTRESSED || (stress == STRESS_IS_PRIMARY && vs(&vowel_stress, v + 1) <= STRESS_IS_NOT_STRESSED))
                {
                    if stress == STRESS_IS_SECONDARY && stressflags & S_NO_AUTO_2 != 0 {
                        continue;
                    }
                    if v > 1 && stressflags & S_2_TO_HEAVY != 0 && syllable_weight[v as usize] == 0 {
                        let skip = (v..vowel_count - 1).any(|i| vs(&syllable_weight, i) > 0);
                        if skip {
                            continue;
                        }
                    }
                    if v > 1 && stressflags & S_2_TO_HEAVY != 0 && syllable_weight[v as usize] == 0 && vs(&syllable_weight, v + 1) > 0 {
                        continue;
                    }
                    set(&mut vowel_stress, v, stress);
                    done = true;
                    stress = STRESS_IS_SECONDARY;
                }
            }
            if vs(&vowel_stress, v) >= STRESS_IS_PRIMARY {
                if first_primary == 0 {
                    first_primary = v;
                } else if stressflags & S_FIRST_PRIMARY != 0 {
                    set(&mut vowel_stress, v, STRESS_IS_SECONDARY);
                }
            }
        }

        if unstressed_word && tonic < 0 {
            tonic = if vowel_count <= 2 { self.t(tr).langopts.unstressed_wd1 } else { self.t(tr).langopts.unstressed_wd2 };
        }

        max_stress = STRESS_IS_DIMINISHED;
        let mut max_stress_posn = 0;
        for v in 1..vowel_count {
            if vs(&vowel_stress, v) >= max_stress {
                max_stress = vs(&vowel_stress, v);
                max_stress_posn = v;
            }
        }

        if tonic >= 0 {
            if tonic > max_stress || max_stress <= STRESS_IS_PRIMARY {
                set(&mut vowel_stress, max_stress_posn, tonic);
            }
            max_stress = tonic;
        }

        // produce output phoneme string
        let mut out = Vec::with_capacity(N_WORD_PHONEMES);
        let max_output = N_WORD_PHONEMES - 3;
        let mut v = 1i32;

        if (control & 1) == 0 {
            if let Some(mut ph) = self.ph(phonetic[0]) {
                let mut p = 0usize;
                while ph.typ == PH_STRESS || phonetic[p] == PHON_END_WORD {
                    p += 1;
                    ph = self.phz(phonetic.get(p).copied().unwrap_or(0));
                    if p >= N_WORD_PHONEMES - 1 {
                        break;
                    }
                }
                let vowel_pause = self.t(tr).langopts.vowel_pause;
                if (vowel_pause & 0x30) != 0 && ph.typ == PH_VOWEL {
                    if (vowel_pause & 0x20) != 0 && vowel_stress[1] as i32 >= STRESS_IS_PRIMARY {
                        out.push(PHON_PAUSE_NOLINK);
                    } else {
                        out.push(PHON_PAUSE_VSHORT);
                    }
                }
            }
        }

        let opt_length = self.t(tr).langopts.param[LOPT_IT_LENGTHEN];
        let mut p = 0usize;
        while out.len() < max_output {
            let phcode = phonetic.get(p).copied().unwrap_or(0);
            p += 1;
            if phcode == 0 {
                break;
            }
            let Some(ph) = self.ph(phcode) else { continue };
            let next = phonetic.get(p).copied().unwrap_or(0);
            if ph.typ == PH_PAUSE {
                self.tm(tr).prev_last_stress = 0;
            } else if (ph.typ == PH_VOWEL && (ph.phflags & PH_NONSYLLABIC) == 0) || next == PHON_SYLLABIC {
                let mut v_stress = vs(&vowel_stress, v);
                self.tm(tr).prev_last_stress = v_stress;
                if v_stress <= STRESS_IS_UNSTRESSED {
                    if v > 1 && max_stress >= 2 && stressflags & S_FINAL_DIM != 0 && v == vowel_count - 1 {
                        v_stress = STRESS_IS_DIMINISHED;
                    } else if stressflags & S_NO_DIM != 0 || v == 1 || v == vowel_count - 1 {
                        v_stress = STRESS_IS_UNSTRESSED;
                    } else if v == vowel_count - 2 && vs(&vowel_stress, vowel_count - 1) <= STRESS_IS_UNSTRESSED {
                        v_stress = STRESS_IS_UNSTRESSED;
                    } else if vs(&vowel_stress, v - 1) < STRESS_IS_DIMINISHED || (stressflags & S_MID_DIM) == 0 {
                        v_stress = STRESS_IS_DIMINISHED;
                        set(&mut vowel_stress, v, v_stress);
                    }
                }
                if v_stress == STRESS_IS_DIMINISHED || v_stress > STRESS_IS_UNSTRESSED {
                    out.push(STRESS_PHONEMES.get(v_stress as usize).copied().unwrap_or(0));
                }
                if vs(&vowel_stress, v) > max_stress {
                    max_stress = vs(&vowel_stress, v);
                }
                if next == PHON_LENGTHEN && (opt_length & 1) != 0 {
                    let shorten = if opt_length & 0x10 != 0 { v != max_stress_posn } else { v_stress < STRESS_IS_PRIMARY };
                    if shorten {
                        p += 1;
                    }
                }
                v += 1;
            }
            if phcode != 1 {
                out.push(phcode);
            }
        }
        out.push(0);
        for (k, &b) in out.iter().enumerate() {
            put(output, out_pos + k as isize, b);
        }
    }

    /// `AppendPhonemes`: appends `ph` to the C string in `string` (unless it won't fit),
    /// counting vowels for the rules' `@` and `&` conditions.
    pub(crate) fn append_phonemes(&mut self, tr: Tr, string: Option<&mut [u8]>, size: usize, ph: &[u8]) {
        let ph = cstr(ph, 0);
        let slen = string.as_deref().map_or(0, |s| cstrlen(s, 0));
        if ph.len() + slen >= size {
            return;
        }
        let mut unstress_mark = false;
        for &c in ph {
            if c as i32 >= self.n_phoneme_tab {
                continue;
            }
            let p = self.phz(c);
            if p.typ == PH_STRESS {
                if p.std_length < 4 {
                    unstress_mark = true;
                }
            } else if p.typ == PH_VOWEL {
                if (p.phflags & PH_UNSTRESSED) == 0 && !unstress_mark {
                    self.tm(tr).word_stressed_count += 1;
                }
                unstress_mark = false;
                self.tm(tr).word_vowel_count += 1;
            }
        }
        if let Some(s) = string {
            cstrcat(s, ph);
        }
    }

    /// `MatchRule`: scores the rules of one group against the word at `w[*word]`.
    #[allow(clippy::too_many_arguments)]
    fn match_rule(&mut self, tr: Tr, dict: &Dict, w: &mut [u8], word: &mut isize, word_start: isize, group_length: i32, rule: Option<usize>, match_out: &mut MatchRecord, word_flags: u32, dict_flags: u32) {
        let d = &dict.data;
        let Some(rule0) = rule else {
            match_out.points = 0;
            *word += 1;
            return;
        };
        let mut rule = rule0 as isize;
        let mut total_consumed: i32 = 0;
        let mut common_phonemes: Option<isize> = None;
        let mut best = MatchRecord::default();
        let mut letter_w: i32;
        let mut last_letter_w: i32;

        while at(d, rule) != RULE_GROUP_END {
            if rule as usize >= d.len() {
                break;
            }
            let mut check_atstart = false;
            let mut consumed: i32 = 0;
            let mut distance_left: i32 = -2;
            let mut distance_right: i32 = -6;
            let mut failed = 0;
            let mut unpron_ignore = word_flags & FLAG_UNPRON_TEST;
            let mut match_type = 0u8;
            letter_w = 0;
            let mut m = MatchRecord { points: 1, ..Default::default() };
            let mut pre_ptr = *word;
            let mut post_ptr = *word + group_length as isize;

            while failed == 0 {
                let rb = at(d, rule);
                rule += 1;
                let mut add_points: i32 = 0;

                if rb <= RULE_LINENUM {
                    match rb {
                        0 => {
                            if let Some(cp) = common_phonemes {
                                let mut q = cp;
                                loop {
                                    let b = at(d, q);
                                    q += 1;
                                    if b == 0 || b == RULE_PHONEMES {
                                        break;
                                    }
                                    if b == RULE_CONDITION {
                                        q += 1;
                                    }
                                    if b == RULE_LINENUM {
                                        q += 2;
                                    }
                                }
                                m.phonemes = Some(q as usize);
                            } else {
                                m.phonemes = None;
                            }
                            rule -= 1;
                            failed = 2;
                        }
                        RULE_PRE_ATSTART => {
                            check_atstart = true;
                            unpron_ignore = 0;
                            match_type = RULE_PRE;
                        }
                        RULE_PRE => {
                            match_type = RULE_PRE;
                            if word_flags & FLAG_UNPRON_TEST != 0 {
                                failed = 1;
                            }
                        }
                        RULE_POST => match_type = RULE_POST,
                        RULE_PHONEMES => {
                            m.phonemes = Some(rule as usize);
                            failed = 2;
                        }
                        RULE_PH_COMMON => common_phonemes = Some(rule),
                        RULE_CONDITION => {
                            let condition_num = at(d, rule) as u32;
                            rule += 1;
                            let cond = self.t(tr).dict_condition as u32;
                            if condition_num >= 32 {
                                if cond & 1u32.wrapping_shl(condition_num - 32) != 0 {
                                    failed = 1;
                                }
                            } else if cond & (1u32 << condition_num) == 0 {
                                failed = 1;
                            }
                            if failed == 0 {
                                m.points += 1;
                            }
                        }
                        RULE_LINENUM => rule += 2,
                        _ => {}
                    }
                    continue;
                }

                match match_type {
                    0 => {
                        let letter = at(w, post_ptr);
                        post_ptr += 1;
                        if letter == rb || (letter == REPLACED_E && rb == b'e') {
                            if letter & 0xc0 != 0x80 {
                                add_points = 21;
                            }
                            consumed += 1;
                        } else {
                            failed = 1;
                        }
                    }
                    RULE_POST => {
                        distance_right += 6;
                        if distance_right > 18 {
                            distance_right = 19;
                        }
                        last_letter_w = letter_w;
                        if at(w, post_ptr - 1) == 0 {
                            failed = 1;
                        } else {
                            let (lw, n) = utf8_in(w, post_ptr);
                            letter_w = lw;
                            let letter_xbytes = n - 1;
                            let letter = at(w, post_ptr);
                            post_ptr += 1;
                            match rb {
                                RULE_LETTERGP => {
                                    let letter_group = letter_group_no(at(d, rule)) as i32;
                                    rule += 1;
                                    if self.is_letter(tr, letter_w, letter_group) {
                                        let lg_pts = if letter_group == 2 { 19 } else { 20 };
                                        add_points = lg_pts - distance_right;
                                        post_ptr += letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_LETTERGP2 => {
                                    let letter_group = letter_group_no(at(d, rule));
                                    rule += 1;
                                    let n_bytes = self.is_letter_group(dict, w, post_ptr - 1, letter_group, false);
                                    if n_bytes >= 0 {
                                        add_points = 20 - distance_right;
                                        post_ptr += n_bytes as isize - 1;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_NOTVOWEL => {
                                    if self.is_letter(tr, letter_w, 0) || (letter_w == b' ' as i32 && word_flags & FLAG_SUFFIX_VOWEL != 0) {
                                        failed = 1;
                                    } else {
                                        add_points = 20 - distance_right;
                                        post_ptr += letter_xbytes;
                                    }
                                }
                                RULE_DIGIT => {
                                    if is_digit(letter_w) {
                                        add_points = 20 - distance_right;
                                        post_ptr += letter_xbytes;
                                    } else if self.t(tr).langopts.tone_numbers != 0 {
                                        add_points = 20 - distance_right;
                                        post_ptr -= 1;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_NONALPHA => {
                                    if !ucd::isalpha(letter_w) {
                                        add_points = 21 - distance_right;
                                        post_ptr += letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_DOUBLE => {
                                    if letter_w == last_letter_w {
                                        add_points = 21 - distance_right;
                                        post_ptr += letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_DOLLAR => {
                                    post_ptr -= 1;
                                    let command = at(d, rule);
                                    rule += 1;
                                    if command == DOLLAR_UNPR {
                                        m.end_type = SUFX_UNPRON;
                                    } else if command == DOLLAR_NOPREFIX {
                                        if word_flags & FLAG_PREFIX_REMOVED != 0 {
                                            failed = 1;
                                        } else {
                                            add_points = 1;
                                        }
                                    } else if (command & 0xf0) == 0x10 {
                                        if dict_flags & (1u32 << (BITNUM_FLAG_ALT + (command & 0xf) as u32)) != 0 {
                                            add_points = 23;
                                        } else {
                                            failed = 1;
                                        }
                                    } else if (command & 0xf0) == 0x20 || command == DOLLAR_LIST {
                                        self.dollar_rule(tr, w, *word, word_start, consumed, group_length, command, &mut failed, &mut add_points);
                                    }
                                }
                                b'-' => {
                                    if letter == b'-' || (letter == b' ' && word_flags & FLAG_HYPHEN_AFTER != 0) {
                                        add_points = 22 - distance_right;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_SYLLABLE => {
                                    let mut p = post_ptr + letter_xbytes;
                                    let mut vowel_count = 0;
                                    let mut syllable_count = 1;
                                    while at(d, rule) == RULE_SYLLABLE {
                                        rule += 1;
                                        syllable_count += 1;
                                    }
                                    let mut vowel = false;
                                    let mut lw = letter_w;
                                    let mut guard = 0;
                                    while lw != RULE_SPACE as i32 {
                                        if !vowel && self.is_letter(tr, lw, LETTERGP_VOWEL2) {
                                            vowel_count += 1;
                                        }
                                        vowel = self.is_letter(tr, lw, LETTERGP_VOWEL2);
                                        let (c, n) = utf8_in(w, p);
                                        lw = c;
                                        p += n;
                                        guard += 1;
                                        if lw == 0 && p > w.len() as isize + 4 || guard > 100_000 {
                                            break;
                                        }
                                    }
                                    letter_w = lw;
                                    if syllable_count <= vowel_count {
                                        add_points = 18 + syllable_count - distance_right;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_NOVOWELS => {
                                    let mut p = post_ptr + letter_xbytes;
                                    let mut lw = letter_w;
                                    let mut guard = 0;
                                    while lw != RULE_SPACE as i32 {
                                        if self.is_letter(tr, lw, LETTERGP_VOWEL2) {
                                            failed = 1;
                                            break;
                                        }
                                        let (c, n) = utf8_in(w, p);
                                        lw = c;
                                        p += n;
                                        guard += 1;
                                        if lw == 0 && p > w.len() as isize + 4 || guard > 100_000 {
                                            break;
                                        }
                                    }
                                    letter_w = lw;
                                    if failed == 0 {
                                        add_points = 19 - distance_right;
                                    }
                                }
                                RULE_SKIPCHARS => {
                                    let mut p = post_ptr - 1;
                                    let mut p2 = p;
                                    let rule_w = utf8_in(d, rule).0;
                                    let mut g_bytes = -1;
                                    let mut lw = letter_w;
                                    while lw != rule_w && lw != RULE_SPACE as i32 && lw != 0 && g_bytes == -1 {
                                        if rule_w == RULE_LETTERGP2 as i32 {
                                            g_bytes = self.is_letter_group(dict, w, p, letter_group_no(at(d, rule + 1)), false);
                                        }
                                        p2 = p;
                                        let (c, n) = utf8_in(w, p);
                                        lw = c;
                                        p += n;
                                    }
                                    letter_w = lw;
                                    if lw == rule_w || g_bytes >= 0 {
                                        post_ptr = p2;
                                    }
                                }
                                RULE_INC_SCORE => {
                                    post_ptr -= 1;
                                    add_points = 20;
                                }
                                RULE_DEC_SCORE => {
                                    post_ptr -= 1;
                                    add_points = -20;
                                }
                                RULE_DEL_FWD => {
                                    let mut p = *word + group_length as isize;
                                    while p < post_ptr {
                                        if at(w, p) == b'e' {
                                            m.del_fwd = Some(p);
                                            break;
                                        }
                                        p += 1;
                                    }
                                }
                                RULE_ENDING => {
                                    let end_type = ((at(d, rule) as u32) << 16) + (((at(d, rule + 1) & 0x7f) as u32) << 8) + (at(d, rule + 2) & 0x7f) as u32;
                                    let t = self.t(tr);
                                    if t.word_vowel_count == 0 && (end_type & SUFX_P) == 0 && (t.langopts.param[LOPT_SUFFIX] & 1) != 0 {
                                        failed = 1;
                                    } else {
                                        m.end_type = end_type;
                                        rule += 3;
                                    }
                                }
                                RULE_NO_SUFFIX => {
                                    if word_flags & FLAG_SUFFIX_REMOVED != 0 {
                                        failed = 1;
                                    } else {
                                        post_ptr -= 1;
                                        add_points = 1;
                                    }
                                }
                                _ => {
                                    if letter == rb {
                                        if letter & 0xc0 != 0x80 {
                                            add_points = 21 - distance_right;
                                        }
                                    } else {
                                        failed = 1;
                                    }
                                }
                            }
                        }
                    }
                    RULE_PRE => {
                        distance_left += 2;
                        if distance_left > 18 {
                            distance_left = 19;
                        }
                        if at(w, pre_ptr) == 0 {
                            failed = 1;
                        } else {
                            last_letter_w = utf8_in(w, pre_ptr).0;
                            pre_ptr -= 1;
                            let (lw, n) = utf8_in2(w, pre_ptr, true);
                            letter_w = lw;
                            let letter_xbytes = n - 1;
                            let letter = at(w, pre_ptr);
                            match rb {
                                RULE_LETTERGP => {
                                    let letter_group = letter_group_no(at(d, rule)) as i32;
                                    rule += 1;
                                    if self.is_letter(tr, letter_w, letter_group) {
                                        let lg_pts = if letter_group == 2 { 19 } else { 20 };
                                        add_points = lg_pts - distance_left;
                                        pre_ptr -= letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_LETTERGP2 => {
                                    let letter_group = letter_group_no(at(d, rule));
                                    rule += 1;
                                    let n_bytes = self.is_letter_group(dict, w, pre_ptr, letter_group, true);
                                    if n_bytes >= 0 {
                                        add_points = 20 - distance_right;
                                        pre_ptr -= n_bytes as isize - 1;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_NOTVOWEL => {
                                    if !self.is_letter(tr, letter_w, 0) {
                                        add_points = 20 - distance_left;
                                        pre_ptr -= letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_DOUBLE => {
                                    if letter_w == last_letter_w {
                                        add_points = 21 - distance_left;
                                        pre_ptr -= letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_DIGIT => {
                                    if is_digit(letter_w) {
                                        add_points = 21 - distance_left;
                                        pre_ptr -= letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_NONALPHA => {
                                    if !ucd::isalpha(letter_w) {
                                        add_points = 21 - distance_right;
                                        pre_ptr -= letter_xbytes;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_DOLLAR => {
                                    pre_ptr += 1;
                                    let command = at(d, rule);
                                    rule += 1;
                                    if command == DOLLAR_LIST || (command & 0xf0) == 0x20 {
                                        self.dollar_rule(tr, w, *word, word_start, consumed, group_length, command, &mut failed, &mut add_points);
                                    }
                                }
                                RULE_SYLLABLE => {
                                    let mut syllable_count = 1;
                                    while at(d, rule) == RULE_SYLLABLE {
                                        rule += 1;
                                        syllable_count += 1;
                                    }
                                    if syllable_count <= self.t(tr).word_vowel_count {
                                        add_points = 18 + syllable_count - distance_left;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_STRESSED => {
                                    pre_ptr += 1;
                                    if self.t(tr).word_stressed_count > 0 {
                                        add_points = 19;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_NOVOWELS => {
                                    let mut p = pre_ptr - letter_xbytes;
                                    let mut lw = letter_w;
                                    while lw != RULE_SPACE as i32 {
                                        if self.is_letter(tr, lw, LETTERGP_VOWEL2) {
                                            failed = 1;
                                            break;
                                        }
                                        let (c, n) = utf8_in2(w, p - 1, true);
                                        lw = c;
                                        p -= n;
                                        if p < -4 {
                                            break;
                                        }
                                    }
                                    letter_w = lw;
                                    if failed == 0 {
                                        add_points = 3;
                                    }
                                }
                                RULE_IFVERB => {
                                    pre_ptr += 1;
                                    if self.t(tr).expect_verb != 0 {
                                        add_points = 1;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_CAPITAL => {
                                    pre_ptr += 1;
                                    if word_flags & FLAG_FIRST_UPPER != 0 {
                                        add_points = 1;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                b'.' => {
                                    let mut p = pre_ptr;
                                    while at(w, p) != b' ' {
                                        if at(w, p) == b'.' {
                                            add_points = 50;
                                            break;
                                        }
                                        p -= 1;
                                        if p < -1 {
                                            break;
                                        }
                                    }
                                    if at(w, p) == b' ' {
                                        failed = 1;
                                    }
                                }
                                b'-' => {
                                    if letter == b'-' || (letter == b' ' && word_flags & FLAG_HYPHEN != 0) {
                                        add_points = 22 - distance_right;
                                    } else {
                                        failed = 1;
                                    }
                                }
                                RULE_SKIPCHARS => {
                                    let mut p = pre_ptr + 1;
                                    let mut p2 = p;
                                    let mut g_bytes = -1;
                                    let r = at(d, rule);
                                    while at(w, p) != r && at(w, p) != RULE_SPACE && at(w, p) != 0 && g_bytes == -1 {
                                        p2 = p;
                                        p -= 1;
                                        if r == RULE_LETTERGP2 {
                                            g_bytes = self.is_letter_group(dict, w, p2, letter_group_no(at(d, rule + 1)), true);
                                        }
                                    }
                                    if at(w, p) == r {
                                        pre_ptr = p2;
                                    }
                                    if g_bytes >= 0 {
                                        pre_ptr = p2 + 1;
                                    }
                                }
                                _ => {
                                    if letter == rb {
                                        if letter == RULE_SPACE {
                                            add_points = 4;
                                        } else if letter & 0xc0 != 0x80 {
                                            add_points = 21 - distance_left;
                                        }
                                    } else {
                                        failed = 1;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }

                if failed == 0 {
                    m.points += add_points;
                }
            }

            if failed == 2 && unpron_ignore == 0 && (!check_atstart || at(w, pre_ptr - 1) == b' ') {
                if check_atstart {
                    m.points += 4;
                }
                if m.points >= best.points {
                    best = m;
                    total_consumed = consumed;
                }
            }

            loop {
                let b = at(d, rule);
                rule += 1;
                if b == 0 || rule as usize > d.len() {
                    break;
                }
            }
        }

        total_consumed += group_length;
        if total_consumed == 0 {
            total_consumed = 1;
        }
        *word += total_consumed as isize;
        if best.points == 0 {
            best.phonemes = None;
        }
        *match_out = best;
    }

    /// `TranslateRules`: spelling-to-sound rules for the word at `w[p_start]`, appended to
    /// `phonemes`; any standard prefix or suffix goes to `end_phonemes`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn translate_rules(&mut self, tr: Tr, w: &mut [u8], p_start: isize, phonemes: &mut [u8], ph_size: usize, mut end_phonemes: Option<&mut [u8]>, word_flags: u32, mut dict_flags: Option<&mut [u32; 2]>) -> u32 {
        let Some(dict) = self.t(tr).dict.clone() else { return 0 };
        let d = &dict.data;
        let mut any_alpha = 0;
        let mut digit_count = 0u32;
        let mut match1 = MatchRecord::default();
        let mut match2 = MatchRecord::default();
        let mut ph_buf = [0u8; N_PHONEME_BYTES];
        let mut word_copy = [0u8; N_WORD_BYTES];
        const STR_PAUSE: [u8; 2] = [PHON_PAUSE_NOLINK, 0];

        let dict_flags0 = dict_flags.as_deref().map_or(0, |f| f[0]);

        let mut ix = 0usize;
        while ix < N_WORD_BYTES - 1 {
            let c = at(w, p_start + ix as isize);
            word_copy[ix] = c;
            ix += 1;
            if c == 0 {
                break;
            }
        }
        word_copy[ix] = 0;

        let mut p = p_start;
        self.tm(tr).word_vowel_count = 0;
        self.tm(tr).word_stressed_count = 0;

        if let Some(e) = end_phonemes.as_deref_mut() {
            put(e, 0, 0);
        }

        let mut restarts = 0;
        loop {
            let c = at(w, p);
            if c == b' ' || c == 0 {
                break;
            }
            let (wc, wc_bytes) = utf8_in(w, p);
            if is_alpha(wc) {
                any_alpha += 1;
            }
            let mut n = dict.groups2_count[c as usize] as usize;
            if is_digit(wc) && (self.t(tr).langopts.tone_numbers == 0 || any_alpha == 0) {
                let mut string = vec![b'_'];
                for k in 0..wc_bytes {
                    string.push(at(w, p + k));
                }
                string.push(0);
                let mut buf = [0u8; 40];
                self.lookup(tr, &string, &mut buf);
                digit_count += 1;
                if digit_count >= 2 {
                    cstrcat(&mut buf, &STR_PAUSE[..1]);
                    digit_count = 0;
                }
                self.append_phonemes(tr, Some(phonemes), ph_size, &buf);
                p += wc_bytes;
                continue;
            }
            digit_count = 0;
            let mut found = false;
            let mut match1_local = false;

            let offset_ix = wc - self.t(tr).letter_bits_offset;
            if (0..128).contains(&offset_ix) {
                if let Some(g3) = dict.groups3[offset_ix as usize] {
                    self.match_rule(tr, &dict, w, &mut p, p_start, wc_bytes as i32, Some(g3), &mut match1, word_flags, dict_flags0);
                    found = true;
                }
            }

            if !found && n > 0 {
                let c2 = at(w, p + 1);
                let c12 = c as u32 + ((c2 as u32) << 8);
                let g1 = dict.groups2_start[c as usize] as usize;
                for g in g1..(g1 + n) {
                    if dict.groups2_name.get(g) == Some(&c12) {
                        found = true;
                        let mut p2 = p;
                        self.match_rule(tr, &dict, w, &mut p2, p_start, 2, dict.groups2.get(g).copied(), &mut match2, word_flags, dict_flags0);
                        if match2.points > 0 {
                            match2.points += 35;
                        }
                        self.match_rule(tr, &dict, w, &mut p, p_start, 1, dict.groups1[c as usize], &mut match1, word_flags, dict_flags0);
                        if match2.points >= match1.points {
                            match1 = match2;
                            p = p2;
                        }
                    }
                }
            }

            if !found {
                if dict.groups1[c as usize].is_some() {
                    self.match_rule(tr, &dict, w, &mut p, p_start, 1, dict.groups1[c as usize], &mut match1, word_flags, dict_flags0);
                } else {
                    self.match_rule(tr, &dict, w, &mut p, p_start, 0, dict.groups1[0], &mut match1, word_flags, dict_flags0);
                    if match1.points == 0 && (self.option_sayas & 0x10) == 0 {
                        let (letter, nb) = utf8_in(w, p - 1);
                        n = (nb - 1) as usize;
                        if self.t(tr).letter_bits_offset > 0 && letter <= 0x241 && ucd::isalpha(letter) {
                            cstrcpy(phonemes, 0, &[PHON_SWITCH, b'e', b'n']);
                            return 0;
                        }
                        if letter == 0xe000 + b'(' as i32 {
                            let v = self.t(tr).langopts.param[LOPT_BRACKET_PAUSE_ANNOUNCED];
                            if self.pre_pause < v {
                                self.pre_pause = v;
                            }
                        }
                        if is_bracket(letter) {
                            let v = self.t(tr).langopts.param[LOPT_BRACKET_PAUSE];
                            if self.pre_pause < v {
                                self.pre_pause = v;
                            }
                        }
                        if letter >= 0xc0 && letter < N_REMOVE_ACCENT {
                            let ixa = REMOVE_ACCENT[(letter - 0xc0) as usize];
                            if ixa != 0 && (at(w, p - 2) != b' ' || at(w, p + n as isize) != b' ') {
                                let p2 = p - 1;
                                put(w, p - 1, ixa);
                                loop {
                                    let v = at(w, p + n as isize);
                                    put(w, p, v);
                                    if v == b' ' || p as usize >= w.len() {
                                        break;
                                    }
                                    p += 1;
                                }
                                for _ in 0..n {
                                    put(w, p, b' ');
                                    p += 1;
                                }
                                restarts += 1;
                                if restarts > N_WORD_BYTES * 4 {
                                    break;
                                }
                                if self.t(tr).langopts.param[LOPT_DIERESES] != 0 && lookupwchar(&DIERESES_LIST, letter) > 0 {
                                    p = p2;
                                    continue;
                                }
                                put(phonemes, 0, 0);
                                p = p_start;
                                self.tm(tr).word_vowel_count = 0;
                                self.tm(tr).word_stressed_count = 0;
                                continue;
                            }
                        }
                        if let Some(alphabet) = alphabet_from_char(letter) {
                            if alphabet.offset != self.t(tr).letter_bits_offset {
                                if self.t(tr).langopts.alt_alphabet == alphabet.offset {
                                    let mut s = vec![PHON_SWITCH];
                                    s.extend(word_to_string2(self.t(tr).langopts.alt_alphabet_lang));
                                    cstrcpy(phonemes, 0, &s);
                                    return 0;
                                }
                                if alphabet.flags & AL_WORDS != 0 {
                                    let mut s = vec![PHON_SWITCH];
                                    s.extend(word_to_string2(alphabet.language));
                                    cstrcpy(phonemes, 0, &s);
                                    return 0;
                                }
                            }
                        }
                    }
                }

                if match1.points == 0 {
                    if (0x300..=0x36f).contains(&wc) {
                    } else if is_alpha(wc) {
                        if any_alpha > 1 || sat(w, p + wc_bytes - 1) > b' ' as i32 {
                            put(phonemes, 0, 0);
                            if let Some(f) = dict_flags.as_deref_mut() {
                                f[0] |= FLAG_SPELLWORD;
                            }
                            break;
                        }
                    } else {
                        ph_buf[0] = 0;
                        self.lookup_letter(tr, wc, -1, &mut ph_buf, 0, 0);
                        if ph_buf[0] != 0 {
                            match1.phonemes = None;
                            match1_local = true;
                            match1.points = 1;
                        }
                    }
                    p += wc_bytes - 1;
                } else {
                    self.tm(tr).phonemes_repeat_count = 0;
                }
            }

            let m_ph: Vec<u8> = if match1_local {
                cstr(&ph_buf, 0).to_vec()
            } else {
                match1.phonemes.map_or(Vec::new(), |o| cstr(d, o as isize).to_vec())
            };

            if match1.points > 0 {
                if word_flags & FLAG_UNPRON_TEST != 0 {
                    return match1.end_type | 1;
                }
                if m_ph.first() == Some(&PHON_SWITCH) && (word_flags & FLAG_DONT_SWITCH_TRANSLATOR) == 0 {
                    cstrcpy(phonemes, 0, &m_ph);
                    return 0;
                }
                match1.end_type &= !SUFX_UNPRON;
                if match1.end_type != 0 && end_phonemes.is_some() {
                    if (match1.end_type & SUFX_P) != 0 && (word_flags & FLAG_NO_PREFIX) != 0 {
                    } else {
                        if (match1.end_type & SUFX_P) != 0 && (match1.end_type & 0x7f) == 0 {
                            match1.end_type |= (p - p_start) as u32;
                        }
                        if let Some(e) = end_phonemes.as_deref_mut() {
                            cstrcpy(e, 0, &m_ph);
                        }
                        let wl = cstrlen(&word_copy, 0);
                        memcpy(w, p_start, &word_copy, 0, wl);
                        return match1.end_type;
                    }
                }
                if let Some(df) = match1.del_fwd {
                    put(w, df, REPLACED_E);
                }
                self.append_phonemes(tr, Some(phonemes), ph_size, &m_ph);
            }
        }

        let wl = cstrlen(&word_copy, 0);
        memcpy(w, p_start, &word_copy, 0, wl);
        0
    }

    /// `TransposeAlphabet`: packs the word's letters into 6-bit codes, as `*_list` stores them.
    /// Returns the new length, with bit 6 set if compressed.
    fn transpose_alphabet(&self, tr: Tr, text: &mut Vec<u8>) -> usize {
        let t = self.t(tr);
        let min = t.transpose_min;
        let max = t.transpose_max;
        let offset = min - 1;
        let map = t.transpose_map;
        let pairs_start = max - min + 2;
        let mut all_alpha = true;
        let mut buf = Vec::with_capacity(N_WORD_BYTES + 1);
        let mut p = 0isize;
        loop {
            let (c, n) = utf8_in(text, p);
            p += n;
            if c != 0 {
                if c >= min && c <= max {
                    match map {
                        None => buf.push((c - offset) as u8),
                        Some(map) => {
                            let v = map.get((c - min) as usize).copied().unwrap_or(0);
                            if v > 0 {
                                buf.push(v as u8);
                            } else {
                                all_alpha = false;
                                break;
                            }
                        }
                    }
                } else {
                    all_alpha = false;
                    break;
                }
            }
            if c == 0 || buf.len() >= N_WORD_BYTES {
                break;
            }
        }
        if all_alpha {
            let mut acc: i32 = 0;
            let mut bits = 0;
            let mut out = Vec::new();
            let mut i = 0usize;
            while i < buf.len() {
                let mut c = buf[i] as i32;
                i += 1;
                if let Some(pairs) = t.frequent_pairs {
                    let c2 = c + ((buf.get(i).copied().unwrap_or(0) as i32) << 8);
                    let mut ix = 0;
                    while ix < pairs.len() && c2 >= pairs[ix] as i32 {
                        if c2 == pairs[ix] as i32 {
                            c = ix as i32 + pairs_start;
                            i += 1;
                            break;
                        }
                        ix += 1;
                    }
                }
                acc = (acc << 6) + (c & 0x3f);
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    out.push((acc >> bits) as u8);
                }
            }
            if bits > 0 {
                out.push((acc << (8 - bits)) as u8);
            }
            let ix = out.len();
            let tail = text.get(ix..).map(|s| s.to_vec()).unwrap_or_default();
            text.clear();
            text.extend_from_slice(&out);
            text.extend_from_slice(&tail);
            return ix | 0x40;
        }
        cstrlen(text, 0)
    }

    /// `LookupDict2`: finds `word` in the dictionary. `w2` is the text after the word (the
    /// following words, for multi-word entries), in buffer `src`. Returns where the matched
    /// text ends in `src`.
    #[allow(clippy::too_many_arguments)]
    fn lookup_dict2(&mut self, tr: Tr, word: &[u8], src: &[u8], word2: isize, in_clause: bool, phonetic: &mut [u8], flags: &mut [u32; 2], end_flags: u32, wtab: Option<&[WordTab]>) -> Found {
        let Some(dict) = self.t(tr).dict.clone() else { return None };
        let d = &dict.data;
        let wflags = wtab.and_then(|w| w.first()).map_or(0, |w| w.flags);
        let lookup_symbol = flags[1] & FLAG_LOOKUP_SYMBOL;
        let mut word_buf: Vec<u8>;
        let wlen;
        let word: &[u8] = if self.t(tr).transpose_min > 0 {
            word_buf = vec![0; N_WORD_BYTES];
            strncpy0(&mut word_buf, word, N_WORD_BYTES);
            word_buf.truncate(N_WORD_BYTES);
            wlen = self.transpose_alphabet(tr, &mut word_buf);
            &word_buf
        } else {
            wlen = cstrlen(word, 0);
            word
        };
        let hash = hash_dictionary(cstr(word, 0));
        let mut p = dict.hashtab[hash] as isize;
        let mut condition_failed = false;

        while at(d, p) != 0 {
            let next = p + at(d, p) as isize;
            if (at(d, p + 1) & 0x7f) as usize != wlen || (0..(wlen & 0x3f) as isize).any(|k| at(word, k) != at(d, p + 2 + k)) {
                p = next;
                continue;
            }
            let mut word_end = word2;
            let mut dictionary_flags: u32 = 0;
            let mut dictionary_flags2: u32 = 0;
            let no_phonemes = at(d, p + 1) & 0x80 != 0;
            p += ((at(d, p + 1) & 0x3f) + 2) as isize;
            let phoneme_len;
            if no_phonemes {
                put(phonetic, 0, 0);
                phoneme_len = 0;
            } else {
                let s = cstr(d, p);
                phoneme_len = s.len();
                cstrcpy(phonetic, 0, s);
                p += phoneme_len as isize + 1;
            }
            while p < next {
                let flag = at(d, p);
                p += 1;
                if flag >= 100 {
                    let cond = self.t(tr).dict_condition as u32;
                    if flag >= 132 {
                        if cond & 1u32.wrapping_shl(flag as u32 - 132) != 0 {
                            condition_failed = true;
                        }
                    } else if cond & 1u32.wrapping_shl(flag as u32 - 100) == 0 {
                        condition_failed = true;
                    }
                } else if flag > 80 {
                    let n_chars = (next - p) as usize;
                    let skipwords = flag as i32 - 80;
                    if let Some(wt) = wtab {
                        let mut ix = 0usize;
                        while ix as i32 <= skipwords {
                            let Some(e) = wt.get(ix) else { break };
                            if e.length == 0 {
                                break;
                            }
                            if e.flags & FLAG_EMPHASIZED2 != 0 {
                                condition_failed = true;
                            }
                            ix += 1;
                        }
                    }
                    // strncmp(word2, p, n_chars)
                    let mut differ = false;
                    for k in 0..n_chars as isize {
                        let a = at(src, word2 + k);
                        let b = at(d, p + k);
                        if a != b {
                            differ = true;
                            break;
                        }
                        if a == 0 {
                            break;
                        }
                    }
                    if differ {
                        condition_failed = true;
                    }
                    if condition_failed {
                        p = next;
                        break;
                    }
                    dictionary_flags |= FLAG_SKIPWORDS;
                    self.dictionary_skipwords = skipwords;
                    p = next;
                    word_end = word2 + n_chars as isize;
                } else if flag > 64 {
                    dictionary_flags = (dictionary_flags & !0xf) | (flag as u32 & 0xf);
                    if (flag & 0xc) == 0xc {
                        dictionary_flags |= FLAG_STRESS_END;
                    }
                } else if flag >= 32 {
                    dictionary_flags2 |= 1u32 << (flag - 32);
                } else {
                    dictionary_flags |= 1u32 << flag;
                }
            }

            if condition_failed {
                condition_failed = false;
                continue;
            }
            if (end_flags & FLAG_SUFX) == 0 && (dictionary_flags2 & FLAG_STEM) != 0 {
                continue;
            }
            if (end_flags & SUFX_P) != 0 && (dictionary_flags2 & (FLAG_ONLY | FLAG_ONLY_S)) != 0 {
                continue;
            }
            if end_flags & FLAG_SUFX != 0 {
                if dictionary_flags2 & FLAG_ONLY != 0 {
                    continue;
                }
                if (dictionary_flags2 & FLAG_ONLY_S) != 0 && (end_flags & FLAG_SUFX_S) == 0 {
                    continue;
                }
            }
            if (dictionary_flags2 & FLAG_CAPITAL) != 0 && (wflags & FLAG_FIRST_UPPER) == 0 {
                continue;
            }
            if (dictionary_flags2 & FLAG_ALLCAPS) != 0 && (wflags & FLAG_ALL_UPPER) == 0 {
                continue;
            }
            if (dictionary_flags & FLAG_NEEDS_DOT) != 0 && (wflags & FLAG_HAS_DOT) == 0 {
                continue;
            }
            if (dictionary_flags2 & FLAG_ATEND) != 0 && lookup_symbol == 0 && self.before_clause_end(in_clause, word_end) {
                continue;
            }
            if (dictionary_flags2 & FLAG_ATSTART) != 0 && (wflags & FLAG_FIRST_WORD) == 0 {
                continue;
            }
            if (dictionary_flags2 & FLAG_SENTENCE) != 0 && (self.t(Tr::Main).clause_terminator & CLAUSE_TYPE_SENTENCE) == 0 {
                continue;
            }
            if dictionary_flags2 & FLAG_VERB != 0 {
                let t = self.t(tr);
                if t.expect_verb != 0 || (t.expect_verb_s != 0 && (end_flags & FLAG_SUFX_S) != 0) {
                    if t.translator_name == l2(b'e', b'n') && (t.prev_dict_flags[0] & FLAG_ALT7_TRANS) != 0 && (end_flags & FLAG_SUFX_S) != 0 {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            if (dictionary_flags2 & FLAG_PAST) != 0 && self.t(tr).expect_past == 0 {
                continue;
            }
            if (dictionary_flags2 & FLAG_NOUN) != 0 && (self.t(tr).expect_noun == 0 || (end_flags & SUFX_V) != 0) {
                continue;
            }
            if (dictionary_flags2 & FLAG_NATIVE) != 0 && tr != Tr::Main {
                continue;
            }
            if (dictionary_flags & FLAG_ALT2_TRANS) != 0 && self.t(tr).translator_name == l2(b'h', b'u') && (self.t(tr).prev_dict_flags[0] & FLAG_ALT_TRANS) == 0 {
                continue;
            }

            flags[0] = dictionary_flags | FLAG_FOUND_ATTRIBUTES;
            flags[1] = dictionary_flags2;

            if phoneme_len == 0 {
                return None;
            }
            flags[0] |= FLAG_FOUND;
            let (c, ix) = utf8_in(word, 0);
            if at(word, ix) == 0 && !is_alpha(c) {
                flags[0] |= FLAG_MAX3;
            }
            return Some(word_end);
        }
        None
    }

    /// C compares `word_end < translator->clause_end`, two pointers. They can be compared
    /// meaningfully when the word is in the clause buffer.
    fn before_clause_end(&self, in_clause: bool, word_end: isize) -> bool {
        if in_clause {
            return word_end < self.t(Tr::Main).clause_end;
        }
        // other buffers (Lookup's text[], number_buf, ...) are on the stack below the clause buffer
        true
    }

    /// `LookupDictList`: looks up the word at `w[*wordptr]`, also trying an `a.b.c`
    /// abbreviation, a removed `e` and an undoubled final letter. If the entry is replacement
    /// text, `*replacement` receives it (C points `*wordptr` at a static buffer).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn lookup_dict_list(&mut self, tr: Tr, w: &[u8], wordptr: isize, in_clause: bool, ph_out: &mut [u8], flags: &mut [u32; 2], end_flags: u32, wtab: Option<&[WordTab]>, replacement: Option<&mut Vec<u8>>) -> bool {
        let mut word = [0u8; N_WORD_BYTES];
        let mut length = 0usize;
        let mut word1 = wordptr;
        let mut word2 = wordptr;

        let utf8_nbytes = |c: u8| -> isize {
            if c < 0x80 {
                1
            } else if c < 0xe0 {
                2
            } else if c < 0xf0 {
                3
            } else {
                4
            }
        };

        loop {
            let nbytes = utf8_nbytes(at(w, word2));
            if !(at(w, word2 + nbytes) == b' ' && at(w, word2 + nbytes + 1) == b'.') {
                break;
            }
            if length + 1 > word.len() {
                length = 0;
                break;
            }
            for k in 0..nbytes {
                if let Some(x) = word.get_mut(length) {
                    *x = at(w, word2 + k);
                }
                length += 1;
            }
            if let Some(x) = word.get_mut(length) {
                *x = b'.';
            }
            length += 1;
            word2 += nbytes + 3;
        }
        if length > 0 {
            let mut nbytes = 0usize;
            loop {
                let c = at(w, word2 + nbytes as isize);
                if c == 0 || c == b' ' {
                    break;
                }
                nbytes += 1;
            }
            if length + nbytes < word.len() {
                for k in 0..nbytes {
                    word[length + k] = at(w, word2 + k as isize);
                }
                word[length + nbytes] = 0;
                let found = self.lookup_dict2(tr, &word, w, word2, in_clause, ph_out, flags, end_flags, wtab);
                if found.is_some() {
                    flags[0] |= FLAG_SKIPWORDS;
                    self.dictionary_skipwords = length as i32;
                    return true;
                }
            }
        }

        length = 0;
        while length < N_WORD_BYTES - 1 {
            let c = at(w, word1);
            word1 += 1;
            if c == 0 || c == b' ' {
                break;
            }
            if c == b'.' && length > 0 && is_digit09(word[length - 1] as i32) {
                break;
            }
            word[length] = c;
            length += 1;
        }
        word[length] = 0;

        let mut found = self.lookup_dict2(tr, &word, w, word1, in_clause, ph_out, flags, end_flags, wtab);

        if flags[0] & FLAG_MAX3 != 0 {
            let t = self.tm(tr);
            if cstr(ph_out, 0) == cstr(&t.phonemes_repeat, 0) {
                t.phonemes_repeat_count += 1;
                if t.phonemes_repeat_count > 3 {
                    put(ph_out, 0, 0);
                }
            } else {
                let src = cstr(ph_out, 0).to_vec();
                strncpy0(&mut t.phonemes_repeat, &src, 20);
                t.phonemes_repeat_count = 1;
            }
        } else {
            self.tm(tr).phonemes_repeat_count = 0;
        }

        if found.is_none() && (flags[1] & FLAG_ACCENT) != 0 {
            let mut w2 = 0isize;
            if word[0] == b'_' {
                w2 = 1;
            }
            let (letter, len) = utf8_in(&word, w2);
            self.lookup_accented_letter(tr, letter, ph_out, 0);
            found = Some(w2 + len);
        }

        if found.is_none() && length >= 2 {
            put(ph_out, 0, 0);
            if (end_flags & FLAG_SUFX_E_ADDED) != 0 && word[length - 1] == b'e' {
                word[length - 1] = 0;
                found = self.lookup_dict2(tr, &word, w, word1, in_clause, ph_out, flags, end_flags, wtab);
            } else if (end_flags & SUFX_D) != 0 && word[length - 1] == word[length - 2] {
                word[length - 1] = 0;
                found = self.lookup_dict2(tr, &word, w, word1, in_clause, ph_out, flags, end_flags, wtab);
            }
        }

        if found.is_some() {
            if self.t(tr).langopts.textmode {
                flags[0] ^= FLAG_TEXTMODE;
            }
            if flags[0] & FLAG_TEXTMODE != 0 {
                if end_flags & FLAG_ALLOW_TEXTMODE != 0 {
                    let mut rep = vec![0u8, b' '];
                    rep.extend_from_slice(cstr(ph_out, 0));
                    rep.push(b' ');
                    rep.truncate(N_WORD_BYTES - 1);
                    rep.push(0);
                    self.word_replacement.fill(0);
                    let n = rep.len().min(N_WORD_BYTES);
                    self.word_replacement[..n].copy_from_slice(&rep[..n]);
                    if let Some(r) = replacement {
                        r.clear();
                        r.extend_from_slice(&self.word_replacement);
                    }
                }
                put(ph_out, 0, 0);
                return false;
            }
            return true;
        }
        put(ph_out, 0, 0);
        false
    }

    /// `Lookup`: looks up `word` (a C string), returning the dictionary flags.
    pub(crate) fn lookup(&mut self, tr: Tr, word: &[u8], ph_out: &mut [u8]) -> u32 {
        let mut flags = [0u32, FLAG_LOOKUP_SYMBOL];
        let mut replacement = Vec::new();
        let mut flags0 = if self.lookup_dict_list(tr, word, 0, false, ph_out, &mut flags, FLAG_ALLOW_TEXTMODE, None, Some(&mut replacement)) { flags[0] } else { 0 };
        if flags[0] & FLAG_TEXTMODE != 0 {
            let say_as = self.option_sayas;
            self.option_sayas = 0;
            let mut text = [0u8; 80];
            text[1] = b' ';
            text[2] = b' ';
            let rep = cstr(&replacement, 2);
            strncpy0(&mut text[3..], rep, 77);
            flags0 = self.translate_word(tr, &mut text, 3, None, false, None);
            let wp = self.word_phonemes;
            cstrcpy(ph_out, 0, cstr(&wp, 0));
            self.option_sayas = say_as;
        }
        flags0
    }

    fn lookup_flags(&mut self, tr: Tr, word: &[u8]) -> [u32; 2] {
        let mut buf = [0u8; 100];
        let mut flags = [0u32; 2];
        self.lookup_dict_list(tr, word, 0, false, &mut buf, &mut flags, 0, None, None);
        flags
    }

    /// `RemoveEnding`: removes the suffix found by the rules from the word at `w[word]`,
    /// optionally saving the whole word in `word_copy`.
    pub(crate) fn remove_ending(&mut self, tr: Tr, w: &mut [u8], word: isize, end_type: u32, word_copy: Option<&mut [u8]>) -> u32 {
        const ADD_E_EXCEPTIONS: [&[u8]; 1] = [b"ion"];
        const ADD_E_ADDITIONS: [&[u8]; 10] = [b"c", b"rs", b"ir", b"ur", b"ath", b"ns", b"u", b"spong", b"rang", b"larg"];
        let mut ending = [0u8; 50];

        let mut word_end = word;
        while at(w, word_end) != b' ' {
            if at(w, word_end) == REPLACED_E {
                put(w, word_end, b'e');
            }
            word_end += 1;
            if word_end as usize >= w.len() {
                break;
            }
        }
        let mut i = (word_end - word) as usize;
        if i >= N_WORD_BYTES {
            i = N_WORD_BYTES - 1;
        }
        if let Some(wc) = word_copy {
            for k in 0..i {
                put(wc, k as isize, at(w, word + k as isize));
            }
            put(wc, i as isize, 0);
        }

        let mut len_ending = (end_type & 0x3f) as usize;
        for _ in 0..(end_type & 0x3f) {
            word_end -= 1;
            while word_end >= word && (at(w, word_end) & 0xc0) == 0x80 {
                word_end -= 1;
                len_ending += 1;
            }
        }
        let mut k = 0usize;
        while k < len_ending && k < ending.len() - 1 {
            ending[k] = at(w, word_end + k as isize);
            put(w, word_end + k as isize, b' ');
            k += 1;
        }
        ending[k] = 0;
        word_end -= 1;

        let mut end_flags = (end_type & 0xfff0) | FLAG_SUFX;

        if (end_type & SUFX_I) != 0 && at(w, word_end) == b'i' {
            put(w, word_end, b'y');
        }

        if end_type & SUFX_E != 0 {
            let name = self.t(tr).translator_name;
            if name == l2(b'n', b'l') {
                if (at(w, word_end) & 0x80) == 0
                    && (at(w, word_end - 1) & 0x80) == 0
                    && self.is_vowel(tr, sat(w, word_end - 1))
                    && self.is_letter(tr, sat(w, word_end), LETTERGP_C)
                    && !self.is_vowel(tr, sat(w, word_end - 2))
                {
                    let a = at(w, word_end);
                    let b = at(w, word_end - 1);
                    put(w, word_end + 1, a);
                    put(w, word_end, b);
                    put(w, word_end + 2, b' ');
                }
            } else if name == l2(b'e', b'n') {
                let ends_with = |s: &[u8]| (0..s.len()).all(|k| at(w, word_end + 1 - s.len() as isize + k as isize) == s[k]);
                if self.is_letter(tr, sat(w, word_end - 1), LETTERGP_VOWEL2) && self.is_letter(tr, sat(w, word_end), 1) {
                    if !ADD_E_EXCEPTIONS.iter().any(|s| ends_with(s)) {
                        end_flags |= FLAG_SUFX_E_ADDED;
                    }
                } else if ADD_E_ADDITIONS.iter().any(|s| ends_with(s)) {
                    end_flags |= FLAG_SUFX_E_ADDED;
                }
            } else if self.t(tr).langopts.suffix_add_e != 0 {
                end_flags |= FLAG_SUFX_E_ADDED;
            }
            if end_flags & FLAG_SUFX_E_ADDED != 0 {
                let e = self.t(tr).langopts.suffix_add_e;
                utf8_out(e, w, word_end + 1);
            }
        }

        if (end_type & SUFX_V) != 0 && self.t(tr).expect_verb == 0 {
            self.tm(tr).expect_verb = 1;
        }

        let ending_s = cstr(&ending, 0);
        if ending_s == b"s" || ending_s == b"es" {
            end_flags |= FLAG_SUFX_S;
        }
        if ending[0] == b'\'' {
            end_flags &= !FLAG_SUFX;
        }
        end_flags
    }

    /// `DollarRule`: `$list` and `$p_alt` conditions look up the word so far.
    #[allow(clippy::too_many_arguments)]
    fn dollar_rule(&mut self, tr: Tr, w: &[u8], word: isize, word_start: isize, consumed: i32, group_length: i32, command: u8, failed: &mut i32, add_points: &mut i32) {
        let ix = (word - word_start + consumed as isize + group_length as isize + 1).max(0) as usize;
        let mut word_buf = vec![0u8; ix + 2];
        for k in 0..ix {
            word_buf[k] = at(w, word_start - 1 + k as isize);
        }
        word_buf[ix] = b' ';
        word_buf[ix + 1] = 0;
        let flags = self.lookup_flags(tr, &word_buf[1..]);
        if command == DOLLAR_LIST && (flags[0] & FLAG_FOUND) != 0 && (flags[1] & FLAG_ONLY) == 0 {
            *add_points = 23;
        } else if flags[0] & (1u32.wrapping_shl(BITNUM_FLAG_ALT + (command & 0xf) as u32)) != 0 {
            *add_points = 23;
        } else {
            *failed = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_matches_c() {
        assert_eq!(hash_dictionary(b"the"), {
            let mut hash: i32 = 0;
            for c in b"the" {
                hash = hash * 8 + *c as i32;
                hash = (hash & 0x3ff) ^ (hash >> 8);
            }
            ((hash + 3) & 0x3ff) as usize
        });
    }
}
