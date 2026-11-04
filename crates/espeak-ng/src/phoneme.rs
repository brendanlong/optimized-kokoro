//! The phoneme-table half of synthdata.c: selecting tables and interpreting phoneme programs
//! (`InterpretPhoneme`), which decide phoneme changes and IPA names.

use crate::data::PhonemeTab;
use crate::defs::*;
use crate::engine::{Espeak, Tr};

/// Where a condition looks: an entry of the phoneme list, or the word's previous vowel.
#[derive(Clone, Copy)]
enum Target {
    List(isize),
    PrevVowel,
}

pub(crate) fn pl(list: &[PhonemeList], i: isize) -> PhonemeList {
    if i < 0 { PhonemeList::default() } else { list.get(i as usize).copied().unwrap_or_default() }
}

fn ph_type(p: &PhonemeList) -> u8 {
    p.ph.map_or(0, |ph| ph.typ)
}

impl Espeak {
    /// `phoneme_tab[code]`.
    pub(crate) fn ph(&self, code: u8) -> Option<PhonemeTab> {
        self.phoneme_tab[code as usize]
    }

    /// `phoneme_tab[code]->...` where C would dereference NULL: a zeroed entry instead.
    pub(crate) fn phz(&self, code: u8) -> PhonemeTab {
        self.phoneme_tab[code as usize].unwrap_or_default()
    }

    fn set_up_phoneme_table(&mut self, number: i32, depth: u32) {
        let Some(table) = self.data.phoneme_tab_list.get(number as usize) else { return };
        let includes = table.includes as i32;
        if includes > 0 && depth < 16 {
            self.set_up_phoneme_table(includes - 1, depth + 1);
        }
        let data = self.data.clone();
        let table = &data.phoneme_tab_list[number as usize];
        for ph in &table.phonemes {
            let ph_code = ph.code as i32;
            self.phoneme_tab[ph_code as usize] = Some(*ph);
            if ph_code > self.n_phoneme_tab {
                for ix in (self.n_phoneme_tab + 1)..ph_code {
                    self.phoneme_tab[ix as usize] = None;
                }
                self.n_phoneme_tab = ph_code;
            }
        }
    }

    pub(crate) fn select_phoneme_table(&mut self, number: i32) {
        if self.current_phoneme_table == number {
            return;
        }
        self.n_phoneme_tab = 0;
        self.set_up_phoneme_table(number, 0);
        self.n_phoneme_tab += 1;
        self.current_phoneme_table = number;
    }

    pub(crate) fn lookup_phoneme_table(&mut self, name: &[u8]) -> i32 {
        match self.data.phoneme_tab_list.iter().position(|t| t.name == name) {
            Some(ix) => {
                self.phoneme_tab_number = ix as i32;
                ix as i32
            }
            None => -1,
        }
    }

    pub(crate) fn select_phoneme_table_name(&mut self, name: &[u8]) -> i32 {
        let ix = self.lookup_phoneme_table(name);
        if ix == -1 {
            return -1;
        }
        self.select_phoneme_table(ix);
        ix
    }

    pub(crate) fn phoneme_code(&self, mnem: u32) -> i32 {
        for ix in 0..self.n_phoneme_tab.min(256) {
            if let Some(ph) = self.phoneme_tab[ix as usize] {
                if ph.mnemonic == mnem {
                    return ph.code as i32;
                }
            }
        }
        0
    }

    pub(crate) fn lookup_phoneme_string(&self, s: &[u8]) -> i32 {
        let mut mnem = 0u32;
        for (ix, &c) in s.iter().take(4).enumerate() {
            if c == 0 {
                break;
            }
            mnem |= (c as u32) << (ix * 8);
        }
        self.phoneme_code(mnem)
    }

    fn prog(&self, ix: isize) -> u16 {
        if ix < 0 { 0 } else { self.data.phoneme_index.get(ix as usize).copied().unwrap_or(0) }
    }

    fn stress_condition(
        &self,
        tr: Option<Tr>,
        list: &[PhonemeList],
        plist: Target,
        prev_vowel: &PhonemeList,
        condition: i32,
        control: i32,
    ) -> bool {
        const CONDITION_LEVEL: [i32; 4] = [1, 2, 4, 15];
        let get = |off: isize| match plist {
            Target::List(i) => pl(list, i + off),
            Target::PrevVowel => {
                if off == 0 {
                    *prev_vowel
                } else {
                    PhonemeList::default()
                }
            }
        };
        let p0 = get(0);
        let pl_ = if self.phz(p0.phcode).typ == PH_VOWEL {
            p0
        } else {
            let p1 = get(1);
            if self.phz(p1.phcode).typ == PH_VOWEL {
                p1
            } else {
                return false;
            }
        };
        let mut stress_level = (pl_.stresslevel & 0xf) as i32;
        if let Some(tr) = tr {
            let t = self.t(tr);
            if (control & 1) != 0 && (p0.synthflags & SFLAG_DICTIONARY) != 0 && (t.langopts.param[LOPT_REDUCE] & 1) == 0
            {
                return false;
            }
            if (t.langopts.param[LOPT_REDUCE] & 0x2) != 0 && stress_level >= pl_.wordstress as i32 {
                stress_level = STRESS_IS_PRIMARY;
            }
        }
        if condition == STRESS_IS_PRIMARY {
            return stress_level >= pl_.wordstress as i32;
        }
        if condition == STRESS_IS_SECONDARY {
            if stress_level > STRESS_IS_SECONDARY {
                return true;
            }
        } else if stress_level < CONDITION_LEVEL[(condition & 3) as usize] {
            return true;
        }
        false
    }

    fn count_vowel_position(list: &[PhonemeList], mut i: isize) -> i32 {
        let mut count = 0;
        loop {
            let p = pl(list, i);
            if ph_type(&p) == PH_VOWEL {
                count += 1;
            }
            if p.sourceix != 0 || i <= 0 {
                break;
            }
            i -= 1;
        }
        count
    }

    #[allow(clippy::too_many_arguments)]
    fn interpret_condition(
        &self,
        tr: Option<Tr>,
        control: i32,
        list: &mut [PhonemeList],
        plist_ix: isize,
        prog: isize,
        prev_vowel: &mut Option<PhonemeList>,
    ) -> bool {
        let instn = (self.prog(prog) & 0xfff) as u32;
        let mut data = instn & 0xff;
        let instn2 = instn >> 8;

        if instn2 < 14 {
            let plist_this = plist_ix;
            let mut which = (instn2 % 7) as i32;
            if which == 6 {
                which = self.prog(prog + 1) as i32;
            }
            if which == 4 && pl(list, plist_ix + 1).sourceix != 0 {
                return false;
            }
            if which == 5 && pl(list, plist_ix).sourceix != 0 {
                return false;
            }
            if which == 6 && (pl(list, plist_ix + 1).sourceix != 0 || pl(list, plist_ix + 2).sourceix != 0) {
                return false;
            }

            let mut check_endtype = false;
            let mut target = Target::List(plist_ix);
            match which {
                0 | 5 => {
                    target = Target::List(plist_ix - 1);
                    check_endtype = true;
                }
                1 => {}
                2 | 4 => target = Target::List(plist_ix + 1),
                3 | 6 => target = Target::List(plist_ix + 2),
                7 => {
                    let mut w = 1isize;
                    loop {
                        let p = pl(list, plist_ix + w);
                        if p.sourceix != 0 || plist_ix + w >= list.len() as isize {
                            return false;
                        }
                        if self.phz(p.phcode).typ == PH_VOWEL {
                            target = Target::List(plist_ix + w);
                            // C reuses `which` as the loop counter, so the prevPh check below sees it
                            which = w as i32;
                            break;
                        }
                        w += 1;
                    }
                }
                8 => {
                    match prev_vowel {
                        Some(pv) if pv.ph.is_some() => {}
                        _ => return false,
                    }
                    target = Target::PrevVowel;
                    check_endtype = true;
                }
                9 => {
                    for ix in 1..=3 {
                        if pl(list, plist_ix + ix).sourceix != 0 {
                            return false;
                        }
                    }
                    target = Target::List(plist_ix + 3);
                }
                10 => {
                    if pl(list, plist_ix).sourceix != 0 || pl(list, plist_ix - 1).sourceix != 0 {
                        return false;
                    }
                    target = Target::List(plist_ix - 2);
                    check_endtype = true;
                }
                _ => {}
            }

            if which == 0 || which == 5 {
                if let Target::List(i) = target {
                    if pl(list, i).phcode == 1 {
                        target = Target::List(i - 1);
                    }
                }
            }

            if control & 0x100 != 0 {
                match target {
                    Target::List(i) => {
                        let ph = self.ph(pl(list, i).phcode);
                        if i >= 0 {
                            if let Some(p) = list.get_mut(i as usize) {
                                p.ph = ph;
                            }
                        }
                    }
                    Target::PrevVowel => {
                        if let Some(pv) = prev_vowel {
                            pv.ph = self.ph(pv.phcode);
                        }
                    }
                }
            }
            let empty = PhonemeList::default();
            let prev_vowel_copy = prev_vowel.unwrap_or(empty);
            let tp = match target {
                Target::List(i) => pl(list, i),
                Target::PrevVowel => prev_vowel_copy,
            };
            let ph = tp.ph.unwrap_or_default();

            if instn2 < 7 {
                if self.phz(data as u8).mnemonic == ph.mnemonic {
                    return true;
                }
                if check_endtype && ph.typ == PH_VOWEL {
                    return data == ph.end_type as u32;
                }
                return data == ph.start_type as u32;
            }

            data = instn & 0x1f;
            match instn & 0xe0 {
                CONDITION_IS_PHONEME_TYPE => return ph.typ as u32 == data,
                CONDITION_IS_PLACE_OF_ARTICULATION => return ((ph.phflags >> 16) & 0xf) == data,
                CONDITION_IS_PHFLAG_SET => return (ph.phflags & (1u32 << data)) != 0,
                CONDITION_IS_OTHER => match data {
                    0..=4 => return self.stress_condition(tr, list, target, &prev_vowel_copy, data as i32, 0),
                    IS_BREAK => return ph.typ == PH_PAUSE || (pl(list, plist_this).synthflags & SFLAG_NEXT_PAUSE) != 0,
                    IS_WORD_START => return tp.sourceix != 0,
                    IS_WORD_END => {
                        return match target {
                            Target::List(i) => {
                                let n = pl(list, i + 1);
                                n.sourceix != 0 || ph_type(&n) == PH_PAUSE
                            }
                            Target::PrevVowel => true,
                        };
                    }
                    IS_AFTER_STRESS => {
                        let Target::List(mut i) = target else { return false };
                        if pl(list, i).sourceix != 0 {
                            return false;
                        }
                        loop {
                            i -= 1;
                            if i < 0 {
                                return false;
                            }
                            let p = pl(list, i);
                            if (p.stresslevel & 0xf) >= 4 {
                                return true;
                            }
                            if p.sourceix != 0 {
                                break;
                            }
                        }
                    }
                    IS_NOT_VOWEL => return ph.typ != PH_VOWEL,
                    IS_FINAL_VOWEL => {
                        let Target::List(mut i) = target else { return true };
                        loop {
                            i += 1;
                            let p = pl(list, i);
                            if p.sourceix != 0 || i >= list.len() as isize {
                                return true;
                            }
                            if ph_type(&p) == PH_VOWEL {
                                return false;
                            }
                        }
                    }
                    IS_VOICED => return ph.typ == PH_VOWEL || ph.typ == PH_LIQUID || (ph.phflags & PH_VOICED) != 0,
                    IS_FIRST_VOWEL | IS_SECOND_VOWEL => {
                        let n = match target {
                            Target::List(i) => Self::count_vowel_position(list, i),
                            Target::PrevVowel => (ph_type(&prev_vowel_copy) == PH_VOWEL) as i32,
                        };
                        return n == if data == IS_FIRST_VOWEL { 1 } else { 2 };
                    }
                    IS_TRANSLATION_GIVEN => return (tp.synthflags & SFLAG_DICTIONARY) != 0,
                    _ => {}
                },
                _ => {}
            }
            false
        } else if instn2 == 0xf {
            data == 1 && (control & 1) != 0
        } else {
            false
        }
    }

    fn num_instn_words(&self, prog: isize) -> isize {
        const N_WORDS: [isize; 16] = [0, 1, 0, 0, 1, 1, 0, 1, 1, 2, 4, 0, 0, 0, 0, 0];
        let instn = self.prog(prog);
        let instn_type = (instn >> 12) as usize;
        let n = N_WORDS[instn_type];
        if n > 0 {
            return n;
        }
        match instn_type {
            0 => {
                if ((instn & 0xf00) >> 8) as usize == I_IPA_NAME {
                    return (((instn & 0xff) as isize) + 1) / 2 + 1;
                }
                1
            }
            6 => {
                let type2 = (instn & 0xf00) >> 9;
                if type2 == 5 || type2 == 6 { 12 } else { 1 }
            }
            2 | 3 => {
                let n = instn & 0x0f00;
                if n == 0x600 || n == 0x0d00 { 2 } else { 1 }
            }
            _ => {
                let instn2 = self.prog(prog + 2);
                if (instn2 >> 12) == 0xf {
                    return 4;
                }
                if instn2 == INSTN_CONTINUE {
                    return 3;
                }
                2
            }
        }
    }

    fn switch_on_vowel_type(
        &self,
        list: &[PhonemeList],
        plist_ix: isize,
        phdata: &mut PhonemeData,
        prog: &mut isize,
        instn_type: usize,
    ) {
        let voweltype = if instn_type == 2 {
            phdata.pd_control |= PD_FORNEXTPH;
            pl(list, plist_ix + 1).ph.map_or(0, |p| p.start_type) as i32
        } else {
            pl(list, plist_ix - 1).ph.map_or(0, |p| p.end_type) as i32
        } - PHON_VOWELTYPES as i32;
        if (0..6).contains(&voweltype) {
            let p = *prog + voweltype as isize * 2;
            phdata.sound_addr[instn_type] = ((((self.prog(p + 1) & 0xf) as i32) << 16) + self.prog(p + 2) as i32) * 4;
            phdata.sound_param[instn_type] = ((self.prog(p + 1) >> 4) & 0xff) as u8 as i8 as i32;
        }
        *prog += 12;
    }

    /// `InterpretPhoneme`: runs the program of `list[ix].ph`.
    pub(crate) fn interpret_phoneme(
        &self,
        tr: Option<Tr>,
        control: i32,
        list: &mut [PhonemeList],
        ix: isize,
        phdata: &mut PhonemeData,
        mut worddata: Option<&mut Option<PhonemeList>>,
    ) {
        const N_RETURN: usize = 10;
        let mut n_return = 0usize;
        let mut return_addr = [0isize; N_RETURN];
        let this = pl(list, ix);
        let ph = this.ph.unwrap_or_default();

        if let Some(wd) = worddata.as_deref_mut() {
            if this.sourceix != 0 {
                if let Some(pv) = wd {
                    pv.ph = None;
                }
            }
        }

        *phdata = PhonemeData::default();
        phdata.pd_param[I_SET_LENGTH] = ph.std_length as i32;
        phdata.pd_param[I_LENGTH_MOD] = ph.length_mod as i32;

        if ph.program == 0 {
            return;
        }

        let mut no_prev_vowel = None;
        let mut end_flag = 0;
        let mut prog = ph.program as isize;
        let mut steps = 0u32;
        while end_flag != 1 {
            steps += 1;
            if steps > 100_000 {
                break;
            }
            let mut instn = self.prog(prog);
            let instn2 = ((instn >> 8) & 0xf) as usize;
            match instn >> 12 {
                0 => {
                    let data = (instn & 0xff) as i32;
                    if instn2 == 0 {
                        if data as u16 == INSTN_RETURN {
                            end_flag = 1;
                        }
                    } else if instn2 == I_APPEND_IFNEXTVOWEL {
                        if self.phz(pl(list, ix + 1).phcode).typ == PH_VOWEL {
                            phdata.pd_param[I_APPEND_PHONEME] = data;
                        }
                    } else if instn2 == I_ADD_LENGTH {
                        let data = if data & 0x80 != 0 { -(0x100 - data) } else { data };
                        phdata.pd_param[I_SET_LENGTH] += data;
                    } else if instn2 == I_IPA_NAME {
                        let mut k = 0usize;
                        while (k as i32) < data && k < 16 {
                            prog += 1;
                            let w = self.prog(prog);
                            phdata.ipa_string[k] = (w >> 8) as u8;
                            phdata.ipa_string[k + 1] = (w & 0xff) as u8;
                            k += 2;
                        }
                        phdata.ipa_string[k] = 0;
                    } else if instn2 < N_PHONEME_DATA_PARAM {
                        phdata.pd_param[instn2] = data;
                        if instn2 == I_CHANGE_PHONEME && (control & 0x100) != 0 {
                            end_flag = 1;
                        }
                    }
                }
                1 => {
                    if tr.is_some() && instn2 < 8 {
                        let pv = worddata.as_deref().copied().flatten().unwrap_or_default();
                        if self.stress_condition(tr, list, Target::List(ix), &pv, (instn2 & 7) as i32, 1) {
                            phdata.pd_param[I_CHANGE_PHONEME] = (instn & 0xff) as i32;
                            end_flag = 1;
                        }
                    }
                }
                2 | 3 => {
                    let mut or_flag = 0u16;
                    let mut truth = true;
                    while (instn & 0xe000) == 0x2000 {
                        let pv: &mut Option<PhonemeList> = match worddata.as_deref_mut() {
                            Some(w) => w,
                            None => &mut no_prev_vowel,
                        };
                        let mut truth2 = self.interpret_condition(tr, control, list, ix, prog, pv);
                        prog += self.num_instn_words(prog);
                        if self.prog(prog) == I_NOT {
                            truth2 = !truth2;
                            prog += 1;
                        }
                        if or_flag != 0 {
                            truth = truth || truth2;
                        } else {
                            truth = truth && truth2;
                        }
                        or_flag = instn & 0x1000;
                        instn = self.prog(prog);
                    }
                    if !truth {
                        if (instn & 0xf800) == I_JUMP_FALSE {
                            prog += (instn & 0xff) as isize;
                        } else {
                            prog += self.num_instn_words(prog);
                            if (self.prog(prog) & 0xfe00) == 0x6000 {
                                prog += 1;
                            }
                        }
                    }
                    prog -= 1;
                }
                6 => match instn2 >> 1 {
                    0 => prog += (instn & 0xff) as isize - 1,
                    5 => self.switch_on_vowel_type(list, ix, phdata, &mut prog, 2),
                    6 => self.switch_on_vowel_type(list, ix, phdata, &mut prog, 3),
                    _ => {}
                },
                9 => {
                    let data = (((instn & 0xf) as isize) << 16) + self.prog(prog + 1) as isize;
                    prog += 1;
                    if instn2 == 1 && n_return < N_RETURN {
                        return_addr[n_return] = prog;
                        n_return += 1;
                        prog = data - 1;
                    }
                }
                10 => prog += 3,
                11..=15 => {
                    let k = ((instn >> 12) - 11) as usize;
                    phdata.sound_addr[k] = (((instn & 0xf) as i32) << 18) + ((self.prog(prog + 1) as i32) << 2);
                    let param_sc = ((instn >> 4) & 0xff) as u8 as i8 as i32;
                    phdata.sound_param[k] = ((instn >> 4) & 0xff) as i32;
                    prog += 1;
                    if self.prog(prog + 1) != INSTN_CONTINUE {
                        if k < 2 {
                            end_flag = 1;
                            if (self.prog(prog + 1) >> 12) == 0xf {
                                end_flag = 2;
                            }
                        } else if k == PD_ADDWAV {
                            end_flag -= 1;
                        }
                        if k == PD_VWLSTART || k == PD_VWLEND {
                            phdata.sound_param[k] = param_sc;
                        }
                    }
                }
                _ => {}
            }
            if end_flag == 1 && n_return > 0 {
                end_flag = 0;
                n_return -= 1;
                prog = return_addr[n_return];
            }
            prog += 1;
        }

        if let Some(wd) = worddata {
            if this.typ == PH_VOWEL {
                *wd = Some(pl(list, ix));
            }
        }
        if ix >= 0 {
            if let Some(p) = list.get_mut(ix as usize) {
                p.std_length = phdata.pd_param[I_SET_LENGTH] as u8;
            }
        }
    }

    /// `InterpretPhoneme2`: an isolated phoneme between pauses.
    pub(crate) fn interpret_phoneme2(&self, phcode: u8, phdata: &mut PhonemeData) {
        let mut plist = [PhonemeList::default(); 4];
        for p in &mut plist {
            p.phcode = PHON_PAUSE;
            p.ph = self.ph(PHON_PAUSE);
        }
        plist[1].phcode = phcode;
        plist[1].ph = self.ph(phcode);
        plist[2].sourceix = 1;
        self.interpret_phoneme(None, 0, &mut plist, 1, phdata, None);
    }
}
