//! Python `re` patterns, matched with Python's semantics.
//!
//! Patterns are written as in the Python source. [`compile`] translates them: `\w`, `\d`, `\s` and
//! `\b` become explicit classes generated from the reference interpreter (`tables.rs`), since the
//! regex crate's Unicode definitions differ from Python's, and IGNORECASE also equates i with ı
//! and İ as Python does.
//!
//! Many of the reference's patterns use lookbehind, lookahead or `\b`, and some rely on
//! backtracking into a quantifier when a trailing lookahead fails, so they run on `fancy_regex`,
//! a backtracking engine. Its VM would try every position, though, so each pattern also gets a
//! relaxed form with the lookarounds removed for the `regex` crate. Every match of the pattern
//! starts where the relaxed form matches, so the backtracker only runs, anchored with `\G`, at
//! those starts.

use crate::tables::{DIGIT, SPACE, WORD};
use std::fmt::Write;
use std::ops::{Index, Range};

type Ranges = Vec<(u32, u32)>;

fn table_ranges(table: &[(char, char)]) -> Ranges {
    table.iter().map(|&(a, b)| (a as u32, b as u32)).collect()
}

fn normalize(mut r: Ranges) -> Ranges {
    r.sort_unstable();
    let mut out: Ranges = Vec::with_capacity(r.len());
    for (a, b) in r {
        match out.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

fn complement(r: &Ranges) -> Ranges {
    let mut out = Vec::new();
    let mut next = 0u32;
    for &(a, b) in r {
        if a > next {
            out.push((next, a - 1));
        }
        next = b + 1;
    }
    if next <= 0x10FFFF {
        out.push((next, 0x10FFFF));
    }
    out
}

/// A flat class; `(?-i:...)` keeps case folding from growing classes built from `\w`/`\s`/`\d`.
fn emit_class(ranges: &Ranges, case_fixed: bool) -> String {
    let mut out = String::from(if case_fixed { "(?-i:[" } else { "[" });
    for &(a, b) in ranges {
        let _ = write!(out, "\\x{{{a:x}}}");
        if b > a {
            let _ = write!(out, "-\\x{{{b:x}}}");
        }
    }
    out.push_str(if case_fixed { "])" } else { "]" });
    out
}

const DOTLESS_I: u32 = 0x131;
const DOTTED_I: u32 = 0x130;

struct Translator<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    ignore_case: bool,
    full: String,
    relaxed: String,
    /// Open parentheses inside the lookaround being copied, which the relaxed form leaves out.
    lookaround_depth: usize,
    fancy: bool,
}

impl Translator<'_> {
    fn emit(&mut self, s: &str) {
        self.full.push_str(s);
        if self.lookaround_depth == 0 {
            self.relaxed.push_str(s);
        }
    }

    fn shorthand(c: char) -> Option<Ranges> {
        Some(match c {
            'w' => table_ranges(WORD),
            'W' => complement(&table_ranges(WORD)),
            'd' => table_ranges(DIGIT),
            'D' => complement(&table_ranges(DIGIT)),
            's' => table_ranges(SPACE),
            'S' => complement(&table_ranges(SPACE)),
            _ => return None,
        })
    }

    fn escaped_literal(&mut self, c: char) -> char {
        let mut hex = |n: usize| {
            let hex: String = (0..n).map(|_| self.chars.next().expect("short \\u escape")).collect();
            char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap()
        };
        match c {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => hex(4),
            'U' => hex(8),
            c if c.is_ascii_alphanumeric() => panic!("unsupported escape \\{c}"),
            c => c,
        }
    }

    fn class(&mut self) {
        let negated = self.chars.next_if_eq(&'^').is_some();
        let mut literals = Ranges::new();
        let mut shorthands = Ranges::new();
        let mut first = true;
        loop {
            let c = self.chars.next().expect("unterminated class");
            if c == ']' && !first {
                break;
            }
            first = false;
            let lo = if c == '\\' {
                let e = self.chars.next().unwrap();
                if let Some(r) = Self::shorthand(e) {
                    shorthands.extend(r);
                    continue;
                }
                self.escaped_literal(e)
            } else {
                c
            };
            let mut ahead = self.chars.clone();
            let hi = if ahead.next() == Some('-') && !matches!(ahead.peek(), Some(']') | None) {
                self.chars.next();
                match self.chars.next().unwrap() {
                    '\\' => {
                        let e = self.chars.next().unwrap();
                        self.escaped_literal(e)
                    }
                    h => h,
                }
            } else {
                lo
            };
            literals.push((lo as u32, hi as u32));
        }
        let covers = |c: char| literals.iter().any(|&(a, b)| (a..=b).contains(&(c as u32)));
        let has_i = covers('i') || covers('I');
        let has_letter = literals.iter().any(|&(a, b)| (a..=b).any(|c| char::from_u32(c).is_some_and(char::is_alphabetic)));
        let shorthand = !shorthands.is_empty();
        if self.ignore_case && has_letter {
            // Python's \w, \s and \d are fixed sets that IGNORECASE doesn't touch, so they are
            // emitted with case folding off, which can't also fold letters in the same class.
            assert!(!negated && !shorthand, "unsupported case-insensitive class");
        }
        if self.ignore_case && has_i {
            literals.push((DOTTED_I, DOTLESS_I));
        }
        literals.extend(shorthands);
        let mut ranges = normalize(literals);
        if negated {
            ranges = complement(&ranges);
        }
        self.emit(&emit_class(&ranges, shorthand));
    }

    fn literal(&mut self, c: char) {
        if self.ignore_case && (c == 'i' || c == 'I') {
            self.emit("[iI\\x{130}\\x{131}]");
        } else {
            self.emit(&regex::escape(c.encode_utf8(&mut [0; 4])));
        }
    }

    fn open_paren(&mut self) {
        let ahead: String = self.chars.clone().take(3).collect();
        let lookaround = ahead.starts_with("?=")
            || ahead.starts_with("?!")
            || ahead.starts_with("?<=")
            || ahead.starts_with("?<!");
        if self.lookaround_depth > 0 {
            self.lookaround_depth += 1;
        } else if lookaround {
            self.fancy = true;
            self.lookaround_depth = 1;
        }
        self.emit("(");
    }

    fn close_paren(&mut self) {
        self.emit(")");
        self.lookaround_depth = self.lookaround_depth.saturating_sub(1);
    }

    fn run(mut self) -> Self {
        if self.ignore_case {
            self.emit("(?i)");
        }
        while let Some(c) = self.chars.next() {
            match c {
                '[' => self.class(),
                '(' => self.open_paren(),
                ')' => self.close_paren(),
                '\\' => {
                    let e = self.chars.next().expect("trailing backslash");
                    if let Some(r) = Self::shorthand(e) {
                        self.emit(&emit_class(&normalize(r), true));
                    } else if e == 'b' || e == 'B' {
                        let w = emit_class(&table_ranges(WORD), true);
                        self.full.push_str(&if e == 'B' {
                            format!("(?:(?<={w})(?={w})|(?<!{w})(?!{w}))")
                        } else {
                            format!("(?:(?<={w})(?!{w})|(?<!{w})(?={w}))")
                        });
                        self.fancy = true;
                    } else if e.is_ascii_digit() {
                        panic!("backreferences are not supported");
                    } else {
                        let lit = self.escaped_literal(e);
                        self.literal(lit);
                    }
                }
                'i' | 'I' => self.literal(c),
                c => self.emit(c.encode_utf8(&mut [0; 4])),
            }
        }
        assert_eq!(self.lookaround_depth, 0, "unbalanced parentheses");
        self
    }
}

/// A compiled Python pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    relaxed: regex::Regex,
    /// The full pattern after `\G`, when it needs lookaround.
    anchored: Option<fancy_regex::Regex>,
}

/// Translates and compiles a Python pattern given without inline flags.
pub fn compile(pattern: &str) -> Pattern {
    build(pattern, false)
}

/// [`compile`] with `re.IGNORECASE`.
pub fn compile_i(pattern: &str) -> Pattern {
    build(pattern, true)
}

fn build(pattern: &str, ignore_case: bool) -> Pattern {
    let t = Translator {
        chars: pattern.chars().peekable(),
        ignore_case,
        full: String::new(),
        relaxed: String::new(),
        lookaround_depth: 0,
        fancy: false,
    }
    .run();
    let relaxed = regex::Regex::new(&t.relaxed).unwrap_or_else(|e| panic!("{pattern}: {e}"));
    let anchored = t.fancy.then(|| {
        let flags = if ignore_case { "(?i)" } else { "" };
        fancy_regex::RegexBuilder::new(&format!(r"{flags}\G(?:{})", t.full))
            .backtrack_limit(usize::MAX)
            .build()
            .unwrap_or_else(|e| panic!("{pattern}: {e}"))
    });
    Pattern { relaxed, anchored }
}

/// One match and its groups.
#[derive(Debug, Clone)]
pub struct Captures<'t> {
    text: &'t str,
    groups: Vec<Option<Range<usize>>>,
}

impl<'t> Captures<'t> {
    pub fn get(&self, i: usize) -> Option<&'t str> {
        self.groups.get(i)?.clone().map(|r| &self.text[r])
    }

    pub fn range(&self) -> Range<usize> {
        self.groups[0].clone().unwrap()
    }

    /// `match.groups()`: every group but the whole match.
    pub fn groups(&self) -> impl Iterator<Item = Option<&'t str>> + '_ {
        (1..self.groups.len()).map(|i| self.get(i))
    }
}

impl Index<usize> for Captures<'_> {
    type Output = str;

    fn index(&self, i: usize) -> &str {
        self.get(i).unwrap_or_else(|| panic!("group {i} didn't take part in the match"))
    }
}

fn next_char_boundary(text: &str, pos: usize) -> usize {
    pos + text[pos..].chars().next().map_or(1, char::len_utf8)
}

impl Pattern {
    /// The leftmost match starting at or after `pos`, as Python's `search(text, pos)` finds it.
    pub fn captures_at<'t>(&self, text: &'t str, mut pos: usize) -> Option<Captures<'t>> {
        let Some(anchored) = &self.anchored else {
            let caps = self.relaxed.captures_at(text, pos)?;
            return Some(Captures { text, groups: caps.iter().map(|m| m.map(|m| m.range())).collect() });
        };
        // Positions before `checked_until` lie inside a relaxed match that has already been
        // scanned; searching again from each would be quadratic, so they go straight to the
        // backtracker, which rejects most of them at their first lookbehind as Python's does.
        let mut checked_until = pos;
        loop {
            if pos >= checked_until {
                let relaxed = self.relaxed.find_at(text, pos)?;
                pos = relaxed.start();
                checked_until = relaxed.end();
            }
            if let Some(caps) = anchored.captures_from_pos(text, pos).expect("backtracking has no limit") {
                return Some(Captures { text, groups: caps.iter().map(|m| m.map(|m| m.range())).collect() });
            }
            if pos == text.len() {
                return None;
            }
            pos = next_char_boundary(text, pos);
        }
    }

    /// `finditer`: successive non-overlapping matches. None of the reference's patterns used this
    /// way can match empty text, so Python's rules for empty matches aren't reproduced.
    pub fn captures_iter<'r, 't>(&'r self, text: &'t str) -> impl Iterator<Item = Captures<'t>> + 'r
    where
        't: 'r,
    {
        let mut pos = 0;
        std::iter::from_fn(move || {
            let caps = self.captures_at(text, pos)?;
            let range = caps.range();
            assert!(!range.is_empty(), "empty match");
            pos = range.end;
            Some(caps)
        })
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.captures_at(text, 0).is_some()
    }

    /// The first match's span.
    pub fn find(&self, text: &str) -> Option<Range<usize>> {
        self.captures_at(text, 0).map(|c| c.range())
    }

    /// `pattern.sub(f, text)`.
    pub fn sub(&self, text: &str, mut f: impl FnMut(&Captures) -> String) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for caps in self.captures_iter(text) {
            let range = caps.range();
            out.push_str(&text[last..range.start]);
            out.push_str(&f(&caps));
            last = range.end;
        }
        out.push_str(&text[last..]);
        out
    }

    /// `pattern.sub(replacement, text)` with a fixed replacement.
    pub fn sub_str(&self, text: &str, replacement: &str) -> String {
        self.sub(text, |_| replacement.to_owned())
    }

    /// `pattern.split(text)`: the text between matches, with each match's groups in between
    /// (`None` for a group that didn't take part).
    pub fn split<'t>(&self, text: &'t str) -> Vec<Option<&'t str>> {
        let mut out = Vec::new();
        let mut last = 0;
        for caps in self.captures_iter(text) {
            let range = caps.range();
            out.push(Some(&text[last..range.start]));
            out.extend(caps.groups());
            last = range.end;
        }
        out.push(Some(&text[last..]));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_classes() {
        assert_eq!(compile(r"\w+").find("é\u{301}x"), Some(0..2));
        assert!(compile(r"\s").is_match("\x1c"));
        assert!(compile_i("[a-z]").is_match("\u{131}"));
        assert!(compile_i("i").is_match("\u{130}"));
        assert!(!compile(r"\bx\b").is_match("²x"));
    }

    #[test]
    fn lookarounds_backtrack() {
        let p = compile(r"(?<![a-z])\d+(?:k|km)?(?![a-z])");
        let found: Vec<_> = p.captures_iter("x12 5kmh 7km 9k").map(|c| c[0].to_owned()).collect();
        assert_eq!(found, ["2", "7km", "9k"]);
    }
}
