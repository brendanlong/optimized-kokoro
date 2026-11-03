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
use std::collections::HashMap;
use std::sync::LazyLock;

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
        let (ranges, case_fixed) = self.parse_class();
        self.emit(&emit_class(&ranges, case_fixed));
    }

    /// A class's characters, and whether they must be matched with case folding off.
    fn parse_class(&mut self) -> (Ranges, bool) {
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
        (ranges, shorthand)
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

/// A lookaround or `\b` at the start or end of a pattern, checked natively: running the
/// backtracker costs about a microsecond a position even when it fails at once.
#[derive(Debug, Clone)]
enum Cond {
    /// `(?<=...)`/`(?=...)`, or `(?<!...)`/`(?!...)` when negated: alternatives, each one set per
    /// character just before (behind) or after the position.
    Look { behind: bool, negated: bool, alternatives: Vec<Vec<Ranges>> },
    /// `\b`.
    Boundary,
}

fn in_ranges(ranges: &Ranges, c: char) -> bool {
    let c = c as u32;
    let i = ranges.partition_point(|&(_, hi)| hi < c);
    ranges.get(i).is_some_and(|&(lo, _)| lo <= c)
}

impl Cond {
    fn holds(&self, text: &str, pos: usize) -> bool {
        match self {
            Self::Boundary => {
                let word = |c: Option<char>| c.is_some_and(crate::py::is_word);
                word(text[..pos].chars().next_back()) != word(text[pos..].chars().next())
            }
            Self::Look { behind, negated, alternatives } => {
                let matched = alternatives.iter().any(|sets| {
                    if *behind {
                        let mut chars = text[..pos].chars().rev();
                        sets.iter().rev().all(|set| chars.next().is_some_and(|c| in_ranges(set, c)))
                    } else {
                        let mut chars = text[pos..].chars();
                        sets.iter().all(|set| chars.next().is_some_and(|c| in_ranges(set, c)))
                    }
                });
                matched != *negated
            }
        }
    }
}

/// A top-level piece of a pattern.
enum Item<'p> {
    Boundary,
    Look { behind: bool, negated: bool, body: &'p str },
    Other,
}

/// The pattern's top-level items with their spans, or `None` if it has a top-level alternation
/// (whose branches would each need their own conditions).
fn top_level_items(pattern: &str) -> Option<Vec<(Item<'_>, Range<usize>)>> {
    let b = pattern.as_bytes();
    let mut items: Vec<(Item, Range<usize>)> = Vec::new();
    let mut i = 0;
    // The end of the class starting at `i` (just after `[`).
    let class_end = |mut i: usize| {
        if b.get(i) == Some(&b'^') {
            i += 1;
        }
        if b.get(i) == Some(&b']') {
            i += 1;
        }
        while b[i] != b']' {
            i += if b[i] == b'\\' { 2 } else { 1 };
        }
        i + 1
    };
    while i < b.len() {
        let start = i;
        let item = match b[i] {
            b'|' => return None,
            b'\\' if b.get(i + 1) == Some(&b'b') => {
                i += 2;
                Item::Boundary
            }
            b'\\' => {
                i += match b[i + 1] {
                    b'u' => 6,
                    b'U' => 10,
                    _ => 1 + pattern[i + 1..].chars().next().unwrap().len_utf8(),
                };
                Item::Other
            }
            b'[' => {
                i = class_end(i + 1);
                Item::Other
            }
            b'(' => {
                let mut depth = 0;
                loop {
                    match b[i] {
                        b'\\' => i += 1,
                        b'[' => {
                            i = class_end(i + 1);
                            continue;
                        }
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                i += 1;
                let group = &pattern[start..i];
                let body = |prefix: usize| &group[prefix..group.len() - 1];
                match group.get(..4) {
                    Some("(?<=") => Item::Look { behind: true, negated: false, body: body(4) },
                    Some("(?<!") => Item::Look { behind: true, negated: true, body: body(4) },
                    _ => match group.get(..3) {
                        Some("(?=") => Item::Look { behind: false, negated: false, body: body(3) },
                        Some("(?!") => Item::Look { behind: false, negated: true, body: body(3) },
                        _ => Item::Other,
                    },
                }
            }
            _ => {
                i += pattern[i..].chars().next().unwrap().len_utf8();
                Item::Other
            }
        };
        if matches!(b.get(i), Some(b'*' | b'+' | b'?' | b'{')) {
            while i < b.len() && matches!(b[i], b'*' | b'+' | b'?') {
                i += 1;
            }
            if b.get(i) == Some(&b'{') {
                i = pattern[i..].find('}').map_or(b.len(), |k| i + k + 1);
                if b.get(i) == Some(&b'?') {
                    i += 1;
                }
            }
            items.push((Item::Other, start..i));
        } else {
            items.push((item, start..i));
        }
    }
    Some(items)
}

/// A lookaround body as alternatives of single characters, or `None` if it's anything more.
/// Under IGNORECASE only sets that case folding can't change are accepted.
fn look_alternatives(body: &str, ignore_case: bool) -> Option<Vec<Vec<Ranges>>> {
    let mut t = Translator {
        chars: body.chars().peekable(),
        ignore_case,
        full: String::new(),
        relaxed: String::new(),
        lookaround_depth: 0,
        fancy: false,
    };
    let mut alternatives = vec![Vec::new()];
    while let Some(c) = t.chars.next() {
        let set = match c {
            '|' => {
                alternatives.push(Vec::new());
                continue;
            }
            '(' | ')' | '*' | '+' | '?' | '{' | '.' | '^' | '$' => return None,
            c => atom(&mut t, c, ignore_case)?,
        };
        alternatives.last_mut().unwrap().push(set);
    }
    Some(alternatives)
}

/// The characters one class, escape or literal (starting with `c`) matches, folded as Python's
/// IGNORECASE does; `None` for `\b` and other escapes that aren't a character.
fn atom(t: &mut Translator, c: char, ignore_case: bool) -> Option<Ranges> {
    let (ranges, case_fixed) = match c {
        '[' => t.parse_class(),
        '\\' => {
            let e = t.chars.next()?;
            match Translator::shorthand(e) {
                Some(r) => (normalize(r), true),
                None if e.is_ascii_alphanumeric() && !"nrtuU".contains(e) => return None,
                None => {
                    let c = t.escaped_literal(e) as u32;
                    (vec![(c, c)], false)
                }
            }
        }
        c => (vec![(c as u32, c as u32)], false),
    };
    Some(if ignore_case && !case_fixed { case_closure(&ranges) } else { ranges })
}

/// Characters grouped by [`crate::py::fold`], for the groups with more than one member.
static FOLD_CLASSES: LazyLock<Vec<Vec<char>>> = LazyLock::new(|| {
    // Only characters that fold to something else join a class with more than one member.
    let mut classes: HashMap<char, Vec<char>> = HashMap::new();
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        let folded = crate::py::fold(c);
        if folded != c {
            classes.entry(folded).or_insert_with(|| vec![folded]).push(c);
        }
    }
    classes.into_values().collect()
});

/// The set as Python's IGNORECASE matches it: every character that folds like a member.
fn case_closure(ranges: &Ranges) -> Ranges {
    let mut out = ranges.clone();
    for class in FOLD_CLASSES.iter() {
        if class.iter().any(|&c| in_ranges(ranges, c)) {
            out.extend(class.iter().map(|&c| (c as u32, c as u32)));
        }
    }
    normalize(out)
}

type Language = Vec<Vec<Ranges>>;

/// Every string a lookaround-free pattern can match, as sequences of character sets, if it has no
/// unbounded or ranged repetition and at most a few hundred of them.
fn finite_language(pattern: &str, ignore_case: bool) -> Option<Language> {
    fn alternation(t: &mut Translator, ignore_case: bool) -> Option<Language> {
        let mut out = concatenation(t, ignore_case)?;
        while t.chars.next_if_eq(&'|').is_some() {
            out.extend(concatenation(t, ignore_case)?);
        }
        Some(out)
    }
    fn concatenation(t: &mut Translator, ignore_case: bool) -> Option<Language> {
        let mut seqs: Language = vec![Vec::new()];
        while let Some(&c) = t.chars.peek() {
            if c == '|' || c == ')' {
                break;
            }
            t.chars.next();
            let mut piece: Language = match c {
                '(' => {
                    if t.chars.next_if_eq(&'?').is_some() && t.chars.next() != Some(':') {
                        return None;
                    }
                    let inner = alternation(t, ignore_case)?;
                    (t.chars.next() == Some(')')).then_some(inner)?
                }
                '*' | '+' | '?' | '{' | '.' | '^' | '$' => return None,
                c => vec![vec![atom(t, c, ignore_case)?]],
            };
            match t.chars.peek() {
                Some('?') => {
                    t.chars.next();
                    if matches!(t.chars.peek(), Some('?' | '*' | '+' | '{')) {
                        return None;
                    }
                    piece.push(Vec::new());
                }
                Some('{') => {
                    t.chars.next();
                    let digits: String = std::iter::from_fn(|| t.chars.next_if(char::is_ascii_digit)).collect();
                    let n: usize = digits.parse().ok()?;
                    if t.chars.next() != Some('}') || matches!(t.chars.peek(), Some('?' | '*' | '+' | '{')) {
                        return None;
                    }
                    let one = std::mem::take(&mut piece);
                    piece = vec![Vec::new()];
                    for _ in 0..n {
                        piece = piece.iter().flat_map(|p| one.iter().map(move |o| [p.clone(), o.clone()].concat())).collect();
                    }
                }
                Some('*' | '+') => return None,
                _ => {}
            }
            seqs = seqs.iter().flat_map(|s| piece.iter().map(move |p| [s.clone(), p.clone()].concat())).collect();
            if seqs.len() > 512 {
                return None;
            }
        }
        Some(seqs)
    }
    let mut t = Translator {
        chars: pattern.chars().peekable(),
        ignore_case,
        full: String::new(),
        relaxed: String::new(),
        lookaround_depth: 0,
        fancy: false,
    };
    let language = alternation(&mut t, ignore_case)?;
    t.chars.next().is_none().then_some(language)
}

/// Where `sequence` ends if it matches the text from byte `start`.
fn sequence_end(text: &str, start: usize, sequence: &[Ranges]) -> Option<usize> {
    let mut chars = text[start..].char_indices();
    for set in sequence {
        let (_, c) = chars.next()?;
        if !in_ranges(set, c) {
            return None;
        }
    }
    Some(chars.next().map_or(text.len(), |(i, _)| start + i))
}

fn cond(item: &Item, ignore_case: bool) -> Option<Cond> {
    match *item {
        Item::Boundary => Some(Cond::Boundary),
        Item::Look { behind, negated, body } => {
            Some(Cond::Look { behind, negated, alternatives: look_alternatives(body, ignore_case)? })
        }
        Item::Other => None,
    }
}

/// The conditions at a pattern's start and end that can be checked natively, and whether they
/// are all its lookarounds, so that a lookaround-free match that passes them is the match.
struct Conds {
    leading: Vec<Cond>,
    trailing: Vec<Cond>,
    exact: bool,
    language: Option<Language>,
}

fn conds(pattern: &str, ignore_case: bool) -> Conds {
    let none = Conds { leading: Vec::new(), trailing: Vec::new(), exact: false, language: None };
    let Some(items) = top_level_items(pattern) else { return none };
    let is_look = |item: &Item, behind_wanted: bool| match item {
        Item::Boundary => true,
        Item::Look { behind, .. } => *behind == behind_wanted,
        Item::Other => false,
    };
    let lead = items.iter().take_while(|(item, _)| is_look(item, true)).count();
    let trail = items[lead..].iter().rev().take_while(|(item, _)| is_look(item, false)).count();
    let leading: Vec<Cond> = items[..lead].iter().map_while(|(item, _)| cond(item, ignore_case)).collect();
    let trailing: Vec<Cond> = items[items.len() - trail..].iter().map_while(|(item, _)| cond(item, ignore_case)).collect();
    let core_start = items.get(lead).map_or(pattern.len(), |(_, r)| r.start);
    let core_end = if trail == 0 { pattern.len() } else { items[items.len() - trail].1.start };
    let core = &pattern[core_start..core_end.max(core_start)];
    let core_clean = !Translator {
        chars: core.chars().peekable(),
        ignore_case,
        full: String::new(),
        relaxed: String::new(),
        lookaround_depth: 0,
        fancy: false,
    }
    .run()
    .fancy;
    let exact = core_clean && leading.len() == lead && trailing.len() == trail;
    let language = if exact { finite_language(core, ignore_case) } else { None };
    Conds { leading, trailing, exact, language }
}

/// A compiled Python pattern.
#[derive(Debug, Clone)]
pub struct Pattern {
    relaxed: regex::Regex,
    /// The full pattern after `\G`, when it needs lookaround.
    anchored: Option<fancy_regex::Regex>,
    leading: Vec<Cond>,
    trailing: Vec<Cond>,
    /// Whether `leading` and `trailing` are all the lookarounds: then when the relaxed pattern's
    /// match passes them it is the full pattern's match, and the backtracker only runs when not.
    exact: bool,
    /// When `exact` and the core between the conditions can only match a few strings: all of
    /// them, so a start can be ruled out without the backtracker.
    language: Option<Language>,
    /// The translated pattern, for checking the fast paths against the plain backtracker.
    #[cfg_attr(not(test), allow(dead_code))]
    full: String,
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
    let Conds { leading, trailing, exact, language } = conds(pattern, ignore_case);
    Pattern { relaxed, anchored, leading, trailing, exact, language, full: t.full }
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
    /// Every match's groups as the plain backtracker finds them, trying each position in turn:
    /// what the fast paths must reproduce.
    #[cfg(test)]
    pub(crate) fn reference(&self) -> impl Fn(&str) -> Vec<Vec<Option<Range<usize>>>> + use<> {
        let re = fancy_regex::RegexBuilder::new(&self.full).backtrack_limit(usize::MAX).build().unwrap();
        move |text| re.captures_iter(text).map(|c| c.unwrap().iter().map(|m| m.map(|m| m.range())).collect()).collect()
    }

    #[cfg(test)]
    pub(crate) fn matches(&self, text: &str) -> Vec<Vec<Option<Range<usize>>>> {
        self.captures_iter(text).map(|c| c.groups.clone()).collect()
    }

    /// The groups of the relaxed pattern's leftmost-first match at `range`.
    fn relaxed_captures<'t>(&self, text: &'t str, range: Range<usize>) -> Captures<'t> {
        if self.relaxed.captures_len() == 1 {
            return Captures { text, groups: vec![Some(range)] };
        }
        let caps = self.relaxed.captures_at(text, range.start).expect("the match found from here");
        Captures { text, groups: caps.iter().map(|m| m.map(|m| m.range())).collect() }
    }

    /// The leftmost match starting at or after `pos`, as Python's `search(text, pos)` finds it.
    pub fn captures_at<'t>(&self, text: &'t str, mut pos: usize) -> Option<Captures<'t>> {
        let Some(anchored) = &self.anchored else {
            let range = self.relaxed.find_at(text, pos)?.range();
            return Some(self.relaxed_captures(text, range));
        };
        // Positions before `checked_until` lie inside a relaxed match that has already been
        // scanned; searching again from each would be quadratic, so they go straight to the
        // backtracker, which rejects most of them at their first lookbehind as Python's does.
        let mut checked_until = pos;
        loop {
            if pos >= checked_until {
                let range = self.relaxed.find_at(text, pos)?.range();
                // The relaxed pattern's first match here is the backtracker's first try; when it
                // passes the conditions it's the match, at a fraction of the cost.
                if self.exact
                    && self.leading.iter().all(|c| c.holds(text, range.start))
                    && self.trailing.iter().all(|c| c.holds(text, range.end))
                {
                    return Some(self.relaxed_captures(text, range));
                }
                if let Some(language) = &self.language {
                    // Every string the core can match is known, so whether any other path
                    // succeeds here is a native check; only then does the backtracker run, to
                    // find which one and its groups. Relaxed matches are short, so searching
                    // again from the next character stays linear.
                    let s = range.start;
                    let other_path = self.leading.iter().all(|c| c.holds(text, s))
                        && language.iter().any(|sequence| {
                            sequence_end(text, s, sequence).is_some_and(|e| self.trailing.iter().all(|c| c.holds(text, e)))
                        });
                    if other_path
                        && let Some(caps) = anchored.captures_from_pos(text, s).expect("backtracking has no limit")
                    {
                        return Some(Captures { text, groups: caps.iter().map(|m| m.map(|m| m.range())).collect() });
                    }
                    if s == text.len() {
                        return None;
                    }
                    pos = next_char_boundary(text, s);
                    continue;
                }
                (pos, checked_until) = (range.start, range.end);
            }
            if self.leading.iter().all(|c| c.holds(text, pos))
                && let Some(caps) = anchored.captures_from_pos(text, pos).expect("backtracking has no limit")
            {
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
        self.sub_with(text, |caps, out| out.push_str(&f(caps)))
    }

    /// `pattern.sub(replacement, text)` with a fixed replacement.
    pub fn sub_str(&self, text: &str, replacement: &str) -> String {
        self.sub_with(text, |_, out| out.push_str(replacement))
    }

    fn sub_with(&self, text: &str, mut write: impl FnMut(&Captures, &mut String)) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for caps in self.captures_iter(text) {
            let range = caps.range();
            out.push_str(&text[last..range.start]);
            write(&caps, &mut out);
            last = range.end;
        }
        out.push_str(&text[last..]);
        out
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
    fn fast_paths_apply() {
        let p = compile_i(r"\b(how|what|there|they)['’]re\b");
        assert!(p.exact && p.language.as_ref().is_some_and(|l| l.len() == 4));
        let p = compile_i(r"\b(y)eah?\b");
        assert!(p.exact && p.language.as_ref().is_some_and(|l| l.len() == 2));
        let p = compile_i(r"(?<=[A-Z])\.(?=[A-Z])");
        assert!(p.exact && p.language.is_some() && p.leading.len() == 1);
        let p = compile_i(r"(?<![^\W_])(?<![^\W_][.,])(-?)(\d+(?:\.\d+)?)(k|m)*(?![^\W_]|\.\d|,\d)");
        assert!(p.exact && p.language.is_none() && p.leading.len() == 2 && p.trailing.len() == 1);
        let p = compile(r"\b(?:Mr\.|MR\.(?= [A-Z]))");
        assert!(!p.exact && p.leading.len() == 1);
    }

    #[test]
    fn lookarounds_backtrack() {
        let p = compile(r"(?<![a-z])\d+(?:k|km)?(?![a-z])");
        let found: Vec<_> = p.captures_iter("x12 5kmh 7km 9k").map(|c| c[0].to_owned()).collect();
        assert_eq!(found, ["2", "7km", "9k"]);
    }
}
