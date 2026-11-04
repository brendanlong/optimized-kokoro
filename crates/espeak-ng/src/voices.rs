//! The parts of voices.c and langopts.c that select a voice: reading its file, building the
//! translator and loading its dictionary.

use crate::common::*;
use crate::data::{Error, Result, parse_c_int, split_key, strip_line};
use crate::defs::*;
use crate::engine::{Espeak, ReplacePhonemes};
use crate::tr_languages::{process_language_options, select_translator};

/// Keys of `langopts_tab` (voice.h).
fn langopts_key(key: &str) -> Option<&'static str> {
    const KEYS: [&str; 21] = [
        "apostrophe",
        "brackets",
        "bracketsAnnounced",
        "dict_min",
        "dictrules",
        "intonation",
        "l_dieresis",
        "l_prefix",
        "l_regressive_v",
        "l_unpronouncable",
        "l_sonorant_min",
        "lowercaseSentence",
        "numbers",
        "spellingStress",
        "stressAdd",
        "stressAmp",
        "stressLength",
        "stressOpt",
        "stressRule",
        "tunes",
        "words",
    ];
    KEYS.iter().copied().find(|&k| k == key)
}

/// `ReadNumbers`: a list of bit numbers.
fn read_numbers(p: &[u8], flags: &mut i32, max_value: i32) {
    let mut i = 0isize;
    while at(p, i) != 0 {
        while c_isspace(sat(p, i)) {
            i += 1;
        }
        let n = atoi(p, i);
        if n > 0 {
            i += 1;
            if n < max_value {
                *flags |= 1i32.wrapping_shl(n as u32);
            }
        }
        while c_isalnum(sat(p, i)) {
            i += 1;
        }
    }
}

/// `sscanf(p, "%d %d %d ...")`: the integers up to the first token that isn't one.
fn scan_ints(p: &str, max: usize) -> Vec<i32> {
    let mut out = Vec::new();
    for tok in p.split_ascii_whitespace().take(max) {
        match parse_c_int(tok) {
            Some(v) => out.push(v),
            None => break,
        }
    }
    out
}

impl Espeak {
    /// `LoadLanguageOptions`.
    fn load_language_options(&mut self, key: &str, value: &str) {
        let Some(tr) = self.translator.as_deref_mut() else { return };
        let vb: Vec<u8> = value.bytes().chain(std::iter::once(0)).collect();
        match key {
            "dict_min" => {
                if let Some(&n) = scan_ints(value, 1).first() {
                    tr.dict_min_size = n;
                }
            }
            "dictrules" => read_numbers(&vb, &mut tr.dict_condition, 32),
            "intonation" => {
                if let Some(&n) = scan_ints(value, 1).first() {
                    self.option_tone_flags = n;
                    if (n & 0xff) != 0 {
                        tr.langopts.intonation_group = n & 0xff;
                    }
                }
            }
            "numbers" => {
                let mut i = 0isize;
                while at(&vb, i) != 0 {
                    while c_isspace(sat(&vb, i)) {
                        i += 1;
                    }
                    let n = atoi(&vb, i);
                    if n > 0 {
                        i += 1;
                        if n < 32 {
                            tr.langopts.numbers |= 1 << n;
                        } else if n < 64 {
                            tr.langopts.numbers2 |= 1 << (n - 32);
                        }
                    }
                    while c_isalnum(sat(&vb, i)) {
                        i += 1;
                    }
                }
                process_language_options(&mut tr.langopts);
            }
            "lowercaseSentence" => tr.langopts.lowercase_sentence = true,
            "spellingStress" => tr.langopts.spelling_stress = true,
            "stressOpt" => read_numbers(&vb, &mut tr.langopts.stress_flags, 32),
            "stressRule" => {
                let v = scan_ints(value, 3);
                if let Some(&x) = v.first() {
                    tr.langopts.stress_rule = x;
                }
                if let Some(&x) = v.get(1) {
                    tr.langopts.unstressed_wd1 = x;
                }
                if let Some(&x) = v.get(2) {
                    tr.langopts.unstressed_wd2 = x;
                }
            }
            "words" => {
                let v = scan_ints(value, 2);
                if let Some(&x) = v.first() {
                    tr.langopts.word_gap = x;
                }
                if let Some(&x) = v.get(1) {
                    tr.langopts.vowel_pause = x;
                }
            }
            "apostrophe" | "brackets" | "bracketsAnnounced" | "l_dieresis" | "l_prefix" | "l_regressive_v"
            | "l_unpronouncable" | "l_sonorant_min" => {
                let ix = match key {
                    "apostrophe" => LOPT_APOSTROPHE,
                    "brackets" => LOPT_BRACKET_PAUSE,
                    "bracketsAnnounced" => LOPT_BRACKET_PAUSE_ANNOUNCED,
                    "l_dieresis" => LOPT_DIERESES,
                    "l_prefix" => LOPT_PREFIXES,
                    "l_regressive_v" => LOPT_REGRESSIVE_VOICING,
                    "l_unpronouncable" => LOPT_UNPRONOUNCABLE,
                    _ => LOPT_SONORANT_MIN,
                };
                if let Some(&n) = scan_ints(value, 1).first() {
                    tr.langopts.param[ix] = n;
                }
            }
            // stress lengths and amplitudes, and tunes, only affect synthesis
            _ => {}
        }
    }

    /// `LoadDictionary` for a translator: returns 0 on success, 1 if the file can't be read.
    pub(crate) fn load_dictionary(&mut self, tr: &mut Translator, name: &[u8]) -> i32 {
        self.dictionary_name = name.to_vec();
        tr.dictionary_name = name.to_vec();
        tr.dict = None;
        let name = String::from_utf8_lossy(name).into_owned();
        match self.data.dict(&name) {
            Some(d) => {
                tr.langopts.replace_chars = d.replace_chars;
                tr.dict = Some(d);
                0
            }
            None => 1,
        }
    }

    /// `PhonemeReplacement`: a `replace` line.
    fn phoneme_replacement(&mut self, p: &str) {
        let mut it = p.split_ascii_whitespace();
        let Some(flags) = it.next().and_then(parse_c_int) else { return };
        let Some(s1) = it.next() else { return };
        let s2 = it.next().unwrap_or("NULL");
        if self.replace_phonemes.len() >= N_REPLACE_PHONEMES {
            return;
        }
        let s1: Vec<u8> = s1.bytes().take(11).collect();
        let s2: Vec<u8> = s2.bytes().take(11).collect();
        let phon = self.lookup_phoneme_string(&s1);
        if phon == 0 {
            return;
        }
        let new_ph = self.lookup_phoneme_string(&s2);
        self.replace_phonemes.push(ReplacePhonemes { old_ph: phon as u8, new_ph: new_ph as u8, typ: flags as i8 });
    }

    /// `LoadVoice(vname, control)`. `rel` is the voice file's path relative to the data
    /// directory, or `None` if no file was found.
    fn load_voice(&mut self, vname: &str, rel: Option<&str>) -> bool {
        let text = rel.and_then(|r| self.data.voice_file(r));
        let mut language_type: Vec<u8> = ESPEAKNG_DEFAULT_VOICE.to_vec();
        if text.is_none() && self.select_phoneme_table_name(vname.as_bytes()) >= 0 {
            language_type = vname.as_bytes().to_vec();
        }
        self.translator = None;
        let mut translator_name = language_type.clone();
        let mut new_dictionary = language_type.clone();
        let mut phonemes_name: Vec<u8> = Vec::new();
        let mut language_set = false;
        let mut phonemes_set = false;
        self.replace_phonemes.clear();

        if let Some(text) = text {
            for raw in text.split_inclusive('\n') {
                let raw = &raw[..floor_char_boundary(raw, 160 + 30 - 1)];
                let line = strip_line(raw);
                let (key, rest) = split_key(line);
                if key.is_empty() {
                    continue;
                }
                if let Some(k) = langopts_key(key) {
                    self.load_language_options(k, rest);
                    continue;
                }
                match key {
                    "language" => {
                        let mut it = rest.split_ascii_whitespace();
                        let language_name = it.next().unwrap_or("");
                        if language_name == "variant" {
                            continue;
                        }
                        if !language_set {
                            let lt = language_name.split('-').find(|s| !s.is_empty()).unwrap_or("");
                            language_set = true;
                            translator_name = lt.as_bytes().to_vec();
                            new_dictionary = lt.as_bytes().to_vec();
                            phonemes_name = lt.as_bytes().to_vec();
                            self.select_phoneme_table_name(&phonemes_name);
                            self.translator = Some(Box::new(select_translator(&translator_name)));
                            self.voice_language_name = language_name.as_bytes().to_vec();
                        }
                    }
                    "dictionary" => {
                        if let Some(d) = rest.split_ascii_whitespace().next() {
                            new_dictionary = d.as_bytes().to_vec();
                        }
                    }
                    "phonemes" => {
                        if let Some(d) = rest.split_ascii_whitespace().next() {
                            phonemes_name = d.as_bytes().to_vec();
                        }
                    }
                    "replace" => {
                        if !phonemes_set {
                            self.select_phoneme_table_name(&phonemes_name.clone());
                            phonemes_set = true;
                        }
                        self.phoneme_replacement(rest);
                    }
                    _ => {}
                }
            }
        }

        if self.translator.is_none() {
            self.translator = Some(Box::new(select_translator(&translator_name)));
        }

        let mut ix = self.select_phoneme_table_name(&phonemes_name);
        if ix < 0 {
            ix = 0;
        }
        self.voice_phoneme_tab_ix = ix;
        let mut tr = self.translator.take().unwrap_or_else(|| Box::new(select_translator(b"")));
        tr.phoneme_tab_ix = ix;
        self.load_dictionary(&mut tr, &new_dictionary);
        self.translator = Some(tr);
        if self.dictionary_name.is_empty() {
            self.translator = None;
            return false;
        }
        true
    }

    /// `espeak_SetVoiceByName` on the current state: `name` is a voice file name (e.g.
    /// `gmw/en-US`) or voice name.
    pub(crate) fn set_voice_by_name_c(&mut self, name: &str) -> Result<()> {
        let mut buf: String = name.chars().take(59).collect();
        if let Some(p) = buf.find('+') {
            // voice variants only change synthesis parameters
            buf.truncate(p);
        }
        let lower = buf.to_ascii_lowercase();
        let rel =
            ["voices/", "lang/"].iter().map(|d| format!("{d}{lower}")).find(|r| self.data.voice_file(r).is_some());
        if let Some(rel) = rel {
            if self.load_voice(&lower, Some(&rel)) {
                self.voice_loaded = true;
                return Ok(());
            }
        }
        let identifier = self.select_voice_by_name(&lower);
        if let Some(id) = identifier {
            let rel =
                ["voices/", "lang/"].iter().map(|d| format!("{d}{id}")).find(|r| self.data.voice_file(r).is_some());
            if self.load_voice(&id, rel.as_deref()) {
                self.voice_loaded = true;
                return Ok(());
            }
        }
        Err(Error::VoiceNotFound(name.to_string()))
    }

    /// `SelectVoiceByName`: the identifier of a voice, matched by name or file name.
    fn select_voice_by_name(&self, name: &str) -> Option<String> {
        let voices = self.data.voices_list();
        let name: String = name.chars().take(39).collect();
        let last_part = format!("/{name}");
        let mut match_fname = None;
        let mut match_fname2 = None;
        for (ix, v) in voices.iter().enumerate() {
            if name.eq_ignore_ascii_case(&v.name) {
                return Some(voices[ix].identifier.clone());
            }
            let id = &v.identifier;
            if name.eq_ignore_ascii_case(id) {
                match_fname = Some(ix);
            } else if id.len() >= last_part.len() && id[id.len() - last_part.len()..].eq_ignore_ascii_case(&last_part) {
                match_fname2 = Some(ix);
            }
        }
        match_fname.or(match_fname2).map(|ix| voices[ix].identifier.clone())
    }
}

fn floor_char_boundary(s: &str, max: usize) -> usize {
    if s.len() <= max {
        return s.len();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
