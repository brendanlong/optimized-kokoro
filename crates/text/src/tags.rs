//! Inline tags in request text (`text_processor.py`): `[pause:Xs]`, and emoji removal. The
//! reference's `[voice:]`, `[rate:]` and `[baserate:]` tags aren't supported (`docs/divergences.md`):
//! they are read as text, as the reference reads them when voice tags aren't allowed.

use crate::normalize::Options;
use crate::py::{self, parse_float};
use crate::pyre::{Pattern, compile_i};
use crate::tables::{EMOJI_MODIFIER, EMOJI_START};
use std::borrow::Cow;
use std::fmt::Write;
use std::sync::LazyLock;

/// The server settings these checks read, with the deployed defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    /// Longest single pause; longer ones are shortened to this.
    pub max_pause_duration_s: f64,
    /// Most silence one request may ask for in total.
    pub max_total_pause_s: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_pause_duration_s: 60.0, max_total_pause_s: 300.0 }
    }
}

pub(crate) static PAUSE_TAG: LazyLock<Pattern> = LazyLock::new(|| compile_i(r"\[pause:(\d+(?:\.\d+)?)s\]"));

fn class(tables: &[&[(char, char)]], extra: &str) -> String {
    let mut out = String::from("[");
    for table in tables {
        for &(a, b) in *table {
            let _ = write!(out, "\\x{{{:x}}}-\\x{{{:x}}}", a as u32, b as u32);
        }
    }
    out.push_str(extra);
    out.push(']');
    out
}

static EMOJI: LazyLock<regex::Regex> = LazyLock::new(|| {
    let start = class(&[EMOJI_START], "");
    let rest = class(&[EMOJI_START, EMOJI_MODIFIER], r"\x{200D}\x{FE0F}\x{20E3}\x{E0020}-\x{E007F}");
    regex::Regex::new(&format!(r" ?(?:[0-9#*]\x{{FE0F}}?\x{{20E3}}|{start}{rest}*) ?")).unwrap()
});

/// `strip_emoji`: each emoji sequence, with a space either side, becomes one space.
pub fn strip_emoji(text: &str) -> String {
    EMOJI.replace_all(text, " ").into_owned()
}

/// A request the reference server refuses before streaming.
#[derive(Debug, Clone, PartialEq)]
pub enum Rejected {
    NothingToSay,
    TooMuchSilence { total_s: f64, limit_s: f64 },
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingToSay => f.write_str("Input contains no speakable text"),
            Self::TooMuchSilence { total_s, limit_s } => {
                write!(f, "Total pause duration {total_s:.1}s exceeds the {limit_s:.1}s limit")
            }
        }
    }
}

impl std::error::Error for Rejected {}

/// `check_speakable` with voice tags not allowed: refuses text that is empty once emoji (if
/// removed) are taken out.
pub fn check_speakable(text: &str, options: &Options) -> Result<(), Rejected> {
    let text = if options.remove_emoji { Cow::Owned(strip_emoji(text)) } else { Cow::Borrowed(text) };
    if py::strip(&text).is_empty() { Err(Rejected::NothingToSay) } else { Ok(()) }
}

/// `check_pause_budget`: refuses a request whose pauses, each capped, add up past the limit.
pub fn check_pause_budget(text: &str, limits: &Limits) -> Result<(), Rejected> {
    let pauses = PAUSE_TAG.captures_iter(text).map(|c| parse_float(&c[1]).min(limits.max_pause_duration_s));
    let total_s = fsum(pauses);
    if total_s > limits.max_total_pause_s {
        return Err(Rejected::TooMuchSilence { total_s, limit_s: limits.max_total_pause_s });
    }
    Ok(())
}

/// `math.fsum`: the correctly rounded sum (Shewchuk's algorithm, as CPython implements it).
fn fsum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut partials: Vec<f64> = Vec::new();
    for mut x in values {
        let mut i = 0;
        for j in 0..partials.len() {
            let mut y = partials[j];
            if x.abs() < y.abs() {
                std::mem::swap(&mut x, &mut y);
            }
            let hi = x + y;
            let lo = y - (hi - x);
            if lo != 0.0 {
                partials[i] = lo;
                i += 1;
            }
            x = hi;
        }
        partials.truncate(i);
        partials.push(x);
    }
    let Some(mut hi) = partials.pop() else { return 0.0 };
    let mut lo = 0.0;
    while let Some(y) = partials.pop() {
        let x = hi;
        hi = x + y;
        let yr = hi - x;
        lo = y - yr;
        if lo != 0.0 {
            break;
        }
    }
    if let Some(&next) = partials.last()
        && ((lo < 0.0 && next < 0.0) || (lo > 0.0 && next > 0.0))
    {
        let y = lo * 2.0;
        let x = hi + y;
        if y == x - hi {
            hi = x;
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsum_is_exact() {
        assert_eq!(fsum(std::iter::repeat_n(0.1, 10)), 1.0);
        assert_eq!(fsum([1e100, 1.0, -1e100, 1e-100]), 1.0);
        assert_eq!(fsum([]), 0.0);
    }

    #[test]
    fn pause_budget() {
        assert!(check_pause_budget("[pause:60s] [pause:500s]", &Limits::default()).is_ok());
        let err = check_pause_budget(&"[pause:61s]".repeat(6), &Limits::default()).unwrap_err();
        assert_eq!(err.to_string(), "Total pause duration 360.0s exceeds the 300.0s limit");
    }
}
