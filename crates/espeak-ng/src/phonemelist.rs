//! phonemelist.c: from the first-stage list (`ph_list2`) to `phoneme_list`, applying voice
//! replacements, regressive voicing and the phonemes' own programs.

use crate::defs::*;
use crate::engine::{Espeak, Tr};
use crate::phoneme::pl;

const PAUSE_PHONEMES: [u8; 8] = [0, PHON_PAUSE_VSHORT, PHON_PAUSE_SHORT, PHON_PAUSE, PHON_PAUSE_LONG, PHON_GLOTTALSTOP, PHON_PAUSE_LONG, PHON_PAUSE_LONG];

/// Copies the `PHONEME_LIST2` part of an entry, leaving the rest as it was (C's `memcpy`).
fn copy2(dst: &mut PhonemeList, src: &PhonemeList2) {
    dst.synthflags = src.synthflags;
    dst.phcode = src.phcode;
    dst.stresslevel = src.stresslevel;
    dst.sourceix = src.sourceix;
    dst.wordstress = src.wordstress;
    dst.tone_ph = src.tone_ph;
}

fn p2(list: &[PhonemeList2], i: isize) -> PhonemeList2 {
    if i < 0 { PhonemeList2::default() } else { list.get(i as usize).copied().unwrap_or_default() }
}

impl Espeak {
    /// `SubstitutePhonemes`: copies `ph_list2` into `ph_list3`, applying the voice's
    /// `replace` lines.
    fn substitute_phonemes(&mut self) -> i32 {
        let mut n_plist_out = 0usize;
        let mut next: Option<crate::data::PhonemeTab> = None;
        let mut deleted_sourceix: i32 = -1;
        let n = self.n_ph_list2.max(0) as usize;
        let mut ix = 0usize;
        while ix < n && n_plist_out < N_PHONEME_LIST {
            if deleted_sourceix != -1 {
                self.ph_list2[ix].sourceix = deleted_sourceix as u16;
                deleted_sourceix = -1;
            }
            let cur = self.ph_list2[ix];
            if cur.phcode == PHON_SWITCH {
                self.select_phoneme_table(cur.tone_ph as i32);
            }
            if (cur.synthflags & SFLAG_SWITCHED_LANG) == 0 {
                if ix + 1 < n {
                    next = self.ph(self.ph_list2[ix + 1].phcode);
                }
                let following = p2(&self.ph_list2, ix as isize + 1);
                let word_end = ix + 1 == n || following.sourceix != 0 || next.is_some_and(|nx| nx.typ == PH_PAUSE);
                for k in 0..self.replace_phonemes.len() {
                    let rp = self.replace_phonemes[k];
                    let plist2 = &mut self.ph_list2[ix];
                    if plist2.phcode == rp.old_ph {
                        let replace_flags = rp.typ as i32;
                        if (replace_flags & 1) != 0 && !word_end {
                            continue;
                        }
                        if (replace_flags & 2) != 0 && (plist2.stresslevel & 0x7) > 3 {
                            continue;
                        }
                        if (replace_flags & 4) != 0 && plist2.sourceix == 0 {
                            continue;
                        }
                        plist2.phcode = rp.new_ph;
                        let unstressed = self.phoneme_tab[rp.new_ph as usize].is_some_and(|p| (p.phflags & PH_UNSTRESSED) != 0);
                        let plist2 = &mut self.ph_list2[ix];
                        if plist2.stresslevel > 1 && unstressed {
                            plist2.stresslevel = 0;
                        }
                        break;
                    }
                }
                if self.ph_list2[ix].phcode == 0 {
                    deleted_sourceix = self.ph_list2[ix].sourceix as i32;
                    ix += 1;
                    continue;
                }
            }
            let src = self.ph_list2[ix];
            let ph = self.ph(src.phcode);
            let out = &mut self.ph_list3[n_plist_out];
            copy2(out, &src);
            out.ph = ph;
            out.typ = ph.map_or(0, |p| p.typ);
            n_plist_out += 1;
            ix += 1;
        }
        n_plist_out as i32
    }

    /// `MakePhonemeList`.
    pub(crate) fn make_phoneme_list(&mut self, tr: Tr, post_pause: i32, start_sentence: bool) {
        let _ = post_pause;
        let mut start_sentence = start_sentence;
        let mut ix = 0usize;
        let mut insert_ph: u8 = 0;
        let mut ph: Option<crate::data::PhonemeTab>;
        let mut next: Option<crate::data::PhonemeTab>;
        let mut unstress_count = 0;
        let mut word_stress: u8 = 0;
        let mut delete_count = 0usize;
        let mut word_start: isize;
        let mut phdata = PhonemeData::default();
        let mut start_of_clause = true;
        let mut plist3_inserted: Option<usize> = None;
        let mut worddata: Option<PhonemeList> = None;

        let n2 = self.n_ph_list2;
        let end_sourceix = p2(&self.ph_list2, n2 as isize - 1).sourceix;

        let mut max_stress = 0;
        let mut j: isize = n2 as isize - 3;
        while j >= 0 {
            let e = self.ph_list2[j as usize];
            if (e.stresslevel & 0x7f) as i32 > max_stress {
                max_stress = (e.stresslevel & 0x7f) as i32;
            }
            if e.sourceix != 0 {
                break;
            }
            j -= 1;
        }
        if max_stress < 4 {
            loop {
                j -= 1;
                if j < 0 {
                    break;
                }
                let e = &mut self.ph_list2[j as usize];
                if e.synthflags & SFLAG_PROMOTE_STRESS != 0 {
                    e.stresslevel = 4;
                    break;
                }
                if e.stresslevel >= 4 {
                    break;
                }
            }
        }

        let tr_tab = self.t(tr).phoneme_tab_ix;
        let mut current_phoneme_tab = tr_tab;
        let mut deleted_sourceix: i32 = -1;
        for j in 0..n2.max(0) as usize {
            if current_phoneme_tab != tr_tab {
                self.ph_list2[j].synthflags |= SFLAG_SWITCHED_LANG;
            }
            if delete_count > 0 {
                self.ph_list2[j - delete_count] = self.ph_list2[j];
                if deleted_sourceix != -1 {
                    self.ph_list2[j - delete_count].sourceix = deleted_sourceix as u16;
                    deleted_sourceix = -1;
                }
            }
            let e = self.ph_list2[j];
            if e.phcode == PHON_SWITCH {
                let n1 = p2(&self.ph_list2, j as isize + 1);
                let n2e = p2(&self.ph_list2, j as isize + 2);
                if (e.synthflags & SFLAG_EMBEDDED) == 0
                    && (e.tone_ph as i32 == current_phoneme_tab || n1.phcode == PHON_SWITCH || (n1.phcode == PHON_PAUSE && n2e.phcode == PHON_SWITCH))
                {
                    if deleted_sourceix == -1 && e.sourceix != 0 {
                        deleted_sourceix = e.sourceix as i32;
                    }
                    delete_count += 1;
                } else {
                    current_phoneme_tab = e.tone_ph as i32;
                }
            }
        }
        self.n_ph_list2 -= delete_count as i32;

        self.select_phoneme_table(current_phoneme_tab);

        let regression = self.t(tr).langopts.param[LOPT_REGRESSIVE_VOICING];
        if regression != 0 {
            self.set_regressive_voicing(regression, tr);
        }

        self.select_phoneme_table(tr_tab);
        let n_ph_list3 = (self.substitute_phonemes() - 2).max(0) as usize;

        let mut j = 0usize;
        while j < n_ph_list3 && ix < N_PHONEME_LIST - 3 {
            if self.ph_list3[j].sourceix != 0 {
                word_stress = 0;
                let mut nextw = j;
                while nextw < n_ph_list3 {
                    if self.ph_list3[nextw].stresslevel > word_stress {
                        word_stress = self.ph_list3[nextw].stresslevel;
                    }
                    nextw += 1;
                    if self.ph_list3.get(nextw).is_some_and(|p| p.sourceix != 0) {
                        break;
                    }
                }
                for k in j..nextw {
                    self.ph_list3[k].wordstress = word_stress;
                }
                j = nextw;
            } else {
                j += 1;
            }
        }

        ph = self.ph(PHON_PAUSE);
        self.ph_list3[0].ph = ph;
        word_start = 1;

        self.select_phoneme_table(tr_tab);
        let mut list3 = std::mem::take(&mut self.ph_list3);
        let mut j: isize = 0;
        let mut steps = 0;
        while insert_ph != 0 || (j < n_ph_list3 as isize && ix < N_PHONEME_LIST - 3) {
            steps += 1;
            if steps > 4 * N_PHONEME_LIST || j as usize + 3 >= list3.len() {
                break;
            }
            let mut plist3 = j as usize;
            let mut inserted = false;
            let mut deleted = false;
            if insert_ph != 0 {
                next = self.ph(list3[plist3].phcode);
                j -= 1;
                plist3 = j.max(0) as usize;
                plist3_inserted = Some(plist3);
                if j > 0 {
                    let mut k: isize;
                    if word_start > 0 {
                        k = word_start;
                        word_start -= 1;
                    } else {
                        k = 2;
                    }
                    while k <= j {
                        list3[(k - 1) as usize] = list3[k as usize];
                        k += 1;
                    }
                }
                list3[plist3] = PhonemeList::default();
                list3[plist3].phcode = insert_ph;
                ph = self.ph(insert_ph);
                list3[plist3].ph = ph;
                insert_ph = 0;
                inserted = true;
            } else {
                if list3[plist3].sourceix != 0 {
                    word_start = j;
                }
                ph = self.ph(list3[plist3].phcode);
                list3[plist3].ph = ph;
                if list3[plist3].phcode == PHON_SWITCH {
                    let t = list3[plist3].tone_ph as i32;
                    self.select_phoneme_table(t);
                }
                next = self.ph(list3[plist3 + 1].phcode);
                list3[plist3 + 1].ph = next;
            }

            let Some(mut phv) = ph else {
                j += 1;
                continue;
            };

            self.interpret_phoneme(Some(tr), 0x100, &mut list3, plist3 as isize, &mut phdata, Some(&mut worddata));

            let alternative = phdata.pd_param[I_REPLACE_NEXT_PHONEME];
            if alternative > 0 {
                let a = alternative as u8;
                let e = &mut list3[plist3 + 1];
                e.ph = self.ph(a);
                e.phcode = a;
                e.typ = self.phz(a).typ;
                next = self.ph(a);
            }

            let alternative = phdata.pd_param[I_INSERT_PHONEME];
            if alternative > 0 && !inserted {
                let ph2 = phv;
                insert_ph = list3[plist3].phcode;
                let a = alternative as u8;
                phv = self.phz(a);
                list3[plist3].ph = self.ph(a);
                list3[plist3].phcode = a;
                self.reinterpret_phoneme(&phv, &ph2, &mut list3, plist3, tr, &mut phdata, &mut worddata);
            }

            let alternative = phdata.pd_param[I_CHANGE_PHONEME];
            if alternative > 0 {
                let ph2 = phv;
                let a = alternative as u8;
                phv = self.phz(a);
                list3[plist3].ph = self.ph(a);
                list3[plist3].phcode = a;
                if alternative == 1 {
                    deleted = true;
                } else {
                    self.reinterpret_phoneme(&phv, &ph2, &mut list3, plist3, tr, &mut phdata, &mut worddata);
                }
            }

            if phv.typ == PH_VOWEL && !deleted {
                if list3[plist3].stresslevel <= 1 {
                    unstress_count += 1;
                    if self.t(tr).langopts.stress_flags & 0x08 != 0 {
                        let mut p = plist3 + 1;
                        while p < list3.len() && list3[p].typ != PH_PAUSE {
                            if list3[p].typ == PH_VOWEL {
                                if list3[p].stresslevel <= 1 {
                                    if list3[plist3].wordstress < 4 {
                                        list3[plist3].stresslevel = 0;
                                    }
                                    if list3[p].wordstress < 4 {
                                        list3[p].stresslevel = 0;
                                    }
                                }
                                break;
                            }
                            p += 1;
                        }
                    } else if unstress_count > 1 && (unstress_count & 1) == 0 {
                        if (self.t(tr).langopts.stress_flags & S_NO_DIM) != 0 || (word_stress > 3 && list3[plist3 + 1].sourceix != 0) {
                            unstress_count = 1;
                        } else {
                            list3[plist3].stresslevel = 0;
                        }
                    }
                } else {
                    unstress_count = 0;
                }
            }

            if list3[plist3 + 1].synthflags & SFLAG_LENGTHEN != 0 {
                // strchr(types_double, type) also matches the terminator, i.e. phPAUSE
                let nt = next.map_or(0, |n| n.typ);
                if j > 0 && matches!(nt, PH_FRICATIVE | PH_VFRICATIVE | PH_NASAL | PH_LIQUID | 0) {
                    insert_ph = next.map_or(0, |n| n.code);
                    list3[plist3 + 1].synthflags ^= SFLAG_LENGTHEN;
                }
            }

            if list3[plist3 + 1].sourceix != 0 {
                let vowel_pause = self.t(tr).langopts.vowel_pause;
                if vowel_pause != 0 && phv.typ != PH_PAUSE {
                    if phv.typ != PH_VOWEL && (vowel_pause & 0x200) != 0 {
                        insert_ph = PHON_PAUSE_NOLINK;
                    }
                    if next.is_some_and(|n| n.typ == PH_VOWEL) {
                        let x = vowel_pause & 0x0c;
                        if x != 0 {
                            insert_ph = if x == 0xc { PHON_PAUSE_NOLINK } else { PHON_PAUSE_VSHORT };
                        }
                        let x = vowel_pause & 0x03;
                        if phv.typ == PH_VOWEL && x != 0 {
                            insert_ph = if x == 2 { PHON_PAUSE_SHORT } else { PHON_PAUSE_VSHORT };
                        }
                        if list3[plist3 + 1].stresslevel >= 4 && (vowel_pause & 0x100) != 0 {
                            insert_ph = PHON_PAUSE_SHORT;
                        }
                    }
                }
                if plist3_inserted != Some(plist3) && ix > 0 {
                    let x = (self.t(tr).langopts.word_gap & 0x7) as usize;
                    if x != 0 && (x > 1 || (insert_ph != PHON_PAUSE_SHORT && insert_ph != PHON_PAUSE_NOLINK)) {
                        insert_ph = PAUSE_PHONEMES[x];
                    }
                    if self.option_wordgap > 0 {
                        insert_ph = PHON_PAUSE_LONG;
                    }
                }
            }

            let c2 = list3[plist3 + 2].phcode;
            list3[plist3 + 2].ph = self.ph(c2);

            if insert_ph == 0 && phdata.pd_param[I_APPEND_PHONEME] != 0 {
                insert_ph = phdata.pd_param[I_APPEND_PHONEME] as u8;
            }

            if !deleted {
                let pause_short_ph = self.ph(PHON_PAUSE_SHORT);
                let e3 = list3[plist3];
                let out = &mut self.phoneme_list[ix];
                out.ph = Some(phv);
                out.typ = phv.typ;
                out.synthflags = e3.synthflags;
                out.stresslevel = e3.stresslevel & 0xf;
                out.wordstress = e3.wordstress;
                out.tone_ph = e3.tone_ph;
                out.sourceix = 0;
                out.phcode = phv.code;
                if e3.sourceix != 0 {
                    out.sourceix = e3.sourceix;
                    out.newword = PHLIST_START_OF_WORD;
                    if start_sentence {
                        out.newword |= PHLIST_START_OF_SENTENCE;
                        start_sentence = false;
                    }
                    if start_of_clause {
                        out.newword |= PHLIST_START_OF_CLAUSE;
                        start_of_clause = false;
                    }
                } else {
                    out.newword = 0;
                }
                if phv.code == PHON_PAUSE_LONG && self.option_wordgap > 0 && list3[plist3 + 1].sourceix != 0 {
                    out.ph = pause_short_ph;
                }
                ix += 1;
            }
            j += 1;
        }
        self.ph_list3 = list3;

        let pause = self.ph(PHON_PAUSE);
        let pause_short = self.ph(PHON_PAUSE_SHORT);
        {
            let e = &mut self.phoneme_list[ix];
            e.newword = PHLIST_END_OF_CLAUSE;
            e.phcode = PHON_PAUSE;
            e.typ = PH_PAUSE;
            e.sourceix = end_sourceix;
            e.synthflags = 0;
            e.ph = pause;
        }
        ix += 1;
        {
            let e = &mut self.phoneme_list[ix];
            e.newword = 0;
            e.phcode = PHON_PAUSE;
            e.typ = PH_PAUSE;
            e.sourceix = 0;
            e.synthflags = 0;
            e.ph = pause_short;
        }
        ix += 1;
        self.n_phoneme_list = ix as i32;
        self.select_phoneme_table(tr_tab);
        self.phoneme_list[N_PHONEME_LIST].ph = None;
        self.phoneme_list[N_PHONEME_LIST].sourceix = 1;
    }

    fn set_regressive_voicing(&mut self, regression: i32, tr: Tr) {
        let mut stop_propagation = false;
        let mut voicing = 0;
        let n = self.n_ph_list2.max(0) as usize;
        for j in (0..n).rev() {
            if self.ph_list2[j].phcode == PHON_SWITCH {
                let mut k = j as isize - 1;
                while k >= 0 {
                    if self.ph_list2[k as usize].phcode == PHON_SWITCH {
                        break;
                    }
                    k -= 1;
                }
                if k >= 0 {
                    let t = self.ph_list2[k as usize].tone_ph as i32;
                    self.select_phoneme_table(t);
                } else {
                    let t = self.t(tr).phoneme_tab_ix;
                    self.select_phoneme_table(t);
                }
            }
            let Some(ph) = self.ph(self.ph_list2[j].phcode) else { continue };
            if self.ph_list2[j].synthflags & SFLAG_SWITCHED_LANG != 0 {
                stop_propagation = false;
                voicing = 0;
                if regression & 0x100 != 0 {
                    voicing = 1;
                }
                continue;
            }
            let typ = ph.typ;
            if regression & 0x2 != 0 && ((ph.mnemonic & 0xff) == b'v' as u32 || (ph.mnemonic & 0xff) == b'R' as u32) {
                stop_propagation = true;
                if regression & 0x10 != 0 {
                    voicing = 0;
                }
            }
            if typ == PH_STOP || typ == PH_FRICATIVE {
                if voicing == 0 && (regression & 0xf) != 0 {
                    voicing = 1;
                } else if voicing == 2 && ph.end_type != 0 {
                    self.ph_list2[j].phcode = ph.end_type;
                }
            } else if typ == PH_VSTOP || typ == PH_VFRICATIVE {
                if voicing == 0 && (regression & 0xf) != 0 {
                    voicing = 2;
                } else if voicing == 1 && ph.end_type != 0 {
                    self.ph_list2[j].phcode = ph.end_type;
                }
            } else if regression & 0x8 != 0 {
                if typ == PH_PAUSE || typ == PH_VOWEL {
                    voicing = 0;
                }
            } else {
                voicing = 0;
            }
            if stop_propagation {
                voicing = 0;
                stop_propagation = false;
            }
            if self.ph_list2[j].sourceix != 0 {
                if regression & 0x04 != 0 {
                    voicing = 0;
                }
                if regression & 0x100 != 0 && voicing == 0 {
                    voicing = 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reinterpret_phoneme(&mut self, ph: &crate::data::PhonemeTab, ph2: &crate::data::PhonemeTab, list3: &mut [PhonemeList], plist3: usize, tr: Tr, phdata: &mut PhonemeData, worddata: &mut Option<PhonemeList>) {
        if ph.typ == PH_VOWEL {
            list3[plist3].synthflags |= SFLAG_SYLLABLE;
            if ph2.typ != PH_VOWEL {
                list3[plist3].stresslevel = 0;
            }
        } else {
            list3[plist3].synthflags &= !SFLAG_SYLLABLE;
        }
        self.interpret_phoneme(Some(tr), 0x100, list3, plist3 as isize, phdata, Some(worddata));
        let _ = pl;
    }
}
