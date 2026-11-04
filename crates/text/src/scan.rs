//! Hand-written matchers for the normalizer patterns whose backtracking is costly: each finds the
//! same matches as Python's `re` on the original pattern (the tests check them against the
//! translated pattern on random text), in linear time.

use crate::py::{fold, is_space, is_word};
use std::ops::Range;

/// The text as characters with their byte offsets, so matchers can index and look around freely.
struct Chars<'t> {
    text: &'t str,
    chars: Vec<(usize, char)>,
}

impl<'t> Chars<'t> {
    fn new(text: &'t str) -> Self {
        Self { text, chars: text.char_indices().collect() }
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn get(&self, i: usize) -> Option<char> {
        self.chars.get(i).map(|&(_, c)| c)
    }

    fn is(&self, i: usize, f: impl Fn(char) -> bool) -> bool {
        self.get(i).is_some_and(f)
    }

    fn byte(&self, i: usize) -> usize {
        self.chars.get(i).map_or(self.text.len(), |&(b, _)| b)
    }

    fn span(&self, r: Range<usize>) -> Range<usize> {
        self.byte(r.start)..self.byte(r.end)
    }

    fn word_before(&self, i: usize) -> bool {
        i > 0 && self.is(i - 1, is_word)
    }

    /// `\b` at character `i`.
    fn boundary(&self, i: usize) -> bool {
        self.word_before(i) != self.is(i, is_word)
    }

    /// The end of the run of characters matching `f` from `i`, at most `max` long.
    fn run(&self, i: usize, max: usize, f: impl Fn(char) -> bool) -> usize {
        let mut j = i;
        while j - i < max && self.is(j, &f) {
            j += 1;
        }
        j
    }
}

/// `CAPS_RUN_PATTERN`, `\b[A-Z]+(?:[^A-Za-z]+[A-Z]+)+\b`.
///
/// From a start (an uppercase letter after a non-word character), the pattern walks a chain of
/// uppercase runs separated by runs of anything but ASCII letters, and backtracks to the last
/// run end (from the second run on) followed by a non-word character. If there's none, every
/// later start in the same chain fails too, since its ends are a subset, so the scan skips the
/// chain instead of retrying each position, which made the regex quadratic.
pub fn caps_runs(text: &str) -> Vec<Range<usize>> {
    let t = Chars::new(text);
    let upper = |i: usize| t.is(i, |c| c.is_ascii_uppercase());
    let mut out = Vec::new();
    let mut i = 0;
    while i < t.len() {
        if !upper(i) || t.word_before(i) {
            i += 1;
            continue;
        }
        let mut end = t.run(i, usize::MAX, |c| c.is_ascii_uppercase());
        let mut runs = 1;
        let mut best = None;
        loop {
            if runs >= 2 && !t.is(end, is_word) {
                best = Some(end);
            }
            let next = t.run(end, usize::MAX, |c| !c.is_ascii_alphabetic());
            if next == end || !upper(next) {
                break;
            }
            end = t.run(next, usize::MAX, |c| c.is_ascii_uppercase());
            runs += 1;
        }
        match best {
            Some(best) => {
                out.push(t.span(i..best));
                i = best;
            }
            None => i = end,
        }
    }
    out
}

/// `[a-z]` under IGNORECASE in Python: ASCII letters, plus ı, İ, ſ and the Kelvin sign.
fn ci_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, 'ı' | 'İ' | 'ſ' | 'K')
}

/// `EMAIL_PATTERN`, `\b[a-zA-Z0-9._%+-]{1,64}@[a-zA-Z0-9.-]{1,253}\.[a-z]{2,}\b` (IGNORECASE).
///
/// Neither side of the `@` can contain one, so each `@` is tried once: the domain part is the
/// same for every start, and the start is the leftmost one within 64 characters that the
/// local part reaches and where `\b` holds.
pub fn emails(text: &str) -> Vec<Range<usize>> {
    let t = Chars::new(text);
    let local = |c: char| ci_letter(c) || c.is_ascii_digit() || "._%+-".contains(c);
    let domain = |c: char| ci_letter(c) || c.is_ascii_digit() || ".-".contains(c);
    let mut out = Vec::new();
    let mut from = 0;
    for at in 0..t.len() {
        if at < from || t.get(at) != Some('@') {
            continue;
        }
        let Some(end) = email_domain_end(&t, at + 1, domain) else { continue };
        let mut lo = at;
        while lo > from && at - lo < 64 && t.is(lo - 1, local) {
            lo -= 1;
        }
        if let Some(start) = (lo..at).find(|&s| t.boundary(s)) {
            out.push(t.span(start..end));
            from = end;
        }
    }
    out
}

/// `[a-zA-Z0-9.-]{1,253}\.[a-z]{2,}\b` from `start`: the greedy domain backs off to each `.`, and
/// the top-level part only ends where `\b` holds, after its whole letter run.
fn email_domain_end(t: &Chars, start: usize, domain: impl Fn(char) -> bool) -> Option<usize> {
    let greedy = t.run(start, 253, domain);
    (start + 1..=greedy).rev().filter(|&dot| t.get(dot) == Some('.')).find_map(|dot| {
        let end = t.run(dot + 1, usize::MAX, ci_letter);
        (end - (dot + 1) >= 2 && !t.is(end, is_word)).then_some(end)
    })
}

/// One `UNIT_PATTERN` match: the whole span, the signed number with its decimals (group 1), and
/// the unit (group 6).
pub struct UnitMatch {
    pub range: Range<usize>,
    pub number: Range<usize>,
    pub unit: Range<usize>,
}

/// `UNIT_PATTERN`: `((?<!\w)(?<!\w[.,])([+-]?)(\d{1,3}(,\d{3})*|\d+,\d+|\d+)(\.\d+)?)\s*`, one of
/// `units` (in the pattern's order, matched as IGNORECASE literals), then `(?=[^\w\d]|\b)`.
///
/// The number's alternatives and lengths are tried in the regex's order. `\s*` needs no
/// backtracking: every unit starts with a non-space, so only the whole whitespace run can work.
/// Every unit ends in a word character, so the lookahead means a non-word character or the end.
pub fn units(text: &str, units: &[&str]) -> Vec<UnitMatch> {
    let t = Chars::new(text);
    let units: Vec<Vec<char>> = units.iter().map(|u| u.chars().map(fold).collect()).collect();
    let digit = |i: usize| t.is(i, |c| crate::py::digit_value(c).is_some());
    let digits = |i: usize| t.run(i, usize::MAX, |c| crate::py::digit_value(c).is_some()) - i;
    let firsts: Vec<char> = units.iter().map(|u| u[0]).collect();
    let unit_at = |u: usize| {
        if !t.get(u).is_some_and(|c| firsts.contains(&fold(c))) {
            return None;
        }
        units.iter().find_map(|unit| {
            let fits = unit.iter().enumerate().all(|(k, &c)| t.get(u + k).is_some_and(|x| fold(x) == c));
            (fits && !t.is(u + unit.len(), is_word)).then_some(u + unit.len())
        })
    };
    // After the integer part: optional decimals (longest first), whitespace, then a unit.
    let rest = |q: usize| -> Option<(usize, usize, usize)> {
        let mut ends = Vec::new();
        if t.get(q) == Some('.') {
            ends.extend((1..=digits(q + 1)).rev().map(|d| q + 1 + d));
        }
        ends.push(q);
        ends.into_iter().find_map(|number_end| {
            let u = t.run(number_end, usize::MAX, is_space);
            unit_at(u).map(|end| (number_end, u, end))
        })
    };
    let mut out = Vec::new();
    let mut s = 0;
    while s < t.len() {
        let behind_ok = !t.word_before(s) && !(s >= 2 && t.is(s - 1, |c| c == '.' || c == ',') && t.word_before(s - 1));
        let p = if t.is(s, |c| c == '+' || c == '-') { s + 1 } else { s };
        let found = (behind_ok && digit(p)).then(|| {
            let run = digits(p);
            // \d{1,3}(,\d{3})*
            let thousands = (1..=run.min(3)).rev().find_map(|a| {
                let mut ends = vec![p + a];
                while t.get(ends[ends.len() - 1]) == Some(',') && digits(ends[ends.len() - 1] + 1) >= 3 {
                    ends.push(ends[ends.len() - 1] + 4);
                }
                ends.into_iter().rev().find_map(rest)
            });
            // \d+,\d+
            let comma = || {
                (t.get(p + run) == Some(',') && digit(p + run + 1))
                    .then(|| (1..=digits(p + run + 1)).rev().find_map(|b| rest(p + run + 1 + b)))
                    .flatten()
            };
            // \d+
            let plain = || (1..=run).rev().find_map(|a| rest(p + a));
            thousands.or_else(comma).or_else(plain)
        });
        match found.flatten() {
            Some((number_end, unit_start, end)) => {
                out.push(UnitMatch {
                    range: t.span(s..end),
                    number: t.span(s..number_end),
                    unit: t.span(unit_start..end),
                });
                s = end;
            }
            None => s += 1,
        }
    }
    out
}

/// One `TIME_PATTERN` match: the whole span, the time (group 1) and am/pm (group 4).
pub struct TimeMatch {
    pub range: Range<usize>,
    pub time: Range<usize>,
    pub half: Option<Range<usize>>,
}

/// `TIME_PATTERN`, `([0-9]{1,2} ?: ?[0-9]{2}( ?: ?[0-9]{2})?)( ?(pm|am)\b)?` (IGNORECASE). Once
/// the hours and minutes match, the rest is optional, so the first way each part matches is final.
pub fn times(text: &str) -> Vec<TimeMatch> {
    let t = Chars::new(text);
    let d = |i: usize| t.is(i, |c| c.is_ascii_digit());
    let space_options = |i: usize| if t.get(i) == Some(' ') { vec![i + 1, i] } else { vec![i] };
    // ` ?: ?[0-9]{2}` from i, trying the optional spaces first.
    let colon_pair = |i: usize| {
        space_options(i).into_iter().find_map(|j| {
            (t.get(j) == Some(':'))
                .then(|| space_options(j + 1).into_iter().find(|&k| d(k) && d(k + 1)).map(|k| k + 2))
                .flatten()
        })
    };
    let mut out = Vec::new();
    let mut s = 0;
    while s < t.len() {
        let hours = [2, 1].into_iter().filter(|&n| (0..n).all(|k| d(s + k)));
        let Some(minutes_end) = hours.into_iter().find_map(|n| colon_pair(s + n)) else {
            s += 1;
            continue;
        };
        let time_end = colon_pair(minutes_end).unwrap_or(minutes_end);
        let half = space_options(time_end).into_iter().find_map(|h| {
            let pair: String = (0..2).filter_map(|k| t.get(h + k)).collect::<String>().to_ascii_lowercase();
            (matches!(pair.as_str(), "pm" | "am") && !t.is(h + 2, is_word)).then_some(h..h + 2)
        });
        let end = half.as_ref().map_or(time_end, |h| h.end);
        out.push(TimeMatch { range: t.span(s..end), time: t.span(s..time_end), half: half.map(|h| t.span(h)) });
        s = end;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::pyre::compile_i;

    /// Random text from characters each matcher reacts to, deterministic per seed.
    pub(crate) fn random_texts(alphabet: &[&str], count: usize, seed: u64) -> Vec<String> {
        let mut state = seed;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        (0..count)
            .map(|_| {
                let len = (next() % 30) as usize;
                (0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect()
            })
            .collect()
    }

    type Groups = Vec<Vec<Option<Range<usize>>>>;

    fn assert_same(reference: &impl Fn(&str) -> Groups, text: &str, actual: Groups, groups: &[usize]) {
        let expected: Groups =
            reference(text).into_iter().map(|m| groups.iter().map(|&g| m[g].clone()).collect()).collect();
        assert_eq!(actual, expected, "{text:?}");
    }

    #[test]
    fn caps_runs_match_the_regex() {
        let re = crate::pyre::compile(r"\b[A-Z]+(?:[^A-Za-z]+[A-Z]+)+\b").reference();
        for text in random_texts(&["A", "B", "a", "1", " ", "_", "-", "é", "²", ".", "AB"], 20000, 1) {
            assert_same(&re, &text, caps_runs(&text).into_iter().map(|r| vec![Some(r)]).collect(), &[0]);
        }
    }

    #[test]
    fn emails_match_the_regex() {
        let re = compile_i(r"\b[a-zA-Z0-9._%+-]{1,64}@[a-zA-Z0-9.-]{1,253}\.[a-z]{2,}\b").reference();
        let alphabet = ["a", "B", "1", ".", "@", "-", "_", "%", " ", "co", "ı", "K", "é", "a.b", "x@y.cc"];
        for text in random_texts(&alphabet, 20000, 2) {
            assert_same(&re, &text, emails(&text).into_iter().map(|r| vec![Some(r)]).collect(), &[0]);
        }
        let long = format!("{}@b.cc", "a".repeat(70));
        assert_same(&re, &long, emails(&long).into_iter().map(|r| vec![Some(r)]).collect(), &[0]);
    }

    #[test]
    fn units_match_the_regex() {
        let mut keys: Vec<&str> = crate::english_data::VALID_UNITS.iter().map(|(k, _)| *k).collect();
        keys.sort_unstable_by(|a, b| b.cmp(a));
        let re = compile_i(&format!(
            r"((?<!\w)(?<!\w[.,])([+-]?)(\d{{1,3}}(,\d{{3}})*|\d+,\d+|\d+)(\.\d+)?)\s*({}){{1}}(?=[^\w\d]{{1}}|\b)",
            keys.join("|")
        ))
        .reference();
        let alphabet = [
            "1", "2", "٣", ",", ".", "+", "-", " ", "\t", "km", "m", "s", "K", "B", "°C", "µF", "μf", "mi/h", "Ω", "x",
            "ſ", "ı",
        ];
        for text in random_texts(&alphabet, 30000, 3) {
            let actual =
                units(&text, &keys).into_iter().map(|m| vec![Some(m.range), Some(m.number), Some(m.unit)]).collect();
            assert_same(&re, &text, actual, &[0, 1, 6]);
        }
    }

    #[test]
    fn times_match_the_regex() {
        let re = compile_i(r"([0-9]{1,2} ?: ?[0-9]{2}( ?: ?[0-9]{2})?)( ?(pm|am)\b)?").reference();
        let alphabet = ["1", "2", "3", ":", " ", "pm", "AM", "a", "m", "x", "_", "١"];
        for text in random_texts(&alphabet, 30000, 4) {
            let actual = times(&text).into_iter().map(|m| vec![Some(m.range), Some(m.time), m.half]).collect();
            assert_same(&re, &text, actual, &[0, 1, 4]);
        }
    }
}
