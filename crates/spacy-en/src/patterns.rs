//! The tokenizer's Python regexes, matched in linear time with the `regex` crate.
//!
//! spaCy's prefix, suffix and infix patterns are alternations whose alternatives have at most a
//! fixed-width lookbehind before a lookaround-free body and a lookahead after it. `export.py`
//! splits them up, so the lookarounds become explicit checks around plain regexes. The URL
//! pattern has negative lookaheads and nested optional parts that backtrack badly, so
//! [`UrlMatcher`] ports its structure instead.

use crate::unicode::CharSet;
use regex::Regex;
use serde::Deserialize;

fn compile(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("exported pattern doesn't compile: {e}"))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RuleData {
    behind: Option<BehindData>,
    variants: Vec<String>,
    ahead: Option<AheadData>,
    max_width: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct BehindData {
    pattern: String,
    width: usize,
}

#[derive(Debug, Deserialize)]
struct AheadData {
    pattern: String,
    negate: bool,
}

/// One alternative (or a run of lookaround-free ones) of an affix pattern.
struct Rule {
    /// Matches exactly the `width` characters before the match.
    behind: Option<(Regex, usize)>,
    /// The body, anchored at the start; with a lookahead, its branches in the order Python tries
    /// them, each of fixed width.
    variants: Vec<Regex>,
    /// The whole body, anchored at both ends.
    full: Regex,
    /// Matches at the start of what follows the match; `true` if it must not.
    ahead: Option<(Regex, bool)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Prefix,
    Suffix,
    Infix,
}

/// A prefix (`re.search` anchored at the start), suffix (`re.search` anchored at the end) or
/// infix (`re.finditer`) pattern.
pub(crate) struct Affix {
    rules: Vec<Rule>,
    /// Every rule's body without its lookarounds: it matches wherever the pattern does.
    filter: Regex,
}

impl Affix {
    pub fn new(data: Vec<RuleData>, kind: Kind) -> Self {
        let bodies = data.iter().map(|r| r.variants.join("|")).collect::<Vec<_>>().join("|");
        let filter = match kind {
            Kind::Prefix => format!(r"\A(?:{bodies})"),
            Kind::Suffix => format!(r"(?:{bodies})\n?\z"),
            Kind::Infix => format!("(?:{bodies})"),
        };
        assert!(
            data.iter().all(|r| r.max_width.is_some() || (r.behind.is_none() && r.ahead.is_none())),
            "rules with lookarounds are bounded"
        );
        let rules = data
            .into_iter()
            .map(|r| Rule {
                behind: r.behind.map(|b| (compile(&format!(r"\A(?:{})\z", b.pattern)), b.width)),
                full: compile(&format!(r"\A(?:{})\z", r.variants.join("|"))),
                variants: r.variants.iter().map(|v| compile(&format!(r"\A(?:{v})"))).collect(),
                ahead: r.ahead.map(|a| (compile(&format!(r"\A(?:{})", a.pattern)), a.negate)),
            })
            .collect();
        Affix { rules, filter: compile(&filter) }
    }

    fn behind_ok(rule: &Rule, s: &str, pos: usize) -> bool {
        let Some((re, width)) = &rule.behind else {
            return true;
        };
        let start = match width.checked_sub(1) {
            Some(back) => s[..pos].char_indices().rev().nth(back).map(|(i, _)| i),
            None => Some(pos),
        };
        start.is_some_and(|start| re.is_match(&s[start..pos]))
    }

    /// The end of the match at `pos`, trying the alternatives in order, as Python's backtracking does.
    fn match_at(&self, s: &str, pos: usize) -> Option<usize> {
        for rule in &self.rules {
            if !Self::behind_ok(rule, s, pos) {
                continue;
            }
            for variant in &rule.variants {
                let Some(m) = variant.find(&s[pos..]) else {
                    continue;
                };
                let end = pos + m.end();
                match &rule.ahead {
                    Some((ahead, negate)) if ahead.is_match(&s[end..]) == *negate => continue,
                    _ => return Some(end),
                }
            }
        }
        None
    }

    /// `find_prefix`: the length of the match at the start, or 0.
    pub fn prefix_len(&self, s: &str) -> usize {
        if !self.filter.is_match(s) {
            return 0;
        }
        self.match_at(s, 0).unwrap_or(0)
    }

    /// `suffix_search`: the leftmost match that ends at the end (or, as Python's `$` allows,
    /// before a final newline).
    pub fn suffix(&self, s: &str) -> Option<(usize, usize)> {
        // No match starts before the filter's. Only lookaround-free rules can match more than
        // `max_width` characters, and those match wherever the filter does, so when the filter's
        // match starts further from the end, the loop stops at once; otherwise it tries at most
        // `max_width` positions.
        let first = self.filter.find(s)?;
        for (pos, _) in s[first.start()..].char_indices() {
            let pos = first.start() + pos;
            if let Some(end) = self.full_match_at(s, pos) {
                return Some((pos, end));
            }
        }
        None
    }

    /// Where a rule matching at `pos` reaches `$`. The bodies never match whitespace (`export.py`
    /// checks), so at most one of the two ends can work.
    fn full_match_at(&self, s: &str, pos: usize) -> Option<usize> {
        let ends = std::iter::once(s.len()).chain(s.strip_suffix('\n').map(str::len));
        ends.filter(|&end| end >= pos).find(|&end| {
            self.rules.iter().any(|r| Self::behind_ok(r, s, pos) && r.full.is_match(&s[pos..end]))
        })
    }

    /// `infix_finditer`: the non-overlapping matches, left to right, as byte ranges.
    pub fn find_all(&self, s: &str) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();
        let mut pos = 0;
        while let Some(candidate) = self.filter.find_at(s, pos) {
            let start = candidate.start();
            match self.match_at(s, start) {
                Some(end) if end > start => {
                    matches.push((start, end));
                    pos = end;
                }
                _ => pos = start + s[start..].chars().next().map_or(1, char::len_utf8),
            }
        }
        matches
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct UrlData {
    filter: String,
    scheme: String,
    excluded: Vec<String>,
    ip: String,
    ip_max_width: usize,
    label_edge: CharSet,
    label_inner: CharSet,
    label_inner_max: usize,
    tld: CharSet,
    tld_min: usize,
    tld_max: usize,
    port_digit: CharSet,
    port_min: usize,
    port_max: usize,
    path_start: CharSet,
}

/// spaCy's `URL_PATTERN` as a `match`: does the whole string match
/// `scheme? credentials? (ip | domain) port? path? $`? Each part is checked at every position in
/// one pass from the end, so this is linear in the string's length.
///
/// - scheme: `[\w+\-.]{2,}://`, only at the start.
/// - credentials: `\S+(?::\S*)?@`, which is any non-space text followed by `@`.
/// - ip: dotted quad, unless it starts with a private or local network (`excluded`, the pattern's
///   negative lookaheads, which only guard this branch).
/// - domain: one or more labels `edge (inner{0,62} edge)? .`, then a TLD of 2–63 `tld` characters.
/// - port: `:` and 2–5 digits; path: one of `/?#` and any non-space text.
pub(crate) struct UrlMatcher {
    filter: Regex,
    scheme: Regex,
    excluded: Vec<Regex>,
    ip: Regex,
    /// The dotted quad at the start, as a quick test.
    ip_start: Regex,
    data: UrlData,
}

impl UrlMatcher {
    pub fn new(data: UrlData) -> Self {
        UrlMatcher {
            filter: compile(&data.filter),
            scheme: compile(&format!(r"\A(?:{})", data.scheme)),
            excluded: data.excluded.iter().map(|p| compile(&format!(r"\A(?:{p})"))).collect(),
            ip: compile(&format!(r"\A(?:{})\z", data.ip)),
            ip_start: compile(&format!(r"\A(?:{})", data.ip)),
            data,
        }
    }

    pub fn is_match(&self, s: &str, space: &CharSet) -> bool {
        // The filter is the pattern without its lookaheads: a cheap test that rejects most words.
        if !self.filter.is_match(s) {
            return false;
        }
        let d = &self.data;
        let chars: Vec<(usize, char)> = s.char_indices().collect();
        let n = chars.len();
        let byte = |i: usize| chars.get(i).map_or(s.len(), |&(b, _)| b);
        // Python's `$` matches at the end, and before a final newline.
        let ends: &[usize] = if chars.last().is_some_and(|&(_, c)| c == '\n') { &[n, n - 1] } else { &[n] };
        // Runs of characters in a class, starting at each position.
        let run = |set: &dyn Fn(char) -> bool| {
            let mut r = vec![0usize; n + 1];
            for i in (0..n).rev() {
                r[i] = if set(chars[i].1) { r[i + 1] + 1 } else { 0 };
            }
            r
        };
        let not_space = run(&|c| !space.contains(c));
        let digits = run(&|c| d.port_digit.contains(c));
        let inner = run(&|c| d.label_inner.contains(c));
        let tld = run(&|c| d.tld.contains(c));
        let is_edge = |i: usize| i < n && d.label_edge.contains(chars[i].1);

        let path = |i: usize| {
            ends.iter().any(|&e| {
                i == e || (i < e && d.path_start.contains(chars[i].1) && not_space[i + 1] >= e - (i + 1))
            })
        };
        // port? path? $ from each position.
        let tail: Vec<bool> = (0..=n)
            .map(|i| {
                path(i)
                    || (i < n
                        && chars[i].1 == ':'
                        && (d.port_min..=d.port_max).any(|k| digits[i + 1] >= k && path(i + 1 + k)))
            })
            .collect();
        // (label .)+ tld tail, from each position.
        let mut domain = vec![false; n + 2];
        for i in (0..n).rev() {
            if !is_edge(i) {
                continue;
            }
            let label_ends = std::iter::once(i + 1)
                .chain((0..=d.label_inner_max.min(inner[i + 1])).map(|k| i + 2 + k).filter(|&e| is_edge(e - 1)));
            domain[i] = label_ends.into_iter().any(|e| {
                e < n && chars[e].1 == '.' && {
                    let next = e + 1;
                    domain[next]
                        || (d.tld_min..=d.tld_max.min(tld[next])).any(|t| tail.get(next + t).copied().unwrap_or(false))
                }
            });
        }
        let ip = |i: usize| {
            self.ip_start.is_match(&s[byte(i)..])
                && !self.excluded.iter().any(|re| re.is_match(&s[byte(i)..]))
                && (i + 1..=(i + d.ip_max_width).min(n)).any(|e| tail[e] && self.ip.is_match(&s[byte(i)..byte(e)]))
        };
        let host = |i: usize| i < n && (domain[i] || ip(i));

        // Where the host can start: at the start, after the scheme, or after "@" that follows at
        // least one non-space character (after the scheme, if there is one).
        let scheme_end = self.scheme.find(s).map(|m| chars.partition_point(|&(b, _)| b < m.end()));
        let mut spaces_before = vec![0usize; n + 1];
        for i in 0..n {
            spaces_before[i + 1] = spaces_before[i] + usize::from(space.contains(chars[i].1));
        }
        let credentials_from = |base: usize, at: usize| at > base && spaces_before[at] == spaces_before[base];
        let after_credentials = (1..n).filter(|&at| chars[at].1 == '@').filter(|&at| {
            credentials_from(0, at) || scheme_end.is_some_and(|base| credentials_from(base, at))
        });
        std::iter::once(0).chain(scheme_end).chain(after_credentials.map(|at| at + 1)).any(host)
    }
}
