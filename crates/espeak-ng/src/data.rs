//! Loading espeak-ng's compiled data: `phontab`, `phonindex`, `*_dict` and the voice files
//! (`LoadPhData` in synthdata.c, `LoadDictionary`/`InitGroups` in dictionary.c, voices.c).
//! Everything here is read once and shared; translation state lives in [`crate::Espeak`].

use crate::common::{at, c_isspace, cstrlen, is_str_totally_null};
use crate::defs::{
    N_HASH_DICT, N_LETTER_GROUPS, N_RULE_GROUP2, RULE_GROUP_END, RULE_GROUP_START, RULE_LETTERGP2,
    RULE_REPLACEMENTS,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug)]
pub enum Error {
    Io(PathBuf, std::io::Error),
    BadData(String),
    VoiceNotFound(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Error::BadData(s) => write!(f, "bad espeak-ng data: {s}"),
            Error::VoiceNotFound(s) => write!(f, "voice not found: {s}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub const N_PHONEME_TAB_NAME: usize = 32;

/// `PHONEME_TAB`, as stored in `phontab`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhonemeTab {
    pub mnemonic: u32,
    pub phflags: u32,
    pub program: u16,
    pub code: u8,
    pub typ: u8,
    pub start_type: u8,
    pub end_type: u8,
    pub std_length: u8,
    pub length_mod: u8,
}

/// `PHONEME_TAB_LIST`.
#[derive(Debug, Clone)]
pub struct PhonemeTabList {
    pub name: Vec<u8>,
    pub phonemes: Vec<PhonemeTab>,
    pub includes: u8,
}

/// A loaded `*_dict` file and the indexes `InitGroups` builds over it. Offsets are into `data`.
#[derive(Debug)]
pub struct Dict {
    pub data: Vec<u8>,
    pub rules: usize,
    pub hashtab: Vec<usize>,
    pub groups1: [Option<usize>; 256],
    pub groups3: [Option<usize>; 128],
    pub groups2: Vec<usize>,
    pub groups2_name: Vec<u32>,
    pub groups2_count: [u8; 256],
    pub groups2_start: [u8; 256],
    pub letter_groups: [Option<usize>; N_LETTER_GROUPS],
    pub replace_chars: Option<usize>,
    /// For each byte of the rules, the distance to the next 0 byte (255 if that far or more),
    /// so `MatchRule` can skip the rest of a failed rule without scanning it.
    pub rule_skip: Vec<u8>,
}

impl Dict {
    /// The offset just past the 0 that ends the rule containing `i` (C: `while (*rule++ != 0);`).
    #[inline]
    pub fn skip_rule(&self, i: usize) -> usize {
        match i.checked_sub(self.rules).and_then(|k| self.rule_skip.get(k)) {
            Some(&s) if s < 255 => i + s as usize + 1,
            _ => match self.data.get(i..).and_then(|r| r.iter().position(|&b| b == 0)) {
                Some(n) => i + n + 1,
                None => self.data.len() + 1,
            },
        }
    }

    /// `LoadDictionary` + `InitGroups`.
    pub fn parse(data: Vec<u8>) -> std::result::Result<Self, String> {
        let size = data.len();
        let rd = |i: usize| i32::from_le_bytes([at(&data, i as isize), at(&data, i as isize + 1), at(&data, i as isize + 2), at(&data, i as isize + 3)]);
        if size <= N_HASH_DICT + 8 {
            return Err("empty _dict file".into());
        }
        let length = rd(4);
        if rd(0) != N_HASH_DICT as i32 || length <= 0 || length > 0x8000000 || length as usize > size {
            return Err(format!("bad dictionary data (length={length:x})"));
        }
        let mut d = Dict {
            data,
            rules: length as usize,
            hashtab: vec![0; N_HASH_DICT],
            groups1: [None; 256],
            groups3: [None; 128],
            groups2: Vec::new(),
            groups2_name: Vec::new(),
            groups2_count: [0; 256],
            groups2_start: [255; 256],
            letter_groups: [None; N_LETTER_GROUPS],
            replace_chars: None,
            rule_skip: Vec::new(),
        };
        let rules = &d.data[d.rules..];
        let mut skip = vec![255u8; rules.len()];
        let mut next_zero: Option<usize> = None;
        for i in (0..rules.len()).rev() {
            if rules[i] == 0 {
                next_zero = Some(i);
            }
            if let Some(z) = next_zero {
                skip[i] = (z - i).min(255) as u8;
            }
        }
        d.rule_skip = skip;
        d.init_groups();
        let mut p = 8usize;
        for hash in 0..N_HASH_DICT {
            d.hashtab[hash] = p;
            loop {
                let len = at(&d.data, p as isize) as usize;
                if len == 0 {
                    break;
                }
                p += len;
            }
            p += 1;
        }
        Ok(d)
    }

    fn init_groups(&mut self) {
        let data = &self.data;
        let mut p = self.rules as isize;
        if at(data, p) == RULE_GROUP_END {
            return;
        }
        while at(data, p) != 0 {
            if at(data, p) != RULE_GROUP_START {
                break;
            }
            p += 1;
            if at(data, p) == RULE_REPLACEMENTS {
                // align to a word boundary; the buffer starts 16-byte aligned in C
                p = (p + 4) & !3;
                self.replace_chars = Some(p as usize);
                while !is_str_totally_null(data, p, 4) {
                    p += 1;
                }
                while at(data, p) != RULE_GROUP_END {
                    if p as usize >= data.len() {
                        return;
                    }
                    p += 1;
                }
                p += 1;
                continue;
            }
            if at(data, p) == RULE_LETTERGP2 {
                let ix = (at(data, p + 1) as i32 - b'A' as i32).rem_euclid(256) as usize;
                p += 2;
                if ix < N_LETTER_GROUPS {
                    self.letter_groups[ix] = Some(p as usize);
                }
            } else {
                let len = cstrlen(data, p) as isize;
                let c = at(data, p);
                let c2 = at(data, p + 1);
                p += len + 1;
                if len == 1 {
                    self.groups1[c as usize] = Some(p as usize);
                } else if len == 0 {
                    self.groups1[0] = Some(p as usize);
                } else if c == 1 {
                    if let Some(g) = self.groups3.get_mut(c2 as usize - 1) {
                        *g = Some(p as usize);
                    }
                } else if self.groups2.len() < N_RULE_GROUP2 {
                    if self.groups2_start[c as usize] == 255 {
                        self.groups2_start[c as usize] = self.groups2.len() as u8;
                    }
                    self.groups2_count[c as usize] = self.groups2_count[c as usize].wrapping_add(1);
                    self.groups2.push(p as usize);
                    self.groups2_name.push(c as u32 + ((c2 as u32) << 8));
                }
            }
            while at(data, p) != RULE_GROUP_END {
                if p as usize >= data.len() {
                    return;
                }
                p += cstrlen(data, p) as isize + 1;
            }
            p += 1;
        }
    }
}

/// One entry of `espeak_ListVoices`.
#[derive(Debug, Clone)]
pub struct VoiceInfo {
    /// The file name within `voices/` or `lang/`, e.g. `gmw/en-US`.
    pub identifier: String,
    pub name: String,
    /// `(priority, language)` pairs, in file order.
    pub languages: Vec<(u8, String)>,
    pub gender: i32,
    pub age: i32,
    pub variants: i32,
}

/// espeak-ng's data directory, loaded once and shared between threads.
pub struct EspeakData {
    dir: PathBuf,
    pub(crate) phoneme_tab_list: Vec<PhonemeTabList>,
    pub(crate) phoneme_index: Vec<u16>,
    dicts: Mutex<HashMap<String, Option<Arc<Dict>>>>,
    voice_files: Mutex<HashMap<String, Option<Arc<String>>>>,
    voices: OnceLock<Vec<VoiceInfo>>,
}

impl std::fmt::Debug for EspeakData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EspeakData").field("dir", &self.dir).finish_non_exhaustive()
    }
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::Io(path.to_path_buf(), e))
}

impl EspeakData {
    /// `LoadPhData`: reads `phontab` and `phonindex` from an `espeak-ng-data` directory.
    pub fn load(dir: impl AsRef<Path>) -> Result<Arc<Self>> {
        let dir = dir.as_ref().to_path_buf();
        let tab = read(&dir.join("phontab"))?;
        let index = read(&dir.join("phonindex"))?;
        let phoneme_index = index.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        let mut phoneme_tab_list = Vec::new();
        let n_tables = at(&tab, 0) as usize;
        let mut p = 4usize;
        for _ in 0..n_tables {
            let n_phonemes = at(&tab, p as isize) as usize;
            let includes = at(&tab, p as isize + 1);
            p += 4;
            let name_bytes = tab.get(p..p + N_PHONEME_TAB_NAME).ok_or_else(|| Error::BadData("phontab truncated".into()))?;
            let name = name_bytes[..cstrlen(name_bytes, 0)].to_vec();
            p += N_PHONEME_TAB_NAME;
            let mut phonemes = Vec::with_capacity(n_phonemes);
            for _ in 0..n_phonemes {
                let e = tab.get(p..p + 16).ok_or_else(|| Error::BadData("phontab truncated".into()))?;
                phonemes.push(PhonemeTab {
                    mnemonic: u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                    phflags: u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
                    program: u16::from_le_bytes([e[8], e[9]]),
                    code: e[10],
                    typ: e[11],
                    start_type: e[12],
                    end_type: e[13],
                    std_length: e[14],
                    length_mod: e[15],
                });
                p += 16;
            }
            phoneme_tab_list.push(PhonemeTabList { name, phonemes, includes });
        }
        Ok(Arc::new(EspeakData {
            dir,
            phoneme_tab_list,
            phoneme_index,
            dicts: Mutex::new(HashMap::new()),
            voice_files: Mutex::new(HashMap::new()),
            voices: OnceLock::new(),
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The parsed `<name>_dict`, or `None` if it can't be read or is malformed.
    pub(crate) fn dict(&self, name: &str) -> Option<Arc<Dict>> {
        let mut dicts = self.dicts.lock().unwrap_or_else(|e| e.into_inner());
        dicts
            .entry(name.to_string())
            .or_insert_with(|| {
                if name.is_empty() || name.contains('/') || name.contains('\0') {
                    return None;
                }
                let data = std::fs::read(self.dir.join(format!("{name}_dict"))).ok()?;
                Dict::parse(data).ok().map(Arc::new)
            })
            .clone()
    }

    /// The text of a voice file, by its path relative to the data directory.
    pub(crate) fn voice_file(&self, rel: &str) -> Option<Arc<String>> {
        if rel.is_empty() || rel.split('/').any(|part| part == "..") || rel.contains('\0') {
            return None;
        }
        let mut files = self.voice_files.lock().unwrap_or_else(|e| e.into_inner());
        files
            .entry(rel.to_string())
            .or_insert_with(|| {
                let path = self.dir.join(rel);
                if !path.is_file() {
                    return None;
                }
                let bytes = std::fs::read(path).ok()?;
                Some(Arc::new(String::from_utf8_lossy(&bytes).into_owned()))
            })
            .clone()
    }

    /// `espeak_ListVoices(NULL)`'s full sorted list (`voices_list`), including variants.
    pub fn voices_list(&self) -> &[VoiceInfo] {
        self.voices.get_or_init(|| {
            let mut out = Vec::new();
            for sub in ["voices", "lang"] {
                let base = self.dir.join(sub);
                collect_voices(&base, &base, &mut out);
            }
            out.sort_by(voice_name_sorter);
            out
        })
    }

    /// `espeak_ListVoices(NULL)`: every voice except variants and mbrola voices.
    pub fn list_voices(&self) -> Vec<&VoiceInfo> {
        self.voices_list()
            .iter()
            .filter(|v| !v.languages.is_empty() && v.languages[0].1 != "variant" && !v.identifier.starts_with("mb/"))
            .collect()
    }
}

fn collect_voices(base: &Path, dir: &Path, out: &mut Vec<VoiceInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_voices(base, &path, out);
        } else if let Ok(bytes) = std::fs::read(&path) {
            let rel = path.strip_prefix(base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if let Some(v) = read_voice_file(&String::from_utf8_lossy(&bytes), &rel) {
                out.push(v);
            }
        }
    }
}

/// `fgets_strip`'s line clean-up.
pub(crate) fn strip_line(line: &str) -> &str {
    if line.starts_with('#') {
        return "";
    }
    let mut s = line;
    while s.len() > 1 && s.as_bytes()[s.len() - 1].is_ascii_whitespace() {
        s = &s[..s.len() - 1];
    }
    if let Some(i) = s.find("//") {
        s = &s[..i];
    }
    s
}

/// `ReadVoiceFile`.
fn read_voice_file(text: &str, fname: &str) -> Option<VoiceInfo> {
    let mut name = String::new();
    let mut gender = String::new();
    let mut age = 0;
    let mut languages = Vec::new();
    let mut langix = 0usize;
    let mut variants = 4;
    for raw in text.split_inclusive('\n') {
        let line = strip_line(&raw[..raw.len().min(119)]);
        let (key, rest) = split_key(line);
        if key.is_empty() {
            continue;
        }
        match key {
            "name" => name = rest.trim_start().chars().take(79).collect(),
            "language" => {
                let mut it = rest.split_ascii_whitespace();
                let lang = it.next().unwrap_or("").to_string();
                let priority = it.next().and_then(parse_c_int).unwrap_or(5);
                let len = lang.len() + 2;
                if langix + len + 1 < 300 {
                    languages.push((priority as u8, lang));
                    langix += len;
                }
            }
            "gender" => {
                let mut it = rest.split_ascii_whitespace();
                gender = it.next().unwrap_or("").to_string();
                age = it.next().and_then(parse_c_int).unwrap_or(0);
            }
            "variants" => variants = rest.split_ascii_whitespace().next().and_then(parse_c_int).unwrap_or(4),
            _ => {}
        }
    }
    if languages.is_empty() {
        return None;
    }
    Some(VoiceInfo {
        identifier: fname.to_string(),
        name: if name.is_empty() { fname.to_string() } else { name },
        languages,
        gender: match gender.as_str() {
            "female" => 2,
            _ => 1,
        },
        age,
        variants,
    })
}

/// Splits a voice-file line into its attribute name and the rest, as `LoadVoice` does.
pub(crate) fn split_key(line: &str) -> (&str, &str) {
    let end = line.bytes().position(|c| c_isspace(c as i8 as i32)).unwrap_or(line.len());
    let rest = if end < line.len() { &line[end + 1..] } else { "" };
    (&line[..end], rest)
}

/// `sscanf("%d")` on one token.
pub(crate) fn parse_c_int(s: &str) -> Option<i32> {
    let s = s.trim_start();
    let bytes = s.as_bytes();
    let mut i = 0;
    if i < bytes.len() && (bytes[i] == b'-' || bytes[i] == b'+') {
        i += 1;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return None;
    }
    s[..i].parse::<i64>().ok().map(|v| v as i32)
}

fn voice_name_sorter(a: &VoiceInfo, b: &VoiceInfo) -> std::cmp::Ordering {
    let la = a.languages.first().map(|l| l.1.as_str()).unwrap_or("");
    let lb = b.languages.first().map(|l| l.1.as_str()).unwrap_or("");
    la.cmp(lb)
        .then_with(|| {
            let pa = a.languages.first().map(|l| l.0).unwrap_or(0) as i32;
            let pb = b.languages.first().map(|l| l.0).unwrap_or(0) as i32;
            pa.cmp(&pb)
        })
        .then_with(|| a.name.as_bytes().cmp(b.name.as_bytes()))
}
