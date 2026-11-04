//! The dictionary MeCab reads (UniDic's compiled `sys.dic`, `unk.dic`, `char.bin` and
//! `matrix.bin`), cut down to what cutlet uses, in one file this crate loads.
//!
//! [`build`] converts a MeCab dictionary directory. It keeps MeCab's double-array trie, its token
//! order, word costs and connection costs, so lookups and costs are MeCab's own; it renumbers the
//! context ids by how often real text uses them ([`ContextOrder`], `data/context_order.txt`) and
//! starts the connection matrix on a page, so the cells text reads share few pages. Of each
//! token's feature string it keeps only the reading cutlet takes (`pron`, else `kana`), already
//! turned into hiragana. [`Dictionary::load`] memory-maps the result.

use std::fmt;
use std::fs::File;
use memmap2::Mmap;
use std::io::{self, BufWriter, Write};
use std::ops::Range;
use std::path::Path;

const MAGIC: &[u8; 8] = b"KOJADIC1";

/// The UniDic release the reference uses (`unidic` package, `python -m unidic download`).
pub const UNIDIC_VERSION: &str = "unidic-3.1.0+2021-08-31";

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The file isn't a dictionary this crate wrote, or a MeCab file is malformed.
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

fn format(m: impl Into<String>) -> Error {
    Error::Format(m.into())
}

/// MeCab's `CharInfo`, as `char.bin` packs it: `type:18, default_type:8, length:4, group:1, invoke:1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharInfo(pub u32);

impl CharInfo {
    pub fn kinds(self) -> u32 {
        self.0 & 0x3ffff
    }
    pub fn default_type(self) -> u8 {
        (self.0 >> 18) as u8
    }
    pub fn length(self) -> usize {
        ((self.0 >> 26) & 0xf) as usize
    }
    pub fn group(self) -> bool {
        self.0 >> 30 & 1 == 1
    }
    pub fn invoke(self) -> bool {
        self.0 >> 31 == 1
    }
    /// `CharInfo::isKindOf`.
    pub fn is_kind_of(self, other: CharInfo) -> bool {
        self.kinds() & other.kinds() != 0
    }
}

/// A dictionary entry: context ids, word cost and reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub left: u16,
    pub right: u16,
    pub cost: i16,
    /// Index into [`Dictionary::reading`], or [`NO_READING`] when cutlet reads the surface.
    pub reading: u32,
}

pub const NO_READING: u32 = u32::MAX;

/// One unit of the darts double array: `base` and `check`.
#[derive(Debug, Clone, Copy, Default)]
struct Unit {
    base: i32,
    check: u32,
}

/// The dictionary file, memory-mapped: the trie, tokens, readings and connection matrix are read
/// from the mapping where they're used, so only the pages real text touches become resident. The
/// small tables (characters, unknown words) are copied out at load.
pub struct Dictionary {
    pub version: String,
    /// `matrix.bin`'s first size: right context ids of the left node index its columns.
    lsize: usize,
    rsize: usize,
    chars: Vec<CharInfo>,
    /// The unknown-word tokens of each character category, by category id.
    unknown: Vec<Vec<Token>>,
    category_names: Vec<String>,
    map: Mmap,
    da: Range<usize>,
    tokens: Range<usize>,
    readings: Range<usize>,
    reading_ends: Range<usize>,
    matrix: Range<usize>,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dictionary").field("version", &self.version).field("tokens", &self.token_count()).finish()
    }
}

/// Maps `file` read-only.
///
/// This is the crate's one `unsafe` block (Brendan's exception to the no-`unsafe` rule,
/// 2026-11-03, PR #19): `Mmap::map` is unsafe because the program's view of the bytes changes,
/// and Rust's aliasing guarantees break, if anyone modifies or truncates the file while it is
/// mapped.
#[allow(unsafe_code)]
fn map_read_only(file: &File) -> io::Result<Mmap> {
    // SAFETY: the file is read-only data written once by `build` (via `tools/fetch_unidic.sh`,
    // which checks its SHA-256) and is never written again: the fetch script and deploy
    // (`deploy/lib.sh`) install a new dictionary by writing a new file and renaming it over
    // the old path, which leaves the inode we mapped unchanged. Nothing in this program writes
    // to it. The mapping is `Mmap` (read-only), so we can't write through it either. `load`
    // checks the header and that the file's size is exactly what its section sizes add up to,
    // and every read through the mapping is bounds-checked, so a short or corrupt file gives
    // wrong answers or an error, never an out-of-range access.
    unsafe { Mmap::map(file) }
}

fn u16_le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn token_at(b: &[u8], i: usize) -> Option<Token> {
    let t = b.get(i..i + 10)?;
    Some(Token {
        left: u16::from_le_bytes([t[0], t[1]]),
        right: u16::from_le_bytes([t[2], t[3]]),
        cost: i16::from_le_bytes([t[4], t[5]]),
        reading: u32::from_le_bytes([t[6], t[7], t[8], t[9]]),
    })
}

/// Walks the file's sections, checking each fits.
struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn u32(&mut self) -> Result<u32, Error> {
        let v = u32_le(self.b, self.pos).ok_or_else(|| format("dictionary file is truncated"))?;
        self.pos += 4;
        Ok(v)
    }

    fn section(&mut self, len: usize) -> Result<Range<usize>, Error> {
        let end = self.pos.checked_add(len).filter(|&e| e <= self.b.len()).ok_or_else(|| format("dictionary file is truncated"))?;
        let r = self.pos..end;
        self.pos = end;
        Ok(r)
    }

    fn string(&mut self) -> Result<String, Error> {
        let n = self.u32()? as usize;
        if n > 1 << 16 {
            return Err(format("string too long"));
        }
        let r = self.section(n)?;
        String::from_utf8(self.b[r].to_vec()).map_err(|_| format("string isn't UTF-8"))
    }

    fn tokens(&mut self) -> Result<Vec<Token>, Error> {
        let n = self.u32()? as usize;
        let r = self.section(n.checked_mul(10).ok_or_else(|| format("token count"))?)?;
        Ok((0..n).filter_map(|i| token_at(self.b, r.start + 10 * i)).collect())
    }
}

impl Dictionary {
    /// `CharProperty::getCharInfo` for a UCS-2 code (MeCab maps characters outside the BMP to 0).
    /// `char.bin` holds 0xffff entries; U+FFFF reads the zeroed page after them.
    pub fn char_info(&self, code: u32) -> CharInfo {
        self.chars.get(code as usize).copied().unwrap_or_default()
    }

    pub fn unknown_tokens(&self, category: u8) -> &[Token] {
        self.unknown.get(category as usize).map_or(&[], Vec::as_slice)
    }

    pub fn category_name(&self, category: u8) -> &str {
        &self.category_names[category as usize]
    }

    /// `Connector::cost` without the right node's word cost (0 for ids out of range, which only
    /// a corrupt file has).
    pub fn connection(&self, left_right_id: u16, right_left_id: u16) -> i32 {
        let (r, l) = (left_right_id as usize, right_left_id as usize);
        if r >= self.lsize || l >= self.rsize {
            return 0;
        }
        let i = self.matrix.start + 2 * (r + self.lsize * l);
        u16_le(&self.map, i).map_or(0, |v| i32::from(v as i16))
    }

    /// Where the connection cost for these ids is in the file (for measuring which pages are read).
    pub fn connection_offset(&self, left_right_id: u16, right_left_id: u16) -> usize {
        self.matrix.start + 2 * (left_right_id as usize + self.lsize * right_left_id as usize)
    }

    /// A token's reading (`None` for [`NO_READING`], or out of range in a corrupt file).
    pub fn reading(&self, index: u32) -> Option<&str> {
        if index == NO_READING {
            return None;
        }
        let i = index as usize;
        let end_at = |i: usize| u32_le(&self.map, self.reading_ends.start + 4 * i).map(|e| e as usize);
        if i >= self.reading_ends.len() / 4 {
            return None;
        }
        let start = if i == 0 { 0 } else { end_at(i - 1)? };
        let end = end_at(i)?;
        if start > end || end > self.readings.len() {
            return None;
        }
        std::str::from_utf8(&self.map[self.readings.start + start..self.readings.start + end]).ok()
    }

    fn unit(&self, p: i64) -> Option<Unit> {
        let p = usize::try_from(p).ok().filter(|&p| p < self.da.len() / 8)?;
        let i = self.da.start + 8 * p;
        Some(Unit { base: u32_le(&self.map, i)? as i32, check: u32_le(&self.map, i + 4)? })
    }

    /// The `i`th dictionary token.
    pub fn token(&self, i: usize) -> Option<Token> {
        if i >= self.token_count() {
            return None;
        }
        token_at(&self.map, self.tokens.start + 10 * i)
    }

    /// darts' `commonPrefixSearch`: every key that is a prefix of `key`, shortest first, as
    /// `(byte length, first token, token count)`.
    pub fn prefixes(&self, key: &[u8], mut found: impl FnMut(usize, usize, usize)) {
        let Some(root) = self.unit(0) else { return };
        let mut b = i64::from(root.base);
        let mut emit = |b: i64, len: usize| {
            if let Some(u) = self.unit(b)
                && u.check == b as u32
                && u.base < 0
            {
                let value = (-i64::from(u.base) - 1) as u32;
                found(len, (value >> 8) as usize, (value & 0xff) as usize);
            }
        };
        for (i, &c) in key.iter().enumerate() {
            emit(b, i);
            match self.unit(b + i64::from(c) + 1) {
                Some(u) if u.check == b as u32 => b = i64::from(u.base),
                _ => return,
            }
        }
        emit(b, key.len());
    }

    pub fn token_count(&self) -> usize {
        self.tokens.len() / 10
    }

    /// Bytes of the file, by part (mapped, so resident only as far as they're read).
    pub fn sizes(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("connection matrix", self.matrix.len()),
            ("trie", self.da.len()),
            ("tokens", self.tokens.len()),
            ("readings", self.readings.len() + self.reading_ends.len()),
            ("character table", self.chars.len() * 4),
        ]
    }

    /// Maps a dictionary [`build`] wrote, checking its header and that its sections add up to
    /// the file's size.
    pub fn load(path: impl AsRef<Path>) -> Result<Dictionary, Error> {
        let path = path.as_ref();
        let named = |e: io::Error| io::Error::new(e.kind(), format!("{}: {e}", path.display()));
        let file = File::open(path).map_err(named)?;
        let map = map_read_only(&file).map_err(named)?;
        // Lookups jump around the matrix and trie: reading ahead would only fill memory with pages
        // nothing reads. Advice only, so a kernel that ignores it is fine.
        let _ = map.advise(memmap2::Advice::Random);
        let mut c = Cursor { b: &map, pos: 0 };
        if c.section(8).map(|r| &map[r] != MAGIC).unwrap_or(true) {
            return Err(format(format!("{}: not a dictionary from kokoro-g2p-ja's build", path.display())));
        }
        let version = c.string()?;
        let lsize = c.u32()? as usize;
        let rsize = c.u32()? as usize;
        let n = c.u32()? as usize;
        if n != 0x10000 || lsize == 0 || rsize == 0 {
            return Err(format("bad dictionary header"));
        }
        let r = c.section(4 * n)?;
        let chars: Vec<CharInfo> = (0..n).map(|i| CharInfo(u32_le(&map, r.start + 4 * i).unwrap_or(0))).collect();
        let categories = c.u32()? as usize;
        if categories > 256 {
            return Err(format("too many character categories"));
        }
        let mut unknown = Vec::with_capacity(categories);
        let mut category_names = Vec::with_capacity(categories);
        for _ in 0..categories {
            category_names.push(c.string()?);
            unknown.push(c.tokens()?);
        }
        let n = c.u32()? as usize;
        let da = c.section(n.checked_mul(8).ok_or_else(|| format("trie size"))?)?;
        let n = c.u32()? as usize;
        let tokens = c.section(n.checked_mul(10).ok_or_else(|| format("token count"))?)?;
        let n = c.u32()? as usize;
        let readings = c.section(n)?;
        let n = c.u32()? as usize;
        let reading_ends = c.section(n.checked_mul(4).ok_or_else(|| format("reading count"))?)?;
        c.section(c.pos.next_multiple_of(MATRIX_ALIGN) - c.pos)?;
        let matrix = c.section(lsize.checked_mul(rsize).and_then(|n| n.checked_mul(2)).ok_or_else(|| format("matrix size"))?)?;
        if c.pos != map.len() {
            return Err(format(format!("{}: {} bytes, but its sections add up to {}", path.display(), map.len(), c.pos)));
        }
        if chars.iter().any(|c| c.default_type() as usize >= unknown.len()) {
            return Err(format("character table refers to an unknown category"));
        }
        Ok(Dictionary { version, lsize, rsize, chars, unknown, category_names, map, da, tokens, readings, reading_ends, matrix })
    }
}

fn write_tokens(w: &mut impl Write, tokens: &[Token]) -> io::Result<()> {
    w.write_all(&(tokens.len() as u32).to_le_bytes())?;
    for t in tokens {
        w.write_all(&t.left.to_le_bytes())?;
        w.write_all(&t.right.to_le_bytes())?;
        w.write_all(&t.cost.to_le_bytes())?;
        w.write_all(&t.reading.to_le_bytes())?;
    }
    Ok(())
}

fn write_string(w: &mut impl Write, s: &str) -> io::Result<()> {
    w.write_all(&(s.len() as u32).to_le_bytes())?;
    w.write_all(s.as_bytes())
}

/// A compiled MeCab dictionary file (`sys.dic`, `unk.dic`).
struct MecabDic {
    da: Vec<Unit>,
    /// `(lcAttr, rcAttr, wcost, feature offset)`
    tokens: Vec<(u16, u16, i16, u32)>,
    features: Vec<u8>,
    lsize: u32,
    rsize: u32,
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

impl MecabDic {
    /// `Dictionary::open`.
    fn read(path: &Path) -> Result<MecabDic, Error> {
        let b = std::fs::read(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        if b.len() < 100 || (u32_at(&b, 0) ^ 0xef71_8f77) as usize != b.len() || u32_at(&b, 4) != 102 {
            return Err(format(format!("{}: not a MeCab 0.996 dictionary", path.display())));
        }
        let (lsize, rsize, dsize, tsize, fsize) =
            (u32_at(&b, 16), u32_at(&b, 20), u32_at(&b, 24) as usize, u32_at(&b, 28) as usize, u32_at(&b, 32) as usize);
        let charset = &b[40..72];
        if !charset.starts_with(b"utf8\0") && !charset.starts_with(b"UTF-8\0") {
            return Err(format(format!("{}: charset isn't UTF-8", path.display())));
        }
        let da_start = 72;
        let t_start = da_start + dsize;
        let f_start = t_start + tsize;
        if f_start + fsize != b.len() || dsize % 8 != 0 || tsize % 16 != 0 {
            return Err(format(format!("{}: sizes don't add up", path.display())));
        }
        let da = (da_start..t_start).step_by(8).map(|i| Unit { base: u32_at(&b, i) as i32, check: u32_at(&b, i + 4) }).collect();
        let tokens = (t_start..f_start)
            .step_by(16)
            .map(|i| (u16_at(&b, i), u16_at(&b, i + 2), u16_at(&b, i + 6) as i16, u32_at(&b, i + 8)))
            .collect();
        Ok(MecabDic { da, tokens, features: b[f_start..].to_vec(), lsize, rsize })
    }

    fn feature(&self, offset: u32) -> Result<&str, Error> {
        let start = offset as usize;
        let len = self.features.get(start..).and_then(|f| f.iter().position(|&c| c == 0)).ok_or_else(|| format("feature offset"))?;
        std::str::from_utf8(&self.features[start..start + len]).map_err(|_| format("feature isn't UTF-8"))
    }

    /// darts' `exactMatchSearch`: the `(first token, count)` stored for `key`.
    fn exact(&self, key: &[u8]) -> Option<(usize, usize)> {
        let mut b = self.da[0].base as i64;
        for &c in key {
            let p = usize::try_from(b + i64::from(c) + 1).ok()?;
            let u = self.da.get(p)?;
            if u.check != b as u32 {
                return None;
            }
            b = i64::from(u.base);
        }
        let u = self.da.get(usize::try_from(b).ok()?)?;
        (u.check == b as u32 && u.base < 0).then(|| {
            let v = (-i64::from(u.base) - 1) as u32;
            ((v >> 8) as usize, (v & 0xff) as usize)
        })
    }
}

/// fugashi's feature parsing (`csv.reader` when the string has a quote, else `split(',')`).
pub fn feature_fields(raw: &str) -> Vec<String> {
    if !raw.contains('"') {
        return raw.split(',').map(str::to_owned).collect();
    }
    // Python's csv module, default dialect, on one line.
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = raw.chars().peekable();
    let mut start = true;
    let mut quoted = false;
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == ',' {
            fields.push(std::mem::take(&mut field));
            start = true;
            continue;
        } else if c == '"' && start {
            quoted = true;
        } else {
            field.push(c);
        }
        start = false;
    }
    fields.push(field);
    fields
}

/// cutlet's reading of a dictionary word: `pron or kana or surface` (`None` for the surface),
/// through `jaconv.kata2hira`. `Err` if fugashi would fail on the feature string.
pub fn token_reading(raw: &str) -> Result<Option<String>, String> {
    let fields = feature_fields(raw);
    if fields.len() > 29 {
        return Err(format!("{} fields: {raw}", fields.len()));
    }
    let pick = |i: usize| fields.get(i).filter(|f| !f.is_empty());
    Ok(pick(9).or_else(|| pick(20)).map(|r| crate::cutlet::kata2hira(r)))
}

/// Converts the MeCab dictionary in `dicdir` (UniDic's `sys.dic`, `unk.dic`, `char.bin`,
/// `matrix.bin`) into the file [`Dictionary::load`] reads.
/// The order to number context ids in: ids listed first get the lowest numbers, the rest
/// follow in their old order. Id 0 stays 0 (MeCab's BOS and EOS nodes use it).
#[derive(Debug, Clone, Default)]
pub struct ContextOrder {
    /// Right context ids (the left node's, which index the matrix's columns), most used first.
    pub right: Vec<u16>,
    /// Left context ids (the right node's, which index its rows).
    pub left: Vec<u16>,
}

impl ContextOrder {
    /// `data/context_order.txt`: a `right` line and a `left` line, each the word and then ids.
    pub fn parse(text: &str) -> Result<ContextOrder, Error> {
        let mut order = ContextOrder::default();
        for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let mut words = line.split_whitespace();
            let list = match words.next() {
                Some("right") => &mut order.right,
                Some("left") => &mut order.left,
                _ => return Err(format("context order: lines start with `right` or `left`")),
            };
            for w in words {
                list.push(w.parse().map_err(|_| format(format!("context order: bad id {w}")))?);
            }
        }
        Ok(order)
    }

    /// `new[old]`: the permutation of `0..n` putting `listed` first (after 0).
    fn permutation(listed: &[u16], n: usize) -> Result<Vec<u16>, Error> {
        let mut new = vec![u16::MAX; n];
        let mut next = 0u16;
        for &id in std::iter::once(&0).chain(listed).chain(&(0..n as u16).collect::<Vec<_>>()) {
            let slot = new.get_mut(id as usize).ok_or_else(|| format(format!("context order: id {id} out of range")))?;
            if *slot == u16::MAX {
                *slot = next;
                next += 1;
            }
        }
        Ok(new)
    }
}

/// The file's offset of the matrix: the next multiple of the page size, so its pages are the
/// file's.
const MATRIX_ALIGN: usize = 4096;

pub fn build(dicdir: &Path, version: &str, order: &ContextOrder, out: &Path) -> Result<(), Error> {
    let sys = MecabDic::read(&dicdir.join("sys.dic"))?;
    let unk = MecabDic::read(&dicdir.join("unk.dic"))?;
    if (unk.lsize, unk.rsize) != (sys.lsize, sys.rsize) {
        return Err(format("unk.dic and sys.dic don't match"));
    }
    let chars = std::fs::read(dicdir.join("char.bin"))?;
    let csize = u32_at(&chars, 0) as usize;
    if chars.len() != 4 + 32 * csize + 4 * 0xffff {
        return Err(format("char.bin: bad size"));
    }
    let names: Vec<String> = (0..csize)
        .map(|i| {
            let raw = &chars[4 + 32 * i..4 + 32 * (i + 1)];
            String::from_utf8_lossy(&raw[..raw.iter().position(|&c| c == 0).unwrap_or(32)]).into_owned()
        })
        .collect();
    let map_start = 4 + 32 * csize;
    let mut char_map: Vec<u32> = (0..0xffff).map(|i| u32_at(&chars, map_start + 4 * i)).collect();
    char_map.push(0);
    let matrix = std::fs::read(dicdir.join("matrix.bin"))?;
    let right_ids = ContextOrder::permutation(&order.right, sys.lsize as usize)?;
    let left_ids = ContextOrder::permutation(&order.left, sys.rsize as usize)?;
    let (ml, mr) = (u16_at(&matrix, 0) as u32, u16_at(&matrix, 2) as u32);
    if (ml, mr) != (sys.lsize, sys.rsize) || matrix.len() != 4 + 2 * (ml as usize) * (mr as usize) {
        return Err(format("matrix.bin doesn't match sys.dic"));
    }

    let mut readings = String::new();
    let mut ends: Vec<u32> = Vec::new();
    let mut index: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut intern = |r: Option<String>| -> u32 {
        let Some(r) = r else { return NO_READING };
        *index.entry(r).or_insert_with_key(|r| {
            readings.push_str(r);
            ends.push(readings.len() as u32);
            ends.len() as u32 - 1
        })
    };
    let mut tokens = Vec::with_capacity(sys.tokens.len());
    for &(l, r, c, f) in &sys.tokens {
        let raw = sys.feature(f)?;
        let reading = token_reading(raw).map_err(|e| format(format!("fugashi can't read a feature: {e}")))?;
        tokens.push(Token { left: left_ids[l as usize], right: right_ids[r as usize], cost: c, reading: intern(reading) });
    }
    let mut unknown = Vec::new();
    for name in &names {
        let (start, n) = unk.exact(name.as_bytes()).ok_or_else(|| format(format!("unk.dic has no category {name}")))?;
        let list = unk.tokens[start..start + n]
            .iter()
            .map(|&(l, r, c, f)| {
                // UniDic's unknown words have six fields, so fugashi's pron and kana are None.
                let reading = token_reading(unk.feature(f)?).map_err(format)?;
                Ok(Token { left: left_ids[l as usize], right: right_ids[r as usize], cost: c, reading: intern(reading) })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        unknown.push(list);
    }

    let (lsize, rsize) = (sys.lsize as usize, sys.rsize as usize);
    let mut reordered = vec![0u8; 2 * lsize * rsize];
    for (l, &new_l) in left_ids.iter().enumerate() {
        for (r, &new_r) in right_ids.iter().enumerate() {
            let from = 4 + 2 * (r + lsize * l);
            let to = 2 * (new_r as usize + lsize * new_l as usize);
            reordered[to..to + 2].copy_from_slice(&matrix[from..from + 2]);
        }
    }
    let mut w = BufWriter::with_capacity(1 << 20, File::create(out)?);
    let mut written = 0usize;
    w.write_all(MAGIC)?;
    write_string(&mut w, version)?;
    w.write_all(&sys.lsize.to_le_bytes())?;
    w.write_all(&sys.rsize.to_le_bytes())?;
    w.write_all(&(char_map.len() as u32).to_le_bytes())?;
    for c in &char_map {
        w.write_all(&c.to_le_bytes())?;
    }
    w.write_all(&(names.len() as u32).to_le_bytes())?;
    for (name, list) in names.iter().zip(&unknown) {
        write_string(&mut w, name)?;
        w.write_all(&(list.len() as u32).to_le_bytes())?;
        for t in list {
            w.write_all(&t.left.to_le_bytes())?;
            w.write_all(&t.right.to_le_bytes())?;
            w.write_all(&t.cost.to_le_bytes())?;
            w.write_all(&t.reading.to_le_bytes())?;
        }
    }
    w.write_all(&(sys.da.len() as u32).to_le_bytes())?;
    for u in &sys.da {
        w.write_all(&u.base.to_le_bytes())?;
        w.write_all(&u.check.to_le_bytes())?;
    }
    write_tokens(&mut w, &tokens)?;
    w.write_all(&(readings.len() as u32).to_le_bytes())?;
    w.write_all(readings.as_bytes())?;
    w.write_all(&(ends.len() as u32).to_le_bytes())?;
    for e in &ends {
        w.write_all(&e.to_le_bytes())?;
    }
    w.flush()?;
    written += w.get_ref().metadata()?.len() as usize;
    w.write_all(&vec![0; written.next_multiple_of(MATRIX_ALIGN) - written])?;
    w.write_all(&reordered)?;
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_split_like_fugashi() {
        assert_eq!(feature_fields("a,b,*"), ["a", "b", "*"]);
        assert_eq!(feature_fields(r#"a,"0,1",*,"x""y",z"q""#), ["a", "0,1", "*", "x\"y", "z\"q\""]);
        assert_eq!(feature_fields(r#""ab"c,d"#), ["abc", "d"]);
    }
}
