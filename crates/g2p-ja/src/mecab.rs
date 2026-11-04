//! MeCab 0.996's analysis (`Tokenizer::lookup`, `Viterbi::viterbi`), as fugashi runs it: the
//! best path through the lattice of dictionary words and unknown-word candidates.
//!
//! The lattice, the order nodes are made and linked in, and the tie-breaking are MeCab's, so
//! the best path is the same. Two things are done differently, without changing the result:
//! MeCab scans a whole run of characters of one category at every position to group unknown
//! words (quadratic in the run's length), though a group of more than 24 characters is never
//! used, so the scan here stops after 25; and text after a NUL, which MeCab never sees (fugashi
//! passes a C string), is read like the rest: the caller removes NULs (see `docs/divergences.md`).

use crate::dict::{CharInfo, Dictionary, NO_READING};

/// `DEFAULT_MAX_GROUPING_SIZE`
const MAX_GROUPING: usize = 24;
/// `Tokenizer::lookup` looks at most this many bytes ahead.
const LOOKAHEAD: usize = 65535;
const NONE: u32 = u32::MAX;

/// A word on the best path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Morph<'a> {
    pub surface: &'a str,
    /// The reading cutlet uses, in hiragana: `None` when it reads the surface.
    pub reading: Option<&'a str>,
    /// MeCab's `char_type`: the category of the word's first character.
    pub char_type: u8,
    /// `stat == MECAB_UNK_NODE`
    pub unknown: bool,
}

#[derive(Clone, Copy)]
struct Node {
    /// Byte offset of the surface (after skipped whitespace).
    start: u32,
    length: u32,
    rlength: u32,
    left: u16,
    right: u16,
    wcost: i16,
    reading: u32,
    char_type: u8,
    unknown: bool,
    cost: i64,
    prev: u32,
    /// The next node in the list of nodes ending where this one ends.
    enext: u32,
}

/// `utf8_to_ucs2`: the UCS-2 code and byte length of the character at `begin`, reading no
/// further than `end`. Characters outside the BMP are 0.
fn decode(text: &[u8], begin: usize, end: usize) -> (u32, usize) {
    let len = end - begin;
    let b0 = text[begin];
    let b = |i: usize| u32::from(text[begin + i]);
    if b0 < 0x80 {
        (u32::from(b0), 1)
    } else if len >= 2 && b0 & 0xe0 == 0xc0 {
        ((u32::from(b0) & 0x1f) << 6 | (b(1) & 0x3f), 2)
    } else if len >= 3 && b0 & 0xf0 == 0xe0 {
        ((u32::from(b0) & 0x0f) << 12 | (b(1) & 0x3f) << 6 | (b(2) & 0x3f), 3)
    } else if len >= 4 && b0 & 0xf8 == 0xf0 {
        (0, 4)
    } else {
        (0, 1)
    }
}

struct Lattice<'d, 't, 'o> {
    dict: &'d Dictionary,
    /// Sees every connection cost looked up, as `(left node's right id, right node's left id)`.
    observe: Option<std::cell::RefCell<&'o mut dyn FnMut(u16, u16)>>,
    text: &'t [u8],
    nodes: Vec<Node>,
    end_heads: Vec<u32>,
    /// Nodes made by the current lookup, in the order MeCab makes them.
    made: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// MeCab's "too long sentence.": every path's cost passed `i32::MAX`.
    TooLong,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("too long sentence")
    }
}

impl std::error::Error for Error {}

impl Lattice<'_, '_, '_> {
    fn info(&self, begin: usize, end: usize) -> (CharInfo, usize) {
        let (code, len) = decode(self.text, begin, end);
        (self.dict.char_info(code), len)
    }

    /// `CharProperty::seekToOtherType`: from `begin`, skips characters while each is of a kind
    /// with the one before it (the first with `c`), counting at most `cap` of them. Returns the
    /// position reached, the last character info read and its length, and the count.
    fn seek(&self, begin: usize, end: usize, mut c: CharInfo, cap: usize) -> (usize, CharInfo, usize, usize) {
        let mut p = begin;
        let mut count = 0;
        let mut fail = CharInfo::default();
        let mut mblen = 0;
        while p != end && count <= cap {
            let (info, len) = self.info(p, end);
            fail = info;
            mblen = len;
            if !c.is_kind_of(info) {
                break;
            }
            p += len;
            count += 1;
            c = info;
        }
        (p, fail, mblen, count)
    }

    fn add_unknown(&mut self, cinfo: CharInfo, begin: usize, begin2: usize, begin3: usize) {
        let category = cinfo.default_type();
        for t in self.dict.unknown_tokens(category) {
            self.made.push(Node {
                start: begin2 as u32,
                length: (begin3 - begin2) as u32,
                rlength: (begin3 - begin) as u32,
                left: t.left,
                right: t.right,
                wcost: t.cost,
                reading: NO_READING,
                char_type: category,
                unknown: true,
                cost: 0,
                prev: NONE,
                enext: NONE,
            });
        }
    }

    /// `Tokenizer::lookup` at `begin`, into `made`.
    fn lookup(&mut self, begin: usize) {
        self.made.clear();
        let len = self.text.len();
        let end = if len - begin >= LOOKAHEAD { begin + LOOKAHEAD } else { len };
        let space = self.dict.char_info(0x20);
        let (begin2, cinfo, mblen, _) = self.seek(begin, end, space, usize::MAX);
        if begin2 == len {
            // Only whitespace is left: MeCab makes a node past the end, which nothing follows.
            return;
        }
        let dict = self.dict;
        let mut found: Vec<(usize, usize, usize)> = Vec::new();
        dict.prefixes(&self.text[begin2..end], |n, first, count| found.push((n, first, count)));
        for (n, first, count) in found {
            for t in (first..first + count).filter_map(|i| dict.token(i)) {
                self.made.push(Node {
                    start: begin2 as u32,
                    length: n as u32,
                    rlength: (begin2 - begin + n) as u32,
                    left: t.left,
                    right: t.right,
                    wcost: t.cost,
                    reading: t.reading,
                    char_type: cinfo.default_type(),
                    unknown: false,
                    cost: 0,
                    prev: NONE,
                    enext: NONE,
                });
            }
        }
        if !self.made.is_empty() && !cinfo.invoke() {
            return;
        }
        let mut begin3 = begin2 + mblen;
        let mut group_begin3 = None;
        if begin3 > end {
            self.add_unknown(cinfo, begin, begin2, begin3);
            if !self.made.is_empty() {
                return;
            }
        }
        if cinfo.group() {
            let (group_end, _, _, count) = self.seek(begin3, end, cinfo, MAX_GROUPING);
            if count <= MAX_GROUPING {
                self.add_unknown(cinfo, begin, begin2, group_end);
            }
            // Past the cap, the group ends further than any length below reaches.
            group_begin3 = (count <= MAX_GROUPING).then_some(group_end);
        }
        for _ in 1..=cinfo.length() {
            if begin3 > end {
                break;
            }
            if Some(begin3) == group_begin3 {
                continue;
            }
            self.add_unknown(cinfo, begin, begin2, begin3);
            if begin3 == end {
                break;
            }
            let (next, next_len) = self.info(begin3, end);
            if !cinfo.is_kind_of(next) {
                break;
            }
            begin3 += next_len;
        }
        if self.made.is_empty() {
            self.add_unknown(cinfo, begin, begin2, begin3);
        }
    }

    /// `connect`: each new node (in MeCab's list order, the reverse of making them) takes the
    /// cheapest node ending at `pos`, the first in list order on a tie.
    fn connect(&mut self, pos: usize) -> Result<(), Error> {
        while let Some(mut rnode) = self.made.pop() {
            let (best, cost) = self.best_left(pos, rnode.left, i64::from(rnode.wcost)).ok_or(Error::TooLong)?;
            rnode.prev = best;
            rnode.cost = cost;
            let x = pos + rnode.rlength as usize;
            if x >= self.end_heads.len() {
                continue;
            }
            rnode.enext = self.end_heads[x];
            self.end_heads[x] = self.nodes.len() as u32;
            self.nodes.push(rnode);
        }
        Ok(())
    }

    fn best_left(&self, pos: usize, left: u16, wcost: i64) -> Option<(u32, i64)> {
        let mut best_cost: i64 = 2_147_483_647;
        let mut best = None;
        let mut l = self.end_heads[pos];
        while l != NONE {
            let lnode = &self.nodes[l as usize];
            if let Some(observe) = &self.observe {
                (observe.borrow_mut())(lnode.right, left);
            }
            let cost = lnode.cost + i64::from(self.dict.connection(lnode.right, left)) + wcost;
            if cost < best_cost {
                best_cost = cost;
                best = Some(l);
            }
            l = lnode.enext;
        }
        best.map(|b| (b, best_cost))
    }
}

/// The best path for `text`, as fugashi's `parseToNodeList` gives it (without BOS and EOS).
pub fn analyze<'a>(dict: &'a Dictionary, text: &'a str) -> Result<Vec<Morph<'a>>, Error> {
    analyze_observed(dict, text, None)
}

/// [`analyze`], calling `observe` with the context ids of every connection cost it looks up
/// (for ordering the dictionary's ids by use; see `examples/context_order.rs`).
pub fn analyze_observed<'a>(
    dict: &'a Dictionary,
    text: &'a str,
    observe: Option<&mut dyn FnMut(u16, u16)>,
) -> Result<Vec<Morph<'a>>, Error> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let bos = Node {
        start: 0,
        length: 0,
        rlength: 0,
        left: 0,
        right: 0,
        wcost: 0,
        reading: NO_READING,
        char_type: 0,
        unknown: false,
        cost: 0,
        prev: NONE,
        enext: NONE,
    };
    let observe = observe.map(std::cell::RefCell::new);
    let mut lattice =
        Lattice { dict, observe, text: bytes, nodes: vec![bos], end_heads: vec![NONE; len + 1], made: Vec::new() };
    lattice.end_heads[0] = 0;
    for pos in 0..len {
        if lattice.end_heads[pos] != NONE {
            lattice.lookup(pos);
            lattice.connect(pos)?;
        }
    }
    let Some(pos) = (0..=len).rev().find(|&p| lattice.end_heads[p] != NONE) else {
        return Ok(Vec::new());
    };
    let (mut node, _) = lattice.best_left(pos, 0, 0).ok_or(Error::TooLong)?;
    let mut out = Vec::new();
    while node != 0 {
        let n = &lattice.nodes[node as usize];
        let start = n.start as usize;
        let surface = text.get(start..start + n.length as usize).unwrap_or("");
        out.push(Morph { surface, reading: dict.reading(n.reading), char_type: n.char_type, unknown: n.unknown });
        node = n.prev;
    }
    out.reverse();
    Ok(out)
}
