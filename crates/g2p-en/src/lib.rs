//! A port of misaki 0.9.4's English G2P (`misaki/en.py`), with kokoro's handling of its tokens.
//!
//! The caller supplies spaCy's tokenization and tags ([`Tagger`]) and the espeak fallback for unknown
//! words ([`Fallback`]), which live in other crates.
#![forbid(unsafe_code)]

mod align;
pub mod kokoro;
mod lexicon;
mod names;
mod subtokenize;
mod text;
mod unicode_tables;

use lexicon::{LexToken, Lexicon, TokenContext, is_consonant, is_vowel};
use std::borrow::Cow;
use std::fmt;

pub use subtokenize::subtokenize;
/// Python's `str.isalpha` for one character.
pub use text::is_alpha;

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
    /// The text's words and spaCy's tokens differ by more than whitespace (spaCy raises `ValueError`).
    Alignment,
    /// The tagger returned a token with no text (spaCy never does).
    EmptyToken,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Alignment => f.write_str("text and tokens don't align"),
            Error::EmptyToken => f.write_str("the tagger returned an empty token"),
        }
    }
}

impl std::error::Error for Error {}

/// `G2P.preprocess`: the text with `[text](feature)` links replaced by their text, its whitespace-separated
/// words, and each phoneme link's phonemes keyed by the index of its word. misaki's stress links
/// (`[word](+1)`) aren't read: the text normalizer reads their numbers as words before G2P, so
/// they never worked through the server (docs/divergences.md).
#[derive(Debug, Clone)]
pub struct Preprocessed {
    pub text: String,
    words: Vec<String>,
    features: Vec<(usize, String)>,
}

fn link_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\[([^\]]+)\]\(([^\)]*)\)").expect("valid regex"))
}

fn python_split(s: &str) -> impl Iterator<Item = &str> {
    s.split(text::is_space).filter(|w| !w.is_empty())
}

/// A link's phonemes, for `/phonemes/` (without IPA's syllable breaks: a link is one word's
/// pronunciation, and a full stop can go after it).
pub fn link_phonemes(feature: &str) -> Option<String> {
    (feature.chars().count() > 1 && feature.starts_with('/') && feature.ends_with('/'))
        .then(|| feature[1..].trim_end_matches('/').chars().filter(|&c| c != '.').collect())
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
            if let Some(f) = link_phonemes(&m[2]) {
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
    prespace: bool,
}

impl MToken {
    fn lex(&self) -> LexToken<'_> {
        LexToken { text: &self.text, tag: &self.tag }
    }

    fn into_token(self) -> Token {
        Token { text: self.text, tag: self.tag, whitespace: self.whitespace, phonemes: self.phonemes }
    }
}

fn tag_weight(text: &str) -> usize {
    text.chars().map(|c| if text::char_is_lower(c) { 1 } else { 2 }).sum()
}

/// Appends `tk`'s phonemes (kokoro's `unk=''` if it has none), after a space if it is `prespace`.
fn push_phonemes(ps: &mut String, tk: &MToken) {
    if tk.prespace
        && ps.chars().next_back().is_some_and(|c| !text::is_space(c))
        && tk.phonemes.as_deref().is_some_and(|p| !p.is_empty())
    {
        ps.push(' ');
    }
    ps.push_str(tk.phonemes.as_deref().unwrap_or_default());
}

/// `merge_tokens`, with `unk=''` if `join_phonemes` and `unk=None` otherwise.
fn merge_tokens(tokens: &[MToken], join_phonemes: bool) -> MToken {
    let first = &tokens[0];
    let last = tokens.last().expect("non-empty");
    let phonemes = join_phonemes.then(|| {
        let mut ps = String::new();
        for tk in tokens {
            push_phonemes(&mut ps, tk);
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
    MToken {
        text,
        tag: tag_token.tag.clone(),
        whitespace: last.whitespace.clone(),
        phonemes,
        is_head: first.is_head,
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
    ps.chars().filter(|c| !matches!(c, '\u{361}' | '\u{32c}')).count()
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

fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\'' || c == '’' || unicode_normalization::char::is_combining_mark(c)
}

/// `text` in words and what is between them, in order. A hyphen or dot between letters joins them,
/// so a compound or a spelled-out acronym is one word ("WELL-KNOWN", "S-H-A-D-E").
fn words(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let word = is_word_char(first);
        let mut end = first.len_utf8();
        let mut chars = rest[end..].chars().peekable();
        while let Some(c) = chars.next() {
            let joins = word && "-.".contains(c) && chars.peek().is_some_and(|&n| n.is_alphabetic());
            if is_word_char(c) != word && !joins {
                break;
            }
            end += c.len_utf8();
        }
        let (piece, after) = rest.split_at(end);
        rest = after;
        Some((piece, word))
    })
}

fn one_char(mut chars: impl Iterator<Item = char>) -> Option<char> {
    let c = chars.next()?;
    chars.next().is_none().then_some(c)
}

/// The sentences of `text`: each ends at a line break, or at whitespace after `.`, `!`, `?` or `…`
/// and any closing quotes or brackets.
fn sentences(text: &str) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
    let mut start = 0;
    let mut ended = false;
    let mut chars = text.char_indices();
    std::iter::from_fn(move || {
        for (i, c) in chars.by_ref() {
            if c == '\n' || ended && text::is_space(c) {
                let end = i + c.len_utf8();
                ended = false;
                return Some(std::mem::replace(&mut start, end)..end);
            }
            if ".!?…".contains(c) {
                ended = true;
            } else if !"\"'”’)]»".contains(c) {
                ended = false;
            }
        }
        (start < text.len()).then(|| std::mem::replace(&mut start, text.len())..text.len())
    })
}

/// Puts back the case of `original` that [`G2p::sentence_case`] changed in `cased`.
fn restore_case(tokens: &mut [Token], cased: &str, original: &str) -> Result<(), Error> {
    // The tokens may leave characters out (misaki's `subtokenize` drops a lone "'" in "X'1").
    let mut pairs = cased.chars().zip(original.chars());
    let mut take = |c: char| pairs.find(|&(x, _)| x == c).map(|(_, o)| o).ok_or(Error::Alignment);
    for token in tokens {
        token.text = token.text.chars().map(&mut take).collect::<Result<_, _>>()?;
        token.whitespace.chars().try_for_each(|c| take(c).map(drop))?;
    }
    Ok(())
}

/// The most subtokens of a run without whitespace looked up together. misaki tries every span of the
/// run, which is cubic in its length; no lexicon key has more than 13 subtokens
/// ("what-you-see-is-what-you-get"), so only dotted abbreviations and numbers split by repeated
/// separators longer than this read differently (docs/divergences.md).
const MAX_SPAN: usize = 16;

/// misaki's English `G2P` as kokoro configures it. With `version=None` kokoro reads misaki's `ɾ` and `ʔ`
/// as the English flap and `t`; the dictionaries here already say `t̬` and `t`, and a link's IPA is
/// read as written.
pub struct G2p {
    lexicon: Lexicon,
    names: names::Names,
}

impl G2p {
    /// American (`british = false`, kokoro's `a`) or British (`b`) English, with kokoro's `unk=''`.
    pub fn new(british: bool) -> Self {
        G2p { lexicon: Lexicon::new(british), names: names::Names::new() }
    }

    /// `G2P.__call__`: the phoneme string and the tokens. Sentences in capitals are read in
    /// [`G2p::sentence_case`], and the tokens keep the text as written.
    pub fn phonemize(
        &self,
        text: &str,
        tagger: &mut dyn Tagger,
        fallback: Option<&mut dyn Fallback>,
    ) -> Result<(String, Vec<Token>), Error> {
        let pre = Preprocessed::new(text);
        let cased = self.sentence_case(&pre.text);
        let tagged = tagger.tag(&cased);
        let (ps, mut tokens) = self.phonemize_tagged(&pre, tagged, fallback)?;
        if let Cow::Owned(cased) = cased {
            restore_case(&mut tokens, &cased, &pre.text)?;
        }
        Ok((ps, tokens))
    }

    /// `text` with each sentence written all in capitals cased the way English usually is:
    /// lowercase but for its first letter, "I", and the short words in ASCII capitals that no
    /// dictionary has, which are acronyms. spaCy's tagger and misaki's lexicon both read case as a clue (a
    /// capital starts a sentence or a name, and stresses a word), which capitals everywhere don't
    /// give. It takes two words of two capitals or more, in a run of sentences without lowercase:
    /// one alone, like "OK" or "NASA.", is an acronym as much as a sentence. Each character keeps its place,
    /// so tokens map back to the text.
    pub fn sentence_case<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let mut previous = false;
        let two_capitals = text.chars().any(|c| std::mem::replace(&mut previous, c.is_uppercase()) && previous);
        if !two_capitals {
            return Cow::Borrowed(text);
        }
        let mut runs: Vec<(Vec<std::ops::Range<usize>>, usize)> = vec![(Vec::new(), 0)];
        for range in sentences(text) {
            let sentence = &text[range.clone()];
            if sentence.chars().any(char::is_lowercase) {
                runs.push((Vec::new(), 0));
                continue;
            }
            let (run, capitals) = runs.last_mut().expect("never empty");
            *capitals += words(sentence)
                .filter(|&(w, word)| word && w.chars().filter(|c| c.is_uppercase()).nth(1).is_some())
                .count();
            run.push(range);
        }
        let mut out = String::new();
        let mut copied = 0;
        for range in runs.into_iter().filter(|(_, capitals)| *capitals >= 2).flat_map(|(run, _)| run) {
            out.push_str(&text[copied..range.start]);
            self.recase(&text[range.clone()], &mut out);
            copied = range.end;
        }
        if copied == 0 {
            return Cow::Borrowed(text);
        }
        out.push_str(&text[copied..]);
        Cow::Owned(out)
    }

    /// Whether `text` reads differently from misaki for its case (docs/divergences.md, "misaki
    /// English: case"): a sentence in capitals, a word in capitals misaki stressed more, or a word
    /// only a spelling in another case reads. For the parity tests.
    #[doc(hidden)]
    pub fn differs_from_misaki_by_case(&self, text: &str) -> bool {
        self.sentence_case(text) != text
            || words(text).any(|(w, word)| {
                word && w
                    .split(['-', '.'])
                    .any(|w| self.lexicon.found_by_case_alone(w) || self.lexicon.shouted_differently(w))
            })
    }

    fn recase(&self, sentence: &str, out: &mut String) {
        let read = |w: &str| self.lexicon.get(&LexToken { text: w, tag: "NN" }, &TokenContext::default()).is_some();
        // A compound's words may each be known ("resting-place"); letters alone are spelled ("S-H-A-D-E").
        let known = |w: &str| read(w) || w.contains('-') && w.split('-').all(|w| w.chars().count() > 1 && read(w));
        let mut first = true;
        for (word, is_word) in words(sentence) {
            if !is_word {
                out.push_str(word);
                continue;
            }
            let mut cased: String = word.chars().map(|c| one_char(c.to_lowercase()).unwrap_or(c)).collect();
            // An acronym is short and ASCII, as the caps pass had it: "ZQX" and "S-H-A-D-E", but
            // "BENNET" and "ÉCOLE" are words the dictionaries don't have.
            let short = cased.split('-').all(|w| w.chars().filter(|c| c.is_alphabetic()).count() < 4);
            if cased.is_ascii() && short && !known(&cased) {
                cased = word.to_owned();
            }
            let i = cased.strip_prefix('i').is_some_and(|r| r.is_empty() || r.starts_with(['\'', '’']));
            if (first || i)
                && let Some((at, c)) = cased.char_indices().find(|&(_, c)| c.is_alphabetic())
                && c.is_lowercase()
                && let Some(u) = one_char(c.to_uppercase())
            {
                cased.replace_range(at..at + c.len_utf8(), u.encode_utf8(&mut [0; 4]));
            }
            first = false;
            out.push_str(&cased);
        }
    }

    /// [`G2p::phonemize`] given spaCy's tokens for `pre.text`.
    pub fn phonemize_tagged(
        &self,
        pre: &Preprocessed,
        tagged: Vec<TaggedToken>,
        mut fallback: Option<&mut dyn Fallback>,
    ) -> Result<(String, Vec<Token>), Error> {
        let tokens = Self::fold_left(self.tokenize(pre, tagged)?);
        let mut words = retokenize(tokens)?;
        let mut ctx = TokenContext::default();
        for word in words.iter_mut().rev() {
            match word {
                Word::One(w) => {
                    if w.phonemes.is_none() {
                        w.phonemes = self.lexicon.get(&w.lex(), &ctx);
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
                let tk = match w {
                    Word::One(tk) => tk,
                    Word::Many(tks) => merge_tokens(&tks, true),
                };
                out.push_str(tk.phonemes.as_deref().unwrap_or_default());
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
            let found = if span.iter().any(|t| t.phonemes.is_some()) {
                None
            } else {
                let tk = merge_tokens(span, false);
                self.lexicon.get(&tk.lex(), ctx).map(|ps| (ps, tk))
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
                let tk = merge_tokens(w, false);
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
                tk.is_head = i == 0;
                tk.phonemes = Some(if i == 0 { v.clone() } else { String::new() });
            }
        }
        Ok(tokens)
    }

    /// `text` with each link that isn't a `/phonemes/` link (`[Sauron](foo)`) written as its text,
    /// as misaki reads it, so the text is split by script and read like any other.
    pub fn plain_links(text: &str) -> std::borrow::Cow<'_, str> {
        link_regex().replace_all(text, |m: &regex::Captures| match link_phonemes(&m[2]) {
            Some(_) => m[0].to_owned(),
            None => m[1].to_owned(),
        })
    }

    /// `text` with each name from `data/names.json` that stands as a word of its own, in any case
    /// and with or without its accents, made a
    /// `[name](/phonemes/)` link, outside the links it has: so every G2P reads names alike. In text
    /// of `language` (espeak's code for the pipeline's own language, `en` for English) the names
    /// that are common words there are left alone (`data/names_skip.json`). "'s" after a name takes
    /// misaki's ending ("Sauron's").
    pub fn link_names<'t>(&self, text: &'t str, language: &str) -> std::borrow::Cow<'t, str> {
        let links: Vec<(usize, usize)> = link_regex().find_iter(text).map(|m| (m.start(), m.end())).collect();
        self.names.link(text, language, &links, |ps| {
            self.lexicon.s(Some(ps.to_owned())).expect("a name's phonemes aren't empty")
        })
    }

    /// `fold_left`: each non-head token is merged into the token before it. Merging one pair at a time
    /// re-copies the growing prefix, so this extends it in place with the same result as
    /// `merge_tokens([prev, tk], unk='')`.
    fn fold_left(tokens: Vec<MToken>) -> Vec<MToken> {
        let mut result: Vec<MToken> = Vec::with_capacity(tokens.len());
        let mut weight = 0;
        for tk in tokens {
            let Some(acc) = result.last_mut().filter(|_| !tk.is_head) else {
                weight = tag_weight(&tk.text);
                result.push(tk);
                continue;
            };
            let mut ps = acc.phonemes.take().unwrap_or_default();
            push_phonemes(&mut ps, &tk);
            acc.phonemes = Some(ps);
            let tk_weight = tag_weight(&tk.text);
            if tk_weight > weight {
                acc.tag = tk.tag;
            }
            weight += tag_weight(&acc.whitespace) + tk_weight;
            acc.text.push_str(&acc.whitespace);
            acc.text.push_str(&tk.text);
            acc.whitespace = tk.whitespace;
        }
        result
    }
}

fn retokenize(tokens: Vec<MToken>) -> Result<Vec<Word>, Error> {
    let mut words: Vec<Word> = Vec::with_capacity(tokens.len());
    for token in tokens {
        let mut tks: Vec<MToken> = if token.phonemes.is_none() {
            subtokenize(&token.text)
                .into_iter()
                .map(|t| MToken {
                    text: t.to_string(),
                    tag: token.tag.clone(),
                    whitespace: String::new(),
                    phonemes: None,
                    is_head: true,
                    prespace: false,
                })
                .collect()
        } else {
            vec![token.clone()]
        };
        tks.last_mut().ok_or(Error::EmptyToken)?.whitespace = token.whitespace;
        for tk in &mut tks {
            if tk.phonemes.is_some() {
            } else if tk.tag == ":" && (tk.text == "-" || tk.text == "–") {
                tk.phonemes = Some("—".to_string());
            } else if is_punct_tag(&tk.tag) && !all_ascii_letters(&tk.text) {
                tk.phonemes = Some(match punct_tag_phonemes(&tk.tag) {
                    Some(p) => p.to_string(),
                    None => tk.text.chars().filter(|&c| is_punct(c)).collect(),
                });
            }
        }
        for mut tk in tks {
            if tk.phonemes.is_some() {
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

#[cfg(test)]
mod tests {
    use super::G2p;

    #[test]
    fn sentences_in_capitals_are_cased_as_usual() {
        let g2p = G2p::new(false);
        let cased = |t: &str| g2p.sentence_case(t).into_owned();
        assert_eq!(cased("THE QUICK BROWN FOX. AM I IN? I'M OUT."), "The quick brown fox. Am I in? I'm out.");
        // Words in capitals no dictionary has are acronyms; the dictionaries' words are words.
        assert_eq!(cased("THE ZQX OF MCDONALD'S"), "The ZQX of mcdonald's");
        assert_eq!(cased("MISS BENNET"), "Miss bennet");
        assert_eq!(cased("AINULINDALË, THE MUSIC"), "Ainulindalë, the music");
        assert_eq!(cased("THE S-H-A-D-E AND U.S. ARMY"), "The S-H-A-D-E and u.s. Army");
        assert_eq!(cased("THE RESTING-PLACE"), "The resting-place");
        assert_eq!(cased("İSTANBUL İSTANBUL"), "İstanbul İstanbul");
        // Other sentences are left as written, and so is one word in capitals: lowercase text may
        // be a clause cut from a sentence, or follow an abbreviation ("e.g. hammers").
        for t in [
            "The FBI said so.",
            "iPhone sales",
            "1, 2, 3.",
            "i saw it. then i left",
            "Use tools, e.g. hammers.",
            "OK",
            "NASA!",
        ] {
            assert_eq!(cased(t), t);
        }
        assert_eq!(cased("It's fine. NO IT ISN'T!\nCHAPTER ONE"), "It's fine. No it isn't!\nChapter one");
    }
}
