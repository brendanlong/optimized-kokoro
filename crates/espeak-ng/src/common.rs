//! common.c, plus the byte-buffer helpers that stand in for C's `char *` arithmetic.
//!
//! Text and phoneme strings live in byte buffers, as in C. A C pointer becomes a buffer and an
//! `isize` position. Reads outside the buffer give 0 and writes outside it are dropped, so code
//! that walks off the end of a buffer stops where C would find a terminator instead of panicking.

use crate::ucd;

// A negative position cast to usize is far past the end, so one bounds check covers both ends.
#[inline]
pub fn at(b: &[u8], i: isize) -> u8 {
    b.get(i as usize).copied().unwrap_or(0)
}

/// A `char` read as C's signed char on x86.
#[inline]
pub fn sat(b: &[u8], i: isize) -> i32 {
    at(b, i) as i8 as i32
}

#[inline]
pub fn put(b: &mut [u8], i: isize, v: u8) {
    if let Some(x) = b.get_mut(i as usize) {
        *x = v;
    }
}

/// `strlen(&b[i])`.
pub fn cstrlen(b: &[u8], i: isize) -> usize {
    if i < 0 {
        return 0;
    }
    let s = b.get(i as usize..).unwrap_or(&[]);
    s.iter().position(|&c| c == 0).unwrap_or(s.len())
}

/// The C string at `b[i]`, without its terminator.
pub fn cstr(b: &[u8], i: isize) -> &[u8] {
    if i < 0 {
        return &[];
    }
    let s = b.get(i as usize..).unwrap_or(&[]);
    &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
}

/// `strcpy(&dst[i], src)` where `src` holds no terminator; truncated to fit `dst`.
pub fn cstrcpy(dst: &mut [u8], i: isize, src: &[u8]) {
    if i < 0 || i as usize >= dst.len() {
        return;
    }
    let i = i as usize;
    let room = dst.len() - i - 1;
    let n = src.len().min(room);
    dst[i..i + n].copy_from_slice(&src[..n]);
    dst[i + n] = 0;
}

/// `strcat(dst, src)`.
pub fn cstrcat(dst: &mut [u8], src: &[u8]) {
    let n = cstrlen(dst, 0);
    cstrcpy(dst, n as isize, src);
}

/// `memcpy(&dst[d], &src[s], n)` with out-of-range bytes skipped.
pub fn memcpy(dst: &mut [u8], d: isize, src: &[u8], s: isize, n: usize) {
    for k in 0..n as isize {
        put(dst, d + k, at(src, s + k));
    }
}

pub fn utf8_in(b: &[u8], i: isize) -> (i32, isize) {
    utf8_in2(b, i, false)
}

pub fn utf8_in2(b: &[u8], mut i: isize, backwards: bool) -> (i32, isize) {
    const MASK: [i32; 4] = [0xff, 0x1f, 0x0f, 0x07];
    while at(b, i) & 0xc0 == 0x80 {
        if backwards {
            i -= 1
        } else {
            i += 1
        }
        if i < -1 || i > b.len() as isize {
            break;
        }
    }
    let mut n_bytes = 0usize;
    let mut c1 = sat(b, i);
    i += 1;
    if c1 & 0x80 != 0 {
        if c1 & 0xe0 == 0xc0 {
            n_bytes = 1;
        } else if c1 & 0xf0 == 0xe0 {
            n_bytes = 2;
        } else if c1 & 0xf8 == 0xf0 {
            n_bytes = 3;
        }
        c1 &= MASK[n_bytes];
        let mut ix = 0;
        while ix < n_bytes {
            if at(b, i) == 0 {
                break;
            }
            c1 = (c1 << 6) + (at(b, i) as i32 & 0x3f);
            i += 1;
            ix += 1;
        }
        n_bytes = ix;
    }
    (c1, n_bytes as isize + 1)
}

/// Writes `c` as UTF-8 at `b[i]`, returning the number of bytes.
pub fn utf8_out(c: i32, b: &mut [u8], i: isize) -> isize {
    let c = c as u32;
    if c < 0x80 {
        put(b, i, c as u8);
        return 1;
    }
    if c >= 0x110000 {
        put(b, i, b' ');
        return 1;
    }
    let n_bytes = if c < 0x800 {
        1
    } else if c < 0x10000 {
        2
    } else {
        3
    };
    const CODE: [u8; 4] = [0, 0xc0, 0xe0, 0xf0];
    let mut shift = 6 * n_bytes;
    put(b, i, CODE[n_bytes as usize] | (c >> shift) as u8);
    for j in 0..n_bytes {
        shift -= 6;
        put(b, i + j as isize + 1, 0x80 + ((c >> shift) & 0x3f) as u8);
    }
    n_bytes as isize + 1
}

/// `utf8_out` onto the end of a vector.
pub fn push_utf8(out: &mut Vec<u8>, c: i32) {
    let mut b = [0u8; 4];
    let n = utf8_out(c, &mut b, 0) as usize;
    out.extend_from_slice(&b[..n]);
}

pub fn lookupwchar(list: &[u16], c: i32) -> i32 {
    for (ix, &x) in list.iter().enumerate() {
        if x == 0 {
            break;
        }
        if x as i32 == c {
            return ix as i32 + 1;
        }
    }
    0
}

/// `wcschr(s, c) != NULL`; like C, the terminator matches `c == 0`.
pub fn wcschr(s: &[u32], c: i32) -> bool {
    for &x in s {
        if x as i32 == c {
            return true;
        }
        if x == 0 {
            return false;
        }
    }
    c == 0
}

pub fn is_alpha(c: i32) -> bool {
    const EXTRA_INDIC_ALPHAS: [u16; 3] = [0xa70, 0xa71, 0];
    if ucd::isalpha(c) {
        return true;
    }
    let c = c as u32;
    if c < 0x300 {
        return false;
    }
    if (0x901..=0xdf7).contains(&c) {
        if c & 0x7f < 0x64 {
            return true;
        }
        if lookupwchar(&EXTRA_INDIC_ALPHAS, c as i32) != 0 {
            return true;
        }
        return (0xd7a..=0xd7f).contains(&c);
    }
    (0x5b0..=0x5c2).contains(&c)
        || c == 0x605
        || c == 0x670
        || (0x64b..=0x65e).contains(&c)
        || (0x300..=0x36f).contains(&c)
        || (0xf40..=0xfbc).contains(&c)
        || (0x1100..=0x11ff).contains(&c)
        || (0x2800..=0x28ff).contains(&c)
        || (c > 0x3040 && c <= 0xa700)
}

const BRACKETS: [u16; 18] = [
    b'(' as u16,
    b')' as u16,
    b'[' as u16,
    b']' as u16,
    b'{' as u16,
    b'}' as u16,
    b'<' as u16,
    b'>' as u16,
    b'"' as u16,
    b'\'' as u16,
    b'`' as u16,
    0xab,
    0xbb,
    0x300a,
    0x300b,
    0xe000 + b'<' as u16,
    0,
    0,
];

pub fn is_bracket(c: i32) -> bool {
    if (0x2014..=0x201f).contains(&c) {
        return true;
    }
    lookupwchar(&BRACKETS, c) != 0
}

pub fn is_digit09(c: i32) -> bool {
    (b'0' as i32..=b'9' as i32).contains(&c)
}

pub fn is_digit(c: i32) -> bool {
    ucd::isdigit(c) || (0x966..=0x96f).contains(&c)
}

pub fn is_space(c: i32) -> bool {
    if c == 0 {
        return false;
    }
    if (0x2500..0x25a0).contains(&c) {
        return true;
    }
    if (0xfff9..=0xffff).contains(&c) {
        return true;
    }
    ucd::isspace(c)
}

pub fn isspace2(c: i32) -> bool {
    !((c & 0xff) == 0 || c > b' ' as i32)
}

/// C's `isspace` in the C.UTF-8 locale, for a (signed) char.
pub fn c_isspace(c: i32) -> bool {
    matches!(c, 0x09..=0x0d | 0x20)
}

pub fn c_isalpha(c: i32) -> bool {
    (b'a' as i32..=b'z' as i32).contains(&c) || (b'A' as i32..=b'Z' as i32).contains(&c)
}

pub fn c_isalnum(c: i32) -> bool {
    c_isalpha(c) || is_digit09(c)
}

pub fn is_str_totally_null(b: &[u8], i: isize, size: usize) -> bool {
    (0..size as isize).all(|k| at(b, i + k) == 0)
}

/// `strncpy0`.
pub fn strncpy0(dst: &mut [u8], src: &[u8], size: usize) {
    let size = size.min(dst.len());
    if size == 0 {
        return;
    }
    let n = cstrlen(src, 0).min(size);
    dst[..n].copy_from_slice(&src[..n]);
    for x in &mut dst[n..size] {
        *x = 0;
    }
    dst[size - 1] = 0;
}

/// `L('e', 'n')` etc.: a language name packed into an int.
pub const fn l2(a: u8, b: u8) -> i32 {
    ((a as i32) << 8) | b as i32
}

pub const fn l3(a: u8, b: u8, c: u8) -> i32 {
    ((a as i32) << 16) | ((b as i32) << 8) | c as i32
}

/// `WordToString2`.
pub fn word_to_string2(word: i32) -> Vec<u8> {
    let mut out = Vec::new();
    for ix in (0..4).rev() {
        let c = (word >> (ix * 8)) as u8;
        if c != 0 {
            out.push(c);
        }
    }
    out
}

/// glibc's `atoi` on the bytes at `b[i]`: `(int)strtol(s, NULL, 10)`, which saturates at the
/// 64-bit `long` limits and then keeps the low 32 bits.
pub fn atoi(b: &[u8], mut i: isize) -> i32 {
    while c_isspace(sat(b, i)) {
        i += 1;
    }
    let mut neg = false;
    if at(b, i) == b'-' || at(b, i) == b'+' {
        neg = at(b, i) == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while at(b, i).is_ascii_digit() {
        let d = (at(b, i) - b'0') as i64;
        v = if neg { v.saturating_mul(10).saturating_sub(d) } else { v.saturating_mul(10).saturating_add(d) };
        i += 1;
    }
    v as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_roundtrip() {
        let mut b = [0u8; 8];
        for c in ['a', 'é', 'ā', 'ꙅ', '𠜎'] {
            let n = utf8_out(c as i32, &mut b, 0);
            assert_eq!(utf8_in(&b, 0), (c as i32, n));
        }
        assert_eq!(utf8_in(b"\xc4", 0), (4, 1));
        assert_eq!(utf8_in2("aé".as_bytes(), 2, true), ('é' as i32, 2));
    }

    #[test]
    fn c_strings() {
        let mut b = [b'x'; 4];
        cstrcpy(&mut b, 1, b"abcdef");
        assert_eq!(&b, b"xab\0");
        assert_eq!(cstr(&b, 0), b"xab");
        assert_eq!(atoi(b"  42x", 0), 42);
        assert_eq!(atoi(b"-42", 0), -42);
        // Values past i32 keep their low 32 bits; past i64 they saturate first.
        assert_eq!(atoi(b"220256541512", 0), 220256541512i64 as i32);
        assert_eq!(atoi(b"20000000000", 0), 20000000000i64 as i32);
        assert_eq!(atoi(b"99999999999999999999", 0), i64::MAX as i32);
        assert_eq!(atoi(b"-99999999999999999999", 0), i64::MIN as i32);
    }
}
