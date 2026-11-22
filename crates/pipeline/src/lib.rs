//! Text in, Kokoro phoneme chunks out: the deployed Kokoro-FastAPI's `smart_split` for English
//! (`lang` a and b), Chinese (`lang` z), Japanese (`j`, kokoro's `g2p_phonemes` with misaki's
//! `JAG2P`) and the espeak languages (`e`, `f`, `h`, `i` and `p`: `g2p_phonemes` with misaki's
//! `EspeakG2P`), end to end.
//!
//! [`Pipeline::chunks`] runs the text crate's tags, normalization, sentence splitting and chunking,
//! with Kokoro-FastAPI's `phonemize` as the G2P: kokoro's `KPipeline.en_g2p_tokens` (runs of other
//! scripts read by their own language, the rest by misaki's English G2P on spaCy's tags, with
//! espeak for unknown words) and `tokens_to_ps`. The G2Ps write IPA, which [`kokoro_phonemes`]
//! reads into Kokoro-82M's input ids, dropping what the vocabulary lacks as `KModel` does.
//!
//! Han runs are read by misaki's Chinese G2P ([`kokoro_g2p_zh`]), in every pipeline, and Japanese
//! runs (Han and kana, with kana) by its Japanese G2P ([`kokoro_g2p_ja`]), whose dictionary is
//! mapped the first time Japanese is read; without the dictionary they are read as "Japanese
//! text". So is a run of any script espeak has no voice for, by the script's name ("Thai text"),
//! and a Han character pypinyin has no reading for ("Chinese text"). [`Chunk::unread`] counts
//! them. `docs/divergences.md` lists every intentional difference from the reference.
#![forbid(unsafe_code)]

mod chinese;
pub mod foreign;
mod tables;

use foreign::{Language, Script};
use kokoro_espeak::{EspeakData, EspeakFallback, EspeakG2P};
use kokoro_g2p_en::{Fallback, G2p, TaggedToken, Tagger, kokoro};
use kokoro_spacy_en::Nlp;
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

pub use kokoro_g2p_en::Token;
#[doc(hidden)]
pub use kokoro_g2p_ja::skip_without_dictionary;
pub use kokoro_text::{DEFAULT_MAX_TOKENS, Split};

/// Whether this process has loaded misaki's Chinese G2P yet: only Han text or a Chinese request
/// loads it.
pub fn chinese_g2p_loaded() -> bool {
    chinese::loaded()
}

/// The Kokoro pipelines this crate covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// `a`
    AmericanEnglish,
    /// `b`
    BritishEnglish,
    /// `z`: Mandarin Chinese.
    Chinese,
    /// `j`: misaki's Japanese G2P (cutlet), with Latin runs read by its American English G2P.
    Japanese,
    /// `e`: misaki's `EspeakG2P('es')`.
    Spanish,
    /// `f`: `EspeakG2P('fr-fr')`.
    French,
    /// `h`: `EspeakG2P('hi')`, with Latin runs read by misaki's American English G2P.
    Hindi,
    /// `i`: `EspeakG2P('it')`.
    Italian,
    /// `p`: `EspeakG2P('pt-br')`.
    BrazilianPortuguese,
}

impl Lang {
    pub const ALL: [Lang; 9] = [
        Lang::AmericanEnglish,
        Lang::BritishEnglish,
        Lang::Spanish,
        Lang::French,
        Lang::Hindi,
        Lang::Italian,
        Lang::Japanese,
        Lang::BrazilianPortuguese,
        Lang::Chinese,
    ];

    /// Kokoro's pipeline code.
    pub fn code(self) -> &'static str {
        match self {
            Lang::AmericanEnglish => "a",
            Lang::BritishEnglish => "b",
            Lang::Chinese => "z",
            Lang::Japanese => "j",
            Lang::Spanish => "e",
            Lang::French => "f",
            Lang::Hindi => "h",
            Lang::Italian => "i",
            Lang::BrazilianPortuguese => "p",
        }
    }

    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.code() == code)
    }

    /// The espeak voice that reads the pipeline's own text (kokoro's `LANG_CODES`), for the
    /// pipelines misaki has no G2P of its own for.
    pub fn espeak_voice(self) -> Option<&'static str> {
        match self {
            Lang::Spanish => Some("es"),
            Lang::French => Some("fr-fr"),
            Lang::Hindi => Some("hi"),
            Lang::Italian => Some("it"),
            Lang::BrazilianPortuguese => Some("pt-br"),
            Lang::AmericanEnglish | Lang::BritishEnglish | Lang::Chinese | Lang::Japanese => None,
        }
    }

    /// What `ForeignScripts` takes as the pipeline's own.
    fn native(self) -> foreign::Native {
        match self {
            Lang::AmericanEnglish | Lang::BritishEnglish => foreign::Native::English,
            Lang::Chinese => foreign::Native::Chinese,
            Lang::Japanese => foreign::Native::Japanese,
            Lang::Hindi => foreign::Native::Espeak(Script::Devanagari),
            Lang::Spanish | Lang::French | Lang::Italian | Lang::BrazilianPortuguese => {
                foreign::Native::Espeak(Script::Latin)
            }
        }
    }

    fn british(self) -> bool {
        self == Lang::BritishEnglish
    }

    fn english(self) -> bool {
        matches!(self, Lang::AmericanEnglish | Lang::BritishEnglish)
    }
}

#[derive(Debug)]
pub enum Error {
    /// misaki's English G2P failed: spaCy's tokens didn't align with the text, or it returned an
    /// empty token.
    G2p(kokoro_g2p_en::Error),
    /// espeak-ng's data couldn't be read.
    Espeak(kokoro_espeak::Error),
    /// The Chinese or Japanese G2P (bundled, or passed to [`Pipeline::with_foreign_g2p`]) failed.
    Foreign { language: Language, message: String },
    /// The Japanese G2P's dictionary couldn't be loaded.
    Dictionary(String),
    /// The caller's `cancelled` returned true before a G2P call
    /// ([`Pipeline::chunks_within_groups`]).
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::G2p(e) => write!(f, "English G2P: {e}"),
            Error::Espeak(e) => write!(f, "espeak: {e}"),
            Error::Foreign { language, message } => write!(f, "{} G2P: {message}", language.code()),
            Error::Dictionary(message) => write!(f, "Japanese dictionary: {message}"),
            Error::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<kokoro_g2p_en::Error> for Error {
    fn from(e: kokoro_g2p_en::Error) -> Self {
        Error::G2p(e)
    }
}

impl From<kokoro_espeak::Error> for Error {
    fn from(e: kokoro_espeak::Error) -> Self {
        Error::Espeak(e)
    }
}

/// A G2P for runs of another language's script, for the languages espeak doesn't read here
/// (Chinese and Japanese). It must not keep state between calls.
pub trait ForeignG2p: Send + Sync {
    /// The language's phonemes for a run, in IPA, before `foreign.py`'s post-processing.
    fn phonemize(&self, text: &str) -> Result<String, String>;

    /// Chinese only: whether the character has a reading. The reference leaves characters
    /// pypinyin can't read (some radicals, Japanese-only kanji) to the English G2P.
    fn reads(&self, _c: char) -> bool {
        true
    }

    /// Whether it can read at all (its data loaded). If not, its runs are read as "<name> text".
    fn available(&self) -> bool {
        true
    }
}

/// One item of [`Pipeline::chunks`]: speech, or silence from a `[pause:Xs]` tag.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// The chunk's text after normalization; empty for a pause.
    pub text: String,
    /// The chunk's phonemes in IPA.
    pub phonemes: String,
    /// `phonemes` as Kokoro-82M input ids, without the padding id 0 at either end. Sounds the
    /// vocabulary lacks are dropped, as `KModel` does ([`Chunk::dropped`]).
    pub ids: Vec<u8>,
    /// Letters and marks of `phonemes` the vocabulary lacks, left out of `ids`.
    pub dropped: usize,
    /// Seconds of silence, for a pause chunk.
    pub pause: Option<f64>,
    /// How many runs with no G2P for their language or script were read as "<name> text".
    pub unread: usize,
    /// Which sides of the chunk were cut from its sentence at clause punctuation, for a sentence
    /// longer than the chunk limit.
    pub split: Split,
    /// The words' tokens, for [`Chunk::words`]; none for a pause.
    tokens: Vec<Token>,
}

impl Chunk {
    /// The words with phonemes, for word timestamps (Kokoro-FastAPI's `_word_spans`).
    pub fn words(&self) -> Vec<Word> {
        word_spans(&self.tokens, &kokoro_phonemes::tokenize(&self.phonemes).kept)
    }
}

/// A word of a chunk, located in its phonemes (Kokoro-FastAPI's `_word_spans`). A run of another
/// script is one word, holding the run's first phoneme word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    /// Character range in [`Chunk::phonemes`].
    pub phonemes: Range<usize>,
    /// Range in [`Chunk::ids`]: the word's phonemes that are in the vocabulary (never empty).
    pub ids: Range<usize>,
}

/// One language's G2P, loaded once and shared across threads. Every call to [`chunks`](Self::chunks)
/// gets its own espeak engines (about 5 µs each), so requests never wait on one another's engine
/// lock; espeak itself starts every call from fresh state, so nothing passes between requests.
pub struct Pipeline {
    lang: Lang,
    nlp: &'static Nlp,
    g2p: &'static G2p,
    espeak: Arc<EspeakData>,
    foreign: HashMap<Language, Box<dyn ForeignG2p>>,
    /// The tokens of each language's "<name> text", read once on first use.
    placeholders: Box<[OnceLock<Vec<Token>>]>,
}

/// spaCy's tagger, loaded once per process and shared by every pipeline: nine pipelines take
/// 124 MB with the lexicons below, where each loading its own took 481 MB. Both only read.
fn nlp() -> &'static Nlp {
    static NLP: OnceLock<Nlp> = OnceLock::new();
    NLP.get_or_init(Nlp::new)
}

/// misaki's English G2P with its lexicons, one per dialect, loaded once per process.
fn english_g2p(british: bool) -> &'static G2p {
    static AMERICAN: OnceLock<G2p> = OnceLock::new();
    static BRITISH: OnceLock<G2p> = OnceLock::new();
    let cell = if british { &BRITISH } else { &AMERICAN };
    cell.get_or_init(|| G2p::new(british))
}

/// misaki's Japanese G2P on the default dictionary, loaded the first time Japanese is read
/// ([`kokoro_g2p_ja::default_dictionary`]), so a pipeline that never reads Japanese never loads it.
struct LazyJapanese;

impl ForeignG2p for LazyJapanese {
    fn phonemize(&self, text: &str) -> Result<String, String> {
        let g2p = kokoro_g2p_ja::JaG2p::load_default().map_err(|e| e.to_string())?;
        g2p.phonemize(text).map_err(|e| e.to_string())
    }

    fn available(&self) -> bool {
        japanese_available()
    }
}

/// Whether misaki's Japanese G2P can read: UniDic is at `$KOKORO_UNIDIC` (this maps it).
pub fn japanese_available() -> bool {
    kokoro_g2p_ja::default_dictionary().is_ok()
}

impl Pipeline {
    /// Loads the pipeline with the bundled espeak-ng data. misaki's Chinese G2P is loaded on the
    /// first Han character or Chinese request (see [`chinese_g2p_loaded`]), once per process.
    /// Japanese (the `j` pipeline's text, and Japanese runs in other pipelines' text) is read by
    /// misaki's Japanese G2P, whose dictionary, UniDic, is mapped the first time Japanese is read,
    /// once per process, from `$KOKORO_UNIDIC`. Without it, the `j` pipeline's sentences fail with
    /// [`Error::Dictionary`], and Japanese runs in other pipelines are read as "Japanese text".
    pub fn new(lang: Lang) -> Result<Pipeline, Error> {
        let espeak = kokoro_espeak::default_data()?;
        EspeakFallback::with_data(lang.british(), espeak.clone())?;
        for voice in foreign::espeak_voices().into_iter().chain(lang.espeak_voice()) {
            EspeakG2P::with_data(espeak_voice(voice, lang), espeak.clone())?;
        }
        let mut foreign: HashMap<Language, Box<dyn ForeignG2p>> = HashMap::new();
        foreign.insert(Language::Chinese, Box::new(chinese::Chinese));
        foreign.insert(Language::Japanese, Box::new(LazyJapanese));
        Ok(Pipeline {
            lang,
            nlp: nlp(),
            g2p: english_g2p(lang.british()),
            espeak,
            foreign,
            placeholders: (0..2 + Script::COUNT).map(|_| OnceLock::new()).collect(),
        })
    }

    /// Reads `language`'s runs with `g2p` instead of as "<name> text" (or instead of espeak, or the
    /// bundled Chinese G2P, which the Chinese pipeline also reads its own text with).
    pub fn with_foreign_g2p(mut self, language: Language, g2p: Box<dyn ForeignG2p>) -> Self {
        self.foreign.insert(language, g2p);
        self
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// `smart_split(text, lang)` as the reference runs it: sentences of more than
    /// [`DEFAULT_MAX_TOKENS`] phonemes cut at clauses, packed into groups of up to that.
    /// See [`chunks_within_groups`](Self::chunks_within_groups).
    pub fn chunks<'a>(&'a self, text: &str) -> impl Iterator<Item = Result<Chunk, Error>> + Send + use<'a> {
        self.chunks_within_groups(text, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| false)
    }

    /// `smart_split(text, lang)` with the default normalization options (requests can't set
    /// them, `docs/divergences.md`): lazy, so each sentence's G2P runs as its chunks are taken. An
    /// error stands in for the sentence it happened in, and later sentences still follow. Voice
    /// and rate tags aren't supported: they're read as text.
    ///
    /// Sentences of more than `max_tokens` phonemes are cut at clauses, and the clauses packed into
    /// groups of at most `clause_groups` phonemes (`kokoro_text::SmartSplit::with_clause_groups`):
    /// the server's limit depends on the request's speed, and it packs its chunks to a soft size.
    ///
    /// `cancelled` is asked before each G2P call (a sentence can take several: itself, its clauses
    /// and their groups); once it says yes, every chunk still to come is [`Error::Cancelled`]. The
    /// server stops processing a request this way once its client has gone.
    pub fn chunks_within_groups<'a>(
        &'a self,
        text: &str,
        max_tokens: usize,
        clause_groups: usize,
        cancelled: &'a (dyn Fn() -> bool + Sync),
    ) -> impl Iterator<Item = Result<Chunk, Error>> + Send + use<'a> {
        let mut request = Request::new(self);
        kokoro_text::try_smart_split(
            text,
            self.lang.code(),
            kokoro_text::Options::default(),
            max_tokens,
            move |t: &str| {
                if cancelled() {
                    return Err(Error::Cancelled);
                }
                request.phonemize(t)
            },
        )
        .with_clause_groups(clause_groups)
        .map(|chunk| {
            chunk.map(|chunk| match chunk {
                kokoro_text::Chunk::Speech(piece) => speech(piece.text, piece.phonemes, piece.split),
                kokoro_text::Chunk::Pause(seconds) => Chunk {
                    text: String::new(),
                    phonemes: String::new(),
                    ids: Vec::new(),
                    dropped: 0,
                    pause: Some(seconds),
                    unread: 0,
                    split: Split::default(),
                    tokens: Vec::new(),
                },
            })
        })
    }

    /// Kokoro-FastAPI's `phonemize(text, lang)` for one piece of text, without chunking: the
    /// phoneme string and misaki's tokens.
    pub fn phonemize(&self, text: &str) -> Result<(String, Vec<Token>), Error> {
        let p = Request::new(self).phonemize(text)?;
        Ok((p.phonemes, p.tokens))
    }
}

/// The espeak voice for a language code: espeak-ng switches Shavian to plain `en`, which is the
/// pipeline's own English here.
fn espeak_voice(code: &str, lang: Lang) -> &str {
    match code {
        "en" if lang.british() => "en-gb",
        "en" => "en-us",
        code => code,
    }
}

/// G2P output: the IPA, the ids it reads as (which chunking measures), the words' tokens,
/// and how many runs were read as a placeholder for want of a G2P.
struct Phonemes {
    phonemes: String,
    read: kokoro_phonemes::Tokens,
    tokens: Vec<Token>,
    unread: usize,
}

impl Phonemes {
    fn new(phonemes: String, tokens: Vec<Token>, unread: usize) -> Self {
        Phonemes { read: kokoro_phonemes::tokenize(&phonemes), phonemes, tokens, unread }
    }
}

impl kokoro_text::Phonemes for Phonemes {
    fn phonemes(&self) -> &str {
        &self.phonemes
    }

    fn ids(&self) -> usize {
        self.read.ids.len()
    }
}

fn speech(text: String, p: Phonemes, split: Split) -> Chunk {
    let kokoro_phonemes::Tokens { ids, dropped, .. } = p.read;
    Chunk { text, phonemes: p.phonemes, ids, dropped, pause: None, unread: p.unread, split, tokens: p.tokens }
}

/// `_word_spans` followed by the vocabulary filter `_render_stitched` applies: each word with
/// phonemes, located by walking the tokens as `tokens_to_ps` joins them. `kept[i]` is the number
/// of ids for the first `i` phonemes.
///
/// Positions are counted in the joined string before `strip`, and each word's span is cut to the
/// stripped string, so a token whose phonemes were stripped away (blank phonemes at either end)
/// gets an empty span and is left out, as Python's `inside` list comes out empty for it.
fn word_spans(tokens: &[Token], kept: &[usize]) -> Vec<Word> {
    let mut leading = 0;
    for t in tokens {
        let ps = t.phonemes.as_deref().unwrap_or("");
        let blank = ps.chars().take_while(|&c| kokoro_text::py::is_space(c)).count();
        leading += blank;
        if blank < ps.chars().count() {
            break;
        }
        leading += usize::from(!t.whitespace.is_empty());
    }
    let len = kept.len() - 1;
    let stripped = |raw: usize| raw.saturating_sub(leading).min(len);
    let mut raw = 0;
    let mut words = Vec::new();
    for t in tokens {
        let n = t.phonemes.as_deref().unwrap_or("").chars().count();
        let text = kokoro_text::py::strip(&t.text);
        if !text.is_empty() && n > 0 && raw >= leading {
            let phonemes = stripped(raw)..stripped(raw + n);
            let ids = kept[phonemes.start]..kept[phonemes.end];
            if !ids.is_empty() {
                words.push(Word { text: text.to_owned(), phonemes, ids });
            }
        }
        raw += n + usize::from(!t.whitespace.is_empty());
    }
    words
}

/// The first and last characters of `segment` as read: a link at either end by its text.
fn spoken_edges(segment: &str) -> (Option<char>, Option<char>) {
    static LINK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\[([^\]]+)\]\(([^\)]*)\)").expect("valid regex"));
    let (mut first, mut last) = (segment.chars().next(), segment.chars().next_back());
    if segment.contains('[') {
        for m in LINK.captures_iter(segment) {
            let whole = m.get(0).expect("match");
            if whole.start() == 0 {
                first = m[1].chars().next();
            }
            if whole.end() == segment.len() {
                last = m[1].chars().next_back();
            }
        }
    }
    (first, last)
}

/// Two runs whose letters meet are still two words: scripts without spaces ("東京Tokyo").
fn space_where_letters_meet(tokens: &mut [Token], before: char, after: char) {
    if kokoro_g2p_en::is_alpha(before)
        && kokoro_g2p_en::is_alpha(after)
        && let Some(last) = tokens.last_mut().filter(|t| t.whitespace.is_empty())
    {
        last.whitespace.push(' ');
    }
}

/// A piece of a run outside English: text for the run's G2P, or a `[text](/phonemes/)` link's.
enum Piece {
    Text(String),
    Phonemes { text: String, ipa: String },
}

/// `segment` cut at its links, each read as its phonemes (misaki's `/phonemes/`, IPA's syllable
/// breaks left out). Outside English every other link is plain text already
/// ([`G2p::plain_links`]).
fn link_pieces(segment: &str) -> Vec<Piece> {
    static LINK: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\[([^\]]+)\]\(([^\)]*)\)").expect("valid regex"));
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut last = 0;
    for m in LINK.captures_iter(segment) {
        let whole = m.get(0).expect("match");
        text.push_str(&segment[last..whole.start()]);
        last = whole.end();
        let feature = &m[2];
        if feature.chars().count() > 1 && feature.starts_with('/') && feature.ends_with('/') {
            if !text.is_empty() {
                pieces.push(Piece::Text(std::mem::take(&mut text)));
            }
            let ipa = feature[1..].trim_end_matches('/').chars().filter(|&c| c != '.').collect();
            pieces.push(Piece::Phonemes { text: m[1].to_owned(), ipa });
        } else {
            text.push_str(&m[1]);
        }
    }
    text.push_str(&segment[last..]);
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    pieces
}

/// Appends a run read by a G2P other than misaki's English: one token per word of `phonemes`,
/// each holding its word of the text when the run has as many words as phoneme words, else the
/// first holding the run's text. The run's whitespace at either end joins it to its neighbours,
/// as misaki's tokens are joined.
fn append_run(tokens: &mut Vec<Token>, segment: &str, phonemes: &str) {
    let words: Vec<&str> = phonemes.split(kokoro_text::py::is_space).filter(|w| !w.is_empty()).collect();
    let stripped = segment.trim_start_matches(kokoro_text::py::is_space);
    let trailing = &stripped[stripped.trim_end_matches(kokoro_text::py::is_space).len()..];
    if let Some(last) = tokens.last_mut() {
        last.whitespace.push_str(&segment[..segment.len() - stripped.len()]);
        // A run read as nothing still parts its neighbours where it has whitespace.
        if words.is_empty() {
            last.whitespace.push_str(trailing);
        }
    }
    let n = words.len();
    let texts: Vec<&str> = segment.split(kokoro_text::py::is_space).filter(|w| !w.is_empty()).collect();
    let text = |i: usize| match texts.len() == n {
        true => texts[i].to_owned(),
        false if i == 0 => kokoro_text::py::strip(segment).to_owned(),
        false => String::new(),
    };
    tokens.extend(words.iter().enumerate().map(|(i, w)| Token {
        text: text(i),
        tag: String::new(),
        whitespace: if i + 1 < n { " ".to_owned() } else { trailing.to_owned() },
        phonemes: Some((*w).to_owned()),
    }));
}

/// Puts a space between each placeholder and a word right against it on either side, or clause
/// punctuation before it, unless there's whitespace already: the run it stands for may have been
/// written without spaces ("，カナ" in Chinese), or with only invisible characters around it.
fn separate(tokens: &mut [Token], placeholders: &[Range<usize>]) {
    let spoken = |t: &Token| t.phonemes.as_deref().is_some_and(|p| !p.is_empty()) || !t.whitespace.is_empty();
    let word = |c: Option<char>| c.is_some_and(char::is_alphabetic);
    for range in placeholders {
        if let Some(before) = tokens[..range.start].iter().rposition(spoken)
            && tokens[before].whitespace.is_empty()
            && tokens[before]
                .phonemes
                .as_deref()
                .and_then(|p| p.chars().next_back())
                .is_some_and(|c| c.is_alphabetic() || ",;:.!?—…".contains(c))
        {
            tokens[before].whitespace.push(' ');
        }
        let last = range.end - 1;
        if let Some(after) = tokens[range.end..].iter().find(|t| spoken(t))
            && word(after.phonemes.as_deref().and_then(|p| p.chars().next()))
            && tokens[last].whitespace.is_empty()
        {
            tokens[last].whitespace.push(' ');
        }
    }
}

/// The end of the unread run at `segments[start]`: runs of one language with only joiners, marks
/// or format characters between them are read as one placeholder.
fn unread_run_end(segments: &[(&str, Option<Language>)], start: usize) -> usize {
    let language = segments[start].1;
    let mut end = start + 1;
    while let [(gap, None), (_, next), ..] = segments[end..]
        && next == language
        && gap.chars().all(foreign::is_gap)
    {
        end += 2;
    }
    end
}

/// One request's G2P state: espeak engines are made on first use and dropped with the request.
struct Request<'p> {
    pipeline: &'p Pipeline,
    fallback: Option<EspeakFallback>,
    espeak: HashMap<Language, EspeakG2P>,
    /// `EspeakG2P` in the pipeline's own language, for the espeak pipelines.
    native: Option<EspeakG2P>,
}

struct Spacy<'a>(&'a Nlp);

impl Tagger for Spacy<'_> {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken> {
        self.0
            .process(text)
            .into_iter()
            .map(|t| TaggedToken {
                text: t.text.to_owned(),
                tag: t.tag.to_owned(),
                whitespace: t.whitespace.to_owned(),
            })
            .collect()
    }
}

/// The espeak fallback for unknown English words, made on first use.
struct LazyFallback<'a> {
    data: &'a Arc<EspeakData>,
    british: bool,
    engine: &'a mut Option<EspeakFallback>,
    error: Option<kokoro_espeak::Error>,
}

impl Fallback for LazyFallback<'_> {
    fn phonemize(&mut self, word: &str) -> Option<String> {
        if self.engine.is_none() {
            match EspeakFallback::with_data(self.british, self.data.clone()) {
                Ok(engine) => *self.engine = Some(engine),
                Err(e) => {
                    self.error.get_or_insert(e);
                    return None;
                }
            }
        }
        self.engine.as_mut().expect("made above").phonemize(word)
    }
}

impl<'p> Request<'p> {
    fn new(pipeline: &'p Pipeline) -> Self {
        Request { pipeline, fallback: None, espeak: HashMap::new(), native: None }
    }

    /// Kokoro-FastAPI's `phonemize`, one way for every pipeline: CJK punctuation made Western
    /// (but in Chinese and Japanese, whose G2Ps do their own), the text split into runs
    /// (`foreign.py`), each run read by its language's G2P into word tokens, and the tokens joined
    /// (`tokens_to_ps`): a space between two runs where the text has whitespace at the join, or
    /// two letters meet there.
    /// The pipeline's own runs go to misaki's English G2P (`lang` a, b), misaki's Chinese or
    /// Japanese G2P, or `EspeakG2P` in the pipeline's language; Latin runs elsewhere go to misaki's
    /// American English G2P.
    ///
    /// A run with no G2P for its language or script (Japanese without UniDic, and the letters
    /// espeak has no voice for) is read as "<name> text" instead: once for runs of one language
    /// with only joiners, marks or format characters between them, and kept apart from the words
    /// on either side. So is a run its G2P reads as nothing, outside `foreign.py`'s scripts (where
    /// the reference reads it as nothing too).
    fn phonemize(&mut self, text: &str) -> Result<Phonemes, Error> {
        let lang = self.pipeline.lang;
        if lang == Lang::Japanese {
            let g2p = self.pipeline.foreign.get(&Language::Japanese).expect("every pipeline has a Japanese G2P");
            if !g2p.available() {
                return Err(Error::Dictionary(
                    kokoro_g2p_ja::default_dictionary().err().map_or_else(String::new, |e| e.to_string()),
                ));
            }
        }
        let graphemes = match lang {
            Lang::Chinese | Lang::Japanese => text.into(),
            _ => foreign::translate_cjk_punctuation(text),
        };
        // Latin text is the pipeline's own in Spanish, French, Italian and Portuguese.
        let latin = match lang.native() {
            foreign::Native::Espeak(Script::Latin) => lang.espeak_voice().and_then(|v| v.get(..2)).unwrap_or("en"),
            _ => "en",
        };
        let graphemes = G2p::plain_links(&graphemes, lang.english());
        let graphemes = self.pipeline.g2p.link_names(&graphemes, latin);
        let mut tokens: Vec<Token> = Vec::new();
        let mut placeholders = Vec::new();
        let chinese = self.pipeline.foreign.get(&Language::Chinese);
        let segments = foreign::split_in(&graphemes, lang.native(), |c| chinese.is_none_or(|g2p| g2p.reads(c)));
        let mut i = 0;
        while i < segments.len() {
            let (segment, language) = segments[i];
            if i > 0
                && let (Some(a), Some(b)) = (spoken_edges(segments[i - 1].0).1, spoken_edges(segment).0)
            {
                space_where_letters_meet(&mut tokens, a, b);
            }
            i += 1;
            let run = match language {
                None if lang.english() => {
                    self.english(&mut tokens, segment)?;
                    continue;
                }
                Some(Language::English) => {
                    self.english(&mut tokens, &foreign::prepare(segment, Language::English))?;
                    continue;
                }
                None => {
                    // misaki reads links in English; elsewhere a link is a run of its own.
                    let mut previous: Option<char> = None;
                    for piece in link_pieces(segment) {
                        let (text, ps) = match piece {
                            Piece::Text(text) => {
                                let ps = self.native(&text)?;
                                (text, ps)
                            }
                            Piece::Phonemes { text, ipa } => (text, ipa),
                        };
                        if let (Some(a), Some(b)) = (previous, text.chars().next()) {
                            space_where_letters_meet(&mut tokens, a, b);
                        }
                        append_run(&mut tokens, &text, &ps);
                        previous = text.chars().next_back().or(previous);
                    }
                    continue;
                }
                Some(language) if self.reads(language) => {
                    let ps = self.foreign(segment, language)?;
                    let reference = language == Language::Japanese
                        || matches!(language, Language::Script(s) if foreign::in_foreign_py(s));
                    if !ps.is_empty() || reference {
                        append_run(&mut tokens, segment, &ps);
                        continue;
                    }
                    segment.to_owned()
                }
                Some(_) => {
                    let start = i - 1;
                    i = unread_run_end(&segments, start);
                    segments[start..i].iter().map(|(s, _)| *s).collect()
                }
            };
            let language = language.expect("the pipeline's own runs are read");
            let start = tokens.len();
            tokens.extend(self.placeholder(language, run)?);
            placeholders.push(start..tokens.len());
        }
        separate(&mut tokens, &placeholders);
        let phonemes = kokoro::tokens_to_ps(&tokens);
        for token in &mut tokens {
            token.phonemes.get_or_insert_with(String::new);
        }
        Ok(Phonemes::new(phonemes, tokens, placeholders.len()))
    }

    fn reads(&self, language: Language) -> bool {
        language == Language::English
            || language.espeak().is_some()
            || self.pipeline.foreign.get(&language).is_some_and(|g2p| g2p.available())
    }

    /// The pipeline's own G2P on a segment `ForeignScripts` didn't split out: misaki's Chinese or
    /// Japanese G2P, or `EspeakG2P` in the pipeline's language, stripped.
    fn native(&mut self, segment: &str) -> Result<String, Error> {
        let pipeline = self.pipeline;
        let raw = match pipeline.lang.espeak_voice() {
            None => {
                let language = if pipeline.lang == Lang::Chinese { Language::Chinese } else { Language::Japanese };
                let g2p = pipeline.foreign.get(&language).expect("every pipeline has Chinese and Japanese G2Ps");
                g2p.phonemize(segment).map_err(|message| Error::Foreign { language, message })?
            }
            Some(voice) => {
                let engine = match &mut self.native {
                    Some(engine) => engine,
                    native => native.insert(EspeakG2P::with_data(voice, pipeline.espeak.clone())?),
                };
                engine.phonemize(segment)
            }
        };
        Ok(kokoro_text::py::strip(&raw).to_owned())
    }

    /// "<name> text" for an unread run, as English words that make one token group like a foreign
    /// run's: the first holds the run's text.
    fn placeholder(&mut self, language: Language, run: String) -> Result<Vec<Token>, Error> {
        let index = match language {
            Language::Chinese => 0,
            Language::Japanese => 1,
            Language::English => unreachable!("English runs always have a G2P"),
            Language::Script(script) | Language::Unread(script) => 2 + script as usize,
        };
        let cell = &self.pipeline.placeholders[index];
        let mut words = match cell.get() {
            Some(words) => words.clone(),
            None => {
                let mut words = Vec::new();
                self.english(&mut words, &format!("{} text", language.name()))?;
                cell.get_or_init(|| words).clone()
            }
        };
        for (i, word) in words.iter_mut().enumerate() {
            word.text = if i == 0 { run.clone() } else { String::new() };
        }
        if let Some(last) = words.last_mut() {
            last.whitespace.clear();
        }
        Ok(words)
    }

    fn english(&mut self, tokens: &mut Vec<Token>, segment: &str) -> Result<(), Error> {
        let pipeline = self.pipeline;
        let mut tagger = Spacy(pipeline.nlp);
        let mut fallback = LazyFallback {
            data: &pipeline.espeak,
            british: pipeline.lang.british(),
            engine: &mut self.fallback,
            error: None,
        };
        kokoro::append_segment(pipeline.g2p, tokens, segment, &mut tagger, Some(&mut fallback))?;
        match fallback.error {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    /// `ForeignScripts.phonemize`, for a language [`reads`](Self::reads) is true of.
    fn foreign(&mut self, segment: &str, language: Language) -> Result<String, Error> {
        let text = foreign::prepare(segment, language);
        let raw = if let Some(g2p) = self.pipeline.foreign.get(&language) {
            g2p.phonemize(&text).map_err(|message| Error::Foreign { language, message })?
        } else {
            let voice = espeak_voice(language.espeak().expect("an espeak language"), self.pipeline.lang);
            let engine = match self.espeak.entry(language) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(EspeakG2P::with_data(voice, self.pipeline.espeak.clone())?)
                }
            };
            engine.phonemize(&text)
        };
        Ok(foreign::finish(&raw, language, self.pipeline.lang.english()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, phonemes: &str, whitespace: &str) -> Token {
        Token { text: text.into(), tag: String::new(), whitespace: whitespace.into(), phonemes: Some(phonemes.into()) }
    }

    #[test]
    fn word_spans_skip_leading_blanks_and_dropped_phonemes() {
        let tokens = [token("", "", " "), token("¡Hola", "¡ola", " "), token("x", "ab", "")];
        let ps = kokoro::tokens_to_ps(&tokens);
        assert_eq!(ps, "¡ola ab");
        let kokoro_phonemes::Tokens { ids, kept, .. } = kokoro_phonemes::tokenize(&ps);
        assert_eq!(ids.len(), 6);
        let words = word_spans(&tokens, &kept);
        assert_eq!(
            words,
            [
                Word { text: "¡Hola".into(), phonemes: 0..4, ids: 0..3 },
                Word { text: "x".into(), phonemes: 5..7, ids: 4..6 },
            ]
        );
    }
}
