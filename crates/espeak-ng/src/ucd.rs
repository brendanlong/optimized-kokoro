//! The parts of ucd-tools (bundled with espeak-ng 1.52) that the text path uses. espeak builds with
//! `include/compat/wctype.h`, which maps `iswalpha` etc. to these, so they replace libc's tables.

use crate::ucd_tables::RANGES;
use std::sync::OnceLock;

const ALPHA: u8 = 1;
const ALNUM: u8 = 2;
const UPPER: u8 = 4;
const LOWER: u8 = 8;
const SPACE: u8 = 16;
const PUNCT: u8 = 32;

#[derive(Clone, Copy)]
struct Props {
    flags: u8,
    lower: i32,
    upper: i32,
    clause: u32,
}

/// Direct lookup for the first code points, which almost all text uses.
const DIRECT: usize = 0x3000;

fn search(c: u32) -> Props {
    let r = RANGES[RANGES.partition_point(|r| r.0 <= c) - 1];
    Props { flags: r.1, lower: r.2, upper: r.3, clause: r.4 }
}

fn lookup(c: i32) -> Props {
    static TABLE: OnceLock<Vec<Props>> = OnceLock::new();
    if (0..DIRECT as i32).contains(&c) {
        let t = TABLE.get_or_init(|| (0..DIRECT as u32).map(search).collect());
        return t[c as usize];
    }
    if !(0..0x110000).contains(&c) {
        return Props { flags: 0, lower: 0, upper: 0, clause: crate::defs::CLAUSE_NONE as u32 };
    }
    search(c as u32)
}

pub fn isalpha(c: i32) -> bool {
    lookup(c).flags & ALPHA != 0
}

pub fn isalnum(c: i32) -> bool {
    lookup(c).flags & ALNUM != 0
}

pub fn isupper(c: i32) -> bool {
    lookup(c).flags & UPPER != 0
}

pub fn islower(c: i32) -> bool {
    lookup(c).flags & LOWER != 0
}

pub fn isspace(c: i32) -> bool {
    lookup(c).flags & SPACE != 0
}

pub fn ispunct(c: i32) -> bool {
    lookup(c).flags & PUNCT != 0
}

pub fn isdigit(c: i32) -> bool {
    (0x30..=0x39).contains(&c)
}

pub fn tolower(c: i32) -> i32 {
    c + lookup(c).lower
}

pub fn toupper(c: i32) -> i32 {
    c + lookup(c).upper
}

/// `clause_type_from_codepoint` (readclause.c), precomputed from ucd-tools' espeak properties.
pub fn clause_type_from_codepoint(c: i32) -> i32 {
    lookup(c).clause as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert!(isalpha('a' as i32) && isupper('A' as i32) && islower('é' as i32));
        assert!(isspace(' ' as i32) && !isspace(0xa0) && isspace('\n' as i32));
        assert_eq!(tolower('Ä' as i32), 'ä' as i32);
        assert_eq!(toupper('ß' as i32), 'ß' as i32);
        assert!(ispunct('!' as i32) && !ispunct('a' as i32));
        assert_eq!(clause_type_from_codepoint('.' as i32), 0x80028);
        assert!(!isalpha(-1) && !isalpha(0x110000));
        assert!(isalpha(0x10450) && isupper('Ա' as i32));
    }

    #[test]
    fn direct_table_matches_search() {
        for c in 0..DIRECT as u32 + 16 {
            let (a, b) = (lookup(c as i32), search(c));
            assert_eq!((a.flags, a.lower, a.upper, a.clause), (b.flags, b.lower, b.upper, b.clause), "{c:x}");
        }
    }
}
