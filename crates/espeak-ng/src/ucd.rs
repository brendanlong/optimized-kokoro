//! The parts of ucd-tools (bundled with espeak-ng 1.52) that the text path uses. espeak builds with
//! `include/compat/wctype.h`, which maps `iswalpha` etc. to these, so they replace libc's tables.

use crate::ucd_tables::RANGES;

const ALPHA: u8 = 1;
const ALNUM: u8 = 2;
const UPPER: u8 = 4;
const LOWER: u8 = 8;
const SPACE: u8 = 16;
const PUNCT: u8 = 32;

fn lookup(c: i32) -> (u8, i32, i32, u32) {
    if !(0..0x110000).contains(&c) {
        return (0, 0, 0, crate::defs::CLAUSE_NONE as u32);
    }
    let c = c as u32;
    let ix = RANGES.partition_point(|r| r.0 <= c) - 1;
    let r = RANGES[ix];
    (r.1, r.2, r.3, r.4)
}

pub fn isalpha(c: i32) -> bool {
    lookup(c).0 & ALPHA != 0
}

pub fn isalnum(c: i32) -> bool {
    lookup(c).0 & ALNUM != 0
}

pub fn isupper(c: i32) -> bool {
    lookup(c).0 & UPPER != 0
}

pub fn islower(c: i32) -> bool {
    lookup(c).0 & LOWER != 0
}

pub fn isspace(c: i32) -> bool {
    lookup(c).0 & SPACE != 0
}

pub fn ispunct(c: i32) -> bool {
    lookup(c).0 & PUNCT != 0
}

pub fn isdigit(c: i32) -> bool {
    (0x30..=0x39).contains(&c)
}

pub fn tolower(c: i32) -> i32 {
    let d = lookup(c).1;
    if d == 0 { c } else { c + d }
}

pub fn toupper(c: i32) -> i32 {
    let d = lookup(c).2;
    if d == 0 { c } else { c + d }
}

/// `clause_type_from_codepoint` (readclause.c), precomputed from ucd-tools' espeak properties.
pub fn clause_type_from_codepoint(c: i32) -> i32 {
    lookup(c).3 as i32
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
    }
}
