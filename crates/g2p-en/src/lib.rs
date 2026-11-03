//! A port of misaki 0.9.4's English G2P (`misaki/en.py`), with kokoro's handling of its tokens.
//!
//! The caller supplies spaCy's tokenization and tags ([`Tagger`]) and the espeak fallback for unknown
//! words ([`Fallback`]), which live in other crates.
#![forbid(unsafe_code)]

mod align;
pub mod kokoro;
mod lexicon;
pub mod num2words;
mod subtokenize;
mod text;
mod unicode_tables;

use lexicon::{LexToken, Lexicon, TokenContext, currency_units, is_consonant, is_vowel};
use std::fmt;

pub use subtokenize::subtokenize;

/// A token from spaCy: `(token.text, token.tag_, token.whitespace_)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedToken {
    pub text: String,
    pub tag: String,
    pub whitespace: String,
}

/// spaCy's `en_core_web_sm` tokenizer and tagger.
pub trait Tagger {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken>;
}

/// Phonemes for a word the lexicon doesn't know (kokoro uses espeak-ng).
pub trait Fallback {
    fn phonemize(&mut self, word: &str) -> Option<String>;
}

/// One of misaki's output tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub text: String,
    pub tag: String,
    pub whitespace: String,
    pub phonemes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A number misaki hands to num2words is too large (Python raises `OverflowError`).
    Number(num2words::Error),
    /// The text's words and spaCy's tokens differ by more than whitespace (spaCy raises `ValueError`).
    Alignment,
    /// The tagger returned a token with no text (spaCy never does).
    EmptyToken,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Number(e) => e.fmt(f),
            Error::Alignment => f.write_str("text and tokens don't align"),
            Error::EmptyToken => f.write_str("the tagger returned an empty token"),
        }
    }
}

impl std::error::Error for Error {}

impl From<num2words::Error> for Error {
    fn from(e: num2words::Error) -> Self {
        Error::Number(e)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Feature {
    Stress(f64),
    /// `/phonemes/`, kept with its leading slash as misaki does.
    Phonemes(String),
    /// `#flags#`. misaki only ever asks whether the flags contain `&`, `a` or `n`, so only those are kept.
    NumFlags(String),
}

/// `G2P.preprocess`: the text with `[text](feature)` links replaced by their text, its whitespace-separated
/// words, and each link's feature keyed by the index of its word.
#[derive(Debug, Clone)]
pub struct Preprocessed {
    pub text: String,
    words: Vec<String>,
    features: Vec<(usize, Feature)>,
}

fn link_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\[([^\]]+)\]\(([^\)]*)\)").expect("valid regex"))
}

fn python_split(s: &str) -> impl Iterator<Item = &str> {
    s.split(text::is_space).filter(|w| !w.is_empty())
}

fn parse_feature(f: &str) -> Option<Feature> {
    let unsigned = f.strip_prefix(['-', '+']).unwrap_or(f);
    if text::is_ascii_digits(unsigned) {
        let value: f64 = f.trim_end_matches('\n').parse().expect("signed digits");
        return Some(Feature::Stress(value));
    }
    match f {
        "0.5" | "+0.5" => return Some(Feature::Stress(0.5)),
        "-0.5" => return Some(Feature::Stress(-0.5)),
        _ => {}
    }
    let n = f.chars().count();
    if n > 1 && f.starts_with('/') && f.ends_with('/') {
        return Some(Feature::Phonemes(format!("/{}", f[1..].trim_end_matches('/'))));
    }
    if n > 1 && f.starts_with('#') && f.ends_with('#') {
        return Some(Feature::NumFlags(['&', 'a', 'n'].into_iter().filter(|&c| f.contains(c)).collect()));
    }
    None
}

impl Preprocessed {
    pub fn new(text: &str) -> Self {
        let text = text::lstrip(text);
        let mut out = String::with_capacity(text.len());
        let mut words = Vec::new();
        let mut features = Vec::new();
        let mut last_end = 0;
        for m in link_regex().captures_iter(text) {
            let whole = m.get(0).expect("match");
            let before = &text[last_end..whole.start()];
            out.push_str(before);
            words.extend(python_split(before).map(str::to_string));
            if let Some(f) = parse_feature(&m[2]) {
                features.push((words.len(), f));
            }
            out.push_str(&m[1]);
            words.push(m[1].to_string());
            last_end = whole.end();
        }
        if last_end < text.len() {
            out.push_str(&text[last_end..]);
            words.extend(python_split(&text[last_end..]).map(str::to_string));
        }
        Preprocessed { text: out, words, features }
    }
}

#[derive(Debug, Clone)]
struct MToken {
    text: String,
    tag: String,
    whitespace: String,
    phonemes: Option<String>,
    is_head: bool,
    alias: Option<String>,
    stress: Option<f64>,
    currency: Option<char>,
    num_flags: String,
    prespace: bool,
}

impl MToken {
    fn lex(&self) -> LexToken<'_> {
        LexToken {
            text: self.alias.as_deref().unwrap_or(&self.text),
            tag: &self.tag,
            stress: self.stress,
            currency: self.currency,
            is_head: self.is_head,
            num_flags: &self.num_flags,
        }
    }

    fn into_token(self) -> Token {
        Token { text: self.text, tag: self.tag, whitespace: self.whitespace, phonemes: self.phonemes }
    }
}

fn tag_weight(text: &str) -> usize {
    text.chars().map(|c| if text::char_is_lower(c) { 1 } else { 2 }).sum()
}

/// `merge_tokens`; phonemes are joined only when `unk` is given.
fn merge_tokens(tokens: &[MToken], unk: Option<&str>) -> MToken {
    let first = &tokens[0];
    let last = tokens.last().expect("non-empty");
    let mut stresses: Vec<f64> = Vec::new();
    for s in tokens.iter().filter_map(|t| t.stress) {
        if !stresses.contains(&s) {
            stresses.push(s);
        }
    }
    let phonemes = unk.map(|unk| {
        let mut ps = String::new();
        for tk in tokens {
            if tk.prespace
                && ps.chars().next_back().is_some_and(|c| !text::is_space(c))
                && tk.phonemes.as_deref().is_some_and(|p| !p.is_empty())
            {
                ps.push(' ');
            }
            ps.push_str(tk.phonemes.as_deref().unwrap_or(unk));
        }
        ps
    });
    let mut text = String::new();
    for tk in &tokens[..tokens.len() - 1] {
        text.push_str(&tk.text);
        text.push_str(&tk.whitespace);
    }
    text.push_str(&last.text);
    let mut tag_token = first;
    let mut best = tag_weight(&first.text);
    for tk in &tokens[1..] {
        let w = tag_weight(&tk.text);
        if w > best {
            best = w;
            tag_token = tk;
        }
    }
    let mut flags: Vec<char> = tokens.iter().flat_map(|t| t.num_flags.chars()).collect();
    flags.sort_unstable();
    flags.dedup();
    MToken {
        text,
        tag: tag_token.tag.clone(),
        whitespace: last.whitespace.clone(),
        phonemes,
        is_head: first.is_head,
        alias: None,
        stress: if stresses.len() == 1 { Some(stresses[0]) } else { None },
        currency: tokens.iter().filter_map(|t| t.currency).max(),
        num_flags: flags.into_iter().collect(),
        prespace: first.prespace,
    }
}

fn is_subtoken_junk(c: char) -> bool {
    "',-._‘’/".contains(c)
}

fn is_punct(c: char) -> bool {
    ";:,.!?—…\"“”".contains(c)
}

fn is_non_quote_punct(c: char) -> bool {
    ";:,.!?—…".contains(c)
}

fn is_punct_tag(tag: &str) -> bool {
    matches!(tag, "." | "," | "-LRB-" | "-RRB-" | "``" | "\"\"" | "''" | ":" | "$" | "#" | "NFP")
}

fn punct_tag_phonemes(tag: &str) -> Option<&'static str> {
    match tag {
        "-LRB-" => Some("("),
        "-RRB-" => Some(")"),
        "``" => Some("\u{201c}"),
        "\"\"" | "''" => Some("\u{201d}"),
        _ => None,
    }
}

/// `all(97 <= ord(c.lower()) <= 122 for c in text)`
fn all_ascii_letters(text: &str) -> bool {
    text.chars().all(|c| {
        let mut l = c.to_lowercase();
        matches!((l.next(), l.next()), (Some(l), None) if l.is_ascii_lowercase())
    })
}

enum Word {
    One(MToken),
    Many(Vec<MToken>),
}

fn token_context(ctx: &TokenContext, ps: Option<&str>, token: &MToken) -> TokenContext {
    let mut future_vowel = ctx.future_vowel;
    if let Some(ps) = ps.filter(|p| !p.is_empty())
        && let Some(c) = ps.chars().find(|&c| is_vowel(c) || is_consonant(c) || is_non_quote_punct(c))
    {
        future_vowel = if is_non_quote_punct(c) { None } else { Some(is_vowel(c)) };
    }
    let t = token.text.as_str();
    let future_to = t == "to" || t == "To" || (t == "TO" && (token.tag == "TO" || token.tag == "IN"));
    TokenContext { future_vowel, future_to }
}

fn stress_weight(ps: &str) -> usize {
    ps.chars().map(|c| if "AIOQWYʤʧ".contains(c) { 2 } else { 1 }).sum()
}

fn resolve_tokens(tokens: &mut [MToken]) {
    let mut text = String::new();
    for (i, tk) in tokens.iter().enumerate() {
        text.push_str(&tk.text);
        if i + 1 < tokens.len() {
            text.push_str(&tk.whitespace);
        }
    }
    let mut kinds = [false; 3];
    for c in text.chars().filter(|&c| !is_subtoken_junk(c)) {
        kinds[if text::is_alpha(c) {
            0
        } else if c.is_ascii_digit() {
            1
        } else {
            2
        }] = true;
    }
    let prespace = text.contains(' ') || text.contains('/') || kinds.iter().filter(|&&k| k).count() > 1;
    let n = tokens.len();
    for (i, tk) in tokens.iter_mut().enumerate() {
        if tk.phonemes.is_none() {
            if i == n - 1 && tk.text.chars().count() == 1 && tk.text.chars().all(is_non_quote_punct) {
                tk.phonemes = Some(tk.text.clone());
            } else if tk.text.chars().all(is_subtoken_junk) {
                tk.phonemes = Some(String::new());
            }
        } else if i > 0 {
            tk.prespace = prespace;
        }
    }
    if prespace {
        return;
    }
    let mut indices: Vec<(bool, usize, usize)> = tokens
        .iter()
        .enumerate()
        .filter_map(|(i, tk)| {
            let ps = tk.phonemes.as_deref().filter(|p| !p.is_empty())?;
            Some((ps.contains(lexicon::PRIMARY_STRESS), stress_weight(ps), i))
        })
        .collect();
    let demote = |tk: &mut MToken| {
        let ps = tk.phonemes.as_deref().expect("has phonemes");
        tk.phonemes = Some(lexicon::apply_stress(ps, Some(-0.5)));
    };
    if indices.len() == 2 && tokens[indices[0].2].text.chars().count() == 1 {
        demote(&mut tokens[indices[1].2]);
        return;
    }
    let primaries = indices.iter().filter(|(p, _, _)| *p).count();
    if indices.len() < 2 || primaries <= indices.len().div_ceil(2) {
        return;
    }
    indices.sort();
    let half = indices.len() / 2;
    for &(_, _, i) in &indices[..half] {
        demote(&mut tokens[i]);
    }
}

/// The most subtokens of a run without whitespace looked up together. misaki tries every span of the
/// run, which is cubic in its length; no lexicon key has more than 13 subtokens
/// ("what-you-see-is-what-you-get"), so only dotted abbreviations and numbers split by repeated
/// separators longer than this read differently (docs/divergences.md).
const MAX_SPAN: usize = 16;

/// misaki's English `G2P` as kokoro configures it (`version=None`, so `ɾ` and `ʔ` become `T` and `t`).
pub struct G2p {
    lexicon: Lexicon,
    unk: String,
}

impl G2p {
    /// American (`british = false`, kokoro's `a`) or British (`b`) English, with kokoro's `unk=''`.
    pub fn new(british: bool) -> Self {
        G2p { lexicon: Lexicon::new(british), unk: String::new() }
    }

    /// Replaces the text of words without phonemes in the joined output (misaki's default is `❓`).
    pub fn with_unk(mut self, unk: impl Into<String>) -> Self {
        self.unk = unk.into();
        self
    }

    /// `G2P.__call__`: the phoneme string and the tokens.
    pub fn phonemize(
        &self,
        text: &str,
        tagger: &mut dyn Tagger,
        fallback: Option<&mut dyn Fallback>,
    ) -> Result<(String, Vec<Token>), Error> {
        let pre = Preprocessed::new(text);
        let tagged = tagger.tag(&pre.text);
        self.phonemize_tagged(&pre, tagged, fallback)
    }

    /// [`G2p::phonemize`] given spaCy's tokens for `pre.text`.
    pub fn phonemize_tagged(
        &self,
        pre: &Preprocessed,
        tagged: Vec<TaggedToken>,
        mut fallback: Option<&mut dyn Fallback>,
    ) -> Result<(String, Vec<Token>), Error> {
        let tokens = self.tokenize(pre, tagged)?;
        let tokens = self.fold_left(tokens);
        let mut words = retokenize(tokens)?;
        let mut ctx = TokenContext::default();
        for word in words.iter_mut().rev() {
            match word {
                Word::One(w) => {
                    if w.phonemes.is_none() {
                        w.phonemes = self.lexicon.get(&w.lex(), &ctx)?;
                    }
                    if w.phonemes.is_none()
                        && let Some(fb) = fallback.as_deref_mut()
                    {
                        w.phonemes = fb.phonemize(&w.text);
                    }
                    ctx = token_context(&ctx, w.phonemes.as_deref(), w);
                }
                Word::Many(w) => self.resolve_group(w, &mut ctx, fallback.as_deref_mut())?,
            }
        }
        let mut out = String::new();
        let tokens: Vec<Token> = words
            .into_iter()
            .map(|w| {
                let mut tk = match w {
                    Word::One(tk) => tk,
                    Word::Many(tks) => merge_tokens(&tks, Some(&self.unk)),
                };
                if let Some(ps) = tk.phonemes.as_mut().filter(|p| p.contains(['ɾ', 'ʔ'])) {
                    *ps = ps.replace('ɾ', "T").replace('ʔ', "t");
                }
                out.push_str(tk.phonemes.as_deref().unwrap_or(&self.unk));
                out.push_str(&tk.whitespace);
                tk.into_token()
            })
            .collect();
        Ok((out, tokens))
    }

    fn resolve_group<'f>(
        &self,
        w: &mut [MToken],
        ctx: &mut TokenContext,
        fallback: Option<&mut (dyn Fallback + 'f)>,
    ) -> Result<(), Error> {
        let first_left = |right: usize| right.saturating_sub(MAX_SPAN);
        let mut right = w.len();
        let mut left = first_left(right);
        let mut should_fallback = false;
        while left < right {
            let span = &w[left..right];
            let found = if span.iter().any(|t| t.alias.is_some() || t.phonemes.is_some()) {
                None
            } else {
                let tk = merge_tokens(span, None);
                self.lexicon.get(&tk.lex(), ctx)?.map(|ps| (ps, tk))
            };
            if let Some((ps, tk)) = found {
                *ctx = token_context(ctx, Some(&ps), &tk);
                w[left].phonemes = Some(ps);
                for x in &mut w[left + 1..right] {
                    x.phonemes = Some(String::new());
                }
                right = left;
                left = first_left(right);
            } else if left + 1 < right {
                left += 1;
            } else {
                right -= 1;
                let tk = &mut w[right];
                if tk.phonemes.is_none() {
                    if tk.text.chars().all(is_subtoken_junk) {
                        tk.phonemes = Some(String::new());
                    } else if fallback.is_some() {
                        should_fallback = true;
                        break;
                    }
                }
                left = first_left(right);
            }
        }
        match fallback.filter(|_| should_fallback) {
            Some(fb) => {
                let tk = merge_tokens(w, None);
                w[0].phonemes = fb.phonemize(&tk.text);
                for x in &mut w[1..] {
                    x.phonemes = Some(String::new());
                }
            }
            None => resolve_tokens(w),
        }
        Ok(())
    }

    fn tokenize(&self, pre: &Preprocessed, tagged: Vec<TaggedToken>) -> Result<Vec<MToken>, Error> {
        let mut tokens: Vec<MToken> = tagged
            .into_iter()
            .map(|t| MToken {
                text: t.text,
                tag: t.tag,
                whitespace: t.whitespace,
                phonemes: None,
                is_head: true,
                alias: None,
                stress: None,
                currency: None,
                num_flags: String::new(),
                prespace: false,
            })
            .collect();
        if pre.features.is_empty() {
            return Ok(tokens);
        }
        let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
        let y2x = align::y2x_data(&pre.words, &texts).ok_or(Error::Alignment)?;
        let mut positions: Vec<Vec<usize>> = vec![Vec::new(); pre.words.len()];
        for (j, &x) in y2x.iter().enumerate() {
            positions[x].push(j);
        }
        for (k, v) in &pre.features {
            for (i, &j) in positions.get(*k).into_iter().flatten().enumerate() {
                let Some(tk) = tokens.get_mut(j) else { continue };
                match v {
                    Feature::Stress(s) => tk.stress = Some(*s),
                    Feature::Phonemes(p) => {
                        tk.is_head = i == 0;
                        tk.phonemes = Some(if i == 0 { p.trim_start_matches('/').to_string() } else { String::new() });
                    }
                    Feature::NumFlags(f) => tk.num_flags.clone_from(f),
                }
            }
        }
        Ok(tokens)
    }

    /// `fold_left`: each non-head token is merged into the token before it. Merging one pair at a time
    /// re-copies the growing prefix, so this extends it in place with the same result as
    /// `merge_tokens([prev, tk], unk)`.
    fn fold_left(&self, tokens: Vec<MToken>) -> Vec<MToken> {
        let mut result: Vec<MToken> = Vec::with_capacity(tokens.len());
        let mut weight = 0;
        let mut flags: Vec<char> = Vec::new();
        for tk in tokens {
            let Some(acc) = result.last_mut().filter(|_| !tk.is_head) else {
                weight = tag_weight(&tk.text);
                flags = tk.num_flags.chars().collect();
                flags.sort_unstable();
                flags.dedup();
                result.push(tk);
                continue;
            };
            let tk_weight = tag_weight(&tk.text);
            if tk_weight > weight {
                acc.tag = tk.tag;
            }
            weight += tag_weight(&acc.whitespace) + tk_weight;
            acc.text.push_str(&acc.whitespace);
            acc.text.push_str(&tk.text);
            acc.whitespace = tk.whitespace;
            let mut ps = acc.phonemes.take().unwrap_or_else(|| self.unk.clone());
            if tk.prespace
                && ps.chars().next_back().is_some_and(|c| !text::is_space(c))
                && tk.phonemes.as_deref().is_some_and(|p| !p.is_empty())
            {
                ps.push(' ');
            }
            ps.push_str(tk.phonemes.as_deref().unwrap_or(&self.unk));
            acc.phonemes = Some(ps);
            acc.stress = match (acc.stress, tk.stress) {
                (Some(a), Some(b)) if a != b => None,
                (a, b) => a.or(b),
            };
            acc.currency = acc.currency.max(tk.currency);
            acc.alias = None;
            if !tk.num_flags.is_empty() {
                flags.extend(tk.num_flags.chars());
                flags.sort_unstable();
                flags.dedup();
            }
            acc.num_flags = flags.iter().collect();
        }
        result
    }
}

fn retokenize(tokens: Vec<MToken>) -> Result<Vec<Word>, Error> {
    let mut words: Vec<Word> = Vec::with_capacity(tokens.len());
    let mut currency: Option<char> = None;
    let next_tags: Vec<bool> = tokens.iter().map(|t| t.tag == "CD").collect();
    for (i, token) in tokens.into_iter().enumerate() {
        let next_is_cd = next_tags.get(i + 1).copied().unwrap_or(false);
        let is_last = i + 1 == next_tags.len();
        let mut tks: Vec<MToken> = if token.alias.is_none() && token.phonemes.is_none() {
            subtokenize(&token.text)
                .into_iter()
                .map(|t| MToken {
                    text: t.to_string(),
                    tag: token.tag.clone(),
                    whitespace: String::new(),
                    phonemes: None,
                    is_head: true,
                    alias: None,
                    stress: token.stress,
                    currency: None,
                    num_flags: token.num_flags.clone(),
                    prespace: false,
                })
                .collect()
        } else {
            vec![token.clone()]
        };
        tks.last_mut().ok_or(Error::EmptyToken)?.whitespace = token.whitespace;
        let n = tks.len();
        for j in 0..n {
            let to_alias = {
                let tk = &tks[j];
                tk.alias.is_none() && tk.phonemes.is_none() && 0 < j && j + 1 < n && tk.text == "2" && {
                    let mut s = String::new();
                    s.extend(tks[j - 1].text.chars().next_back());
                    s.extend(tks[j + 1].text.chars().next());
                    text::str_is_alpha(&s)
                }
            };
            let tk = &mut tks[j];
            if tk.alias.is_some() || tk.phonemes.is_some() {
            } else if tk.tag == "$"
                && tk.text.chars().count() == 1
                && currency_units(tk.text.chars().next().expect("one char")).is_some()
            {
                currency = tk.text.chars().next();
                tk.phonemes = Some(String::new());
            } else if tk.tag == ":" && (tk.text == "-" || tk.text == "–") {
                tk.phonemes = Some("—".to_string());
            } else if is_punct_tag(&tk.tag) && !all_ascii_letters(&tk.text) {
                tk.phonemes = Some(match punct_tag_phonemes(&tk.tag) {
                    Some(p) => p.to_string(),
                    None => tk.text.chars().filter(|&c| is_punct(c)).collect(),
                });
            } else if let Some(symbol) = currency {
                if tk.tag != "CD" {
                    currency = None;
                } else if j + 1 == n && (is_last || !next_is_cd) {
                    tk.currency = Some(symbol);
                }
            } else if to_alias {
                tk.alias = Some("to".to_string());
            }
        }
        for mut tk in tks {
            if tk.alias.is_some() || tk.phonemes.is_some() {
                words.push(Word::One(tk));
            } else if let Some(Word::Many(group)) = words
                .last_mut()
                .filter(|w| matches!(w, Word::Many(g) if g.last().is_some_and(|t| t.whitespace.is_empty())))
            {
                tk.is_head = false;
                group.push(tk);
            } else if tk.whitespace.is_empty() {
                words.push(Word::Many(vec![tk]));
            } else {
                words.push(Word::One(tk));
            }
        }
    }
    Ok(words
        .into_iter()
        .map(|w| match w {
            Word::Many(mut g) if g.len() == 1 => Word::One(g.pop().expect("one")),
            w => w,
        })
        .collect())
}
