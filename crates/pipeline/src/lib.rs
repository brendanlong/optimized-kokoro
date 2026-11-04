//! Text in, Kokoro phoneme chunks out: the deployed Kokoro-FastAPI's `smart_split` for English
//! (`lang` a and b), Chinese (`lang` z), Japanese (`j`, kokoro's `g2p_phonemes` with misaki's
//! `JAG2P`) and the espeak languages (`e`, `f`, `h`, `i` and `p`: `g2p_phonemes` with misaki's
//! `EspeakG2P`), end to end.
//!
//! [`Pipeline::chunks`] runs the text crate's tags, normalization, sentence splitting and chunking,
//! with Kokoro-FastAPI's `phonemize` as the G2P: kokoro's `KPipeline.en_g2p_tokens` (runs of other
//! scripts read by their own language, the rest by misaki's English G2P on spaCy's tags, with
//! espeak for unknown words) and `tokens_to_ps`. Each chunk's phonemes are then mapped to
//! Kokoro-82M's input ids, dropping what the vocabulary lacks as `KModel` does.
//!
//! Han runs are read by misaki's Chinese G2P ([`kokoro_g2p_zh`]), in every pipeline, and Japanese
//! runs (Han and kana, with kana) by its Japanese G2P ([`kokoro_g2p_ja`]), whose dictionary is
//! mapped the first time Japanese is read; without the dictionary they are read as "Japanese
//! text". So is a run of any script espeak has no voice for, by the script's name ("Thai text"),
//! and a Han character pypinyin has no reading for ("Chinese text"). [`Chunk::unread`] counts
//! them. The Chinese pipeline is in `chinese.rs`. `docs/divergences.md` lists every intentional difference from
//! the reference.
#![forbid(unsafe_code)]

mod chinese;
pub mod foreign;
mod tables;
mod vocab;

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
pub use kokoro_text::{DEFAULT_MAX_TOKENS, Limits, Options as NormalizationOptions};
pub use vocab::id;

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
    /// misaki's English G2P failed where the reference raises (a number too large to read).
    G2p(kokoro_g2p_en::Error),
    /// espeak-ng's data couldn't be read.
    Espeak(kokoro_espeak::Error),
    /// A G2P passed to [`Pipeline::with_foreign_g2p`] failed.
    Foreign { language: Language, message: String },
    /// The Japanese G2P's dictionary couldn't be loaded.
    Dictionary(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::G2p(e) => write!(f, "English G2P: {e}"),
            Error::Espeak(e) => write!(f, "espeak: {e}"),
            Error::Foreign { language, message } => write!(f, "{} G2P: {message}", language.code()),
            Error::Dictionary(message) => write!(f, "Japanese dictionary: {message}"),
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
    /// The language's phonemes for a run, before `foreign.py`'s post-processing.
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
    /// Kokoro phonemes, as the reference server hands them to the model.
    pub phonemes: String,
    /// `phonemes` as Kokoro-82M input ids, without the padding id 0 at either end. Phonemes the
    /// vocabulary lacks are dropped, as `KModel` does; `dropped` counts them.
    pub ids: Vec<u8>,
    pub dropped: usize,
    /// Seconds of silence, for a pause chunk.
    pub pause: Option<f64>,
    /// The words with phonemes, for word timestamps.
    pub words: Vec<Word>,
    /// How many runs with no G2P for their language or script were read as "<name> text".
    pub unread: usize,
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
    limits: Limits,
    max_tokens: usize,
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
        kokoro_g2p_ja::default_dictionary().is_ok()
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
        Self::with_espeak_data(lang, kokoro_espeak::default_data()?)
    }

    pub fn with_espeak_data(lang: Lang, espeak: Arc<EspeakData>) -> Result<Pipeline, Error> {
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
            limits: Limits::default(),
            max_tokens: DEFAULT_MAX_TOKENS,
        })
    }

    /// Reads `language`'s runs with `g2p` instead of as "<name> text" (or instead of espeak, or the
    /// bundled Chinese G2P, which the Chinese pipeline also reads its own text with).
    pub fn with_foreign_g2p(mut self, language: Language, g2p: Box<dyn ForeignG2p>) -> Self {
        self.foreign.insert(language, g2p);
        self
    }

    /// Pause limits (default: the deployed server's).
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Phonemes per chunk before a sentence is cut at clauses (default [`DEFAULT_MAX_TOKENS`]).
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// `smart_split(text, lang)`: lazy, so each sentence's G2P runs as its chunks are taken. An
    /// error stands in for the sentence it happened in, and later sentences still follow.
    ///
    /// Voice and rate tags aren't supported: they're read as text.
    pub fn chunks<'a>(
        &'a self,
        text: &str,
        options: &'a NormalizationOptions,
    ) -> impl Iterator<Item = Result<Chunk, Error>> + Send + use<'a> {
        let mut request = Request::new(self);
        kokoro_text::try_smart_split(text, self.lang.code(), options, &self.limits, self.max_tokens, move |t: &str| {
            request.phonemize(t)
        })
        .map(|chunk| {
            chunk.map(|chunk| match chunk {
                kokoro_text::Chunk::Speech(piece) => speech(piece.text, piece.phonemes),
                kokoro_text::Chunk::Pause(seconds) => Chunk {
                    text: String::new(),
                    phonemes: String::new(),
                    ids: Vec::new(),
                    dropped: 0,
                    pause: Some(seconds),
                    words: Vec::new(),
                    unread: 0,
                },
            })
        })
    }

    /// Kokoro-FastAPI's `phonemize(text, lang)` for one piece of text, without chunking: the
    /// phoneme string and misaki's tokens.
    pub fn phonemize(&self, text: &str) -> Result<(String, Vec<Token>), Error> {
        let p = Request::new(self).phonemize(text)?;
        Ok((p.phonemes, p.tokens.unwrap_or_default()))
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

/// G2P output: the phoneme string chunking measures, misaki's tokens (English only), and how many
/// runs were read as a placeholder for want of a G2P.
struct Phonemes {
    phonemes: String,
    tokens: Option<Vec<Token>>,
    unread: usize,
}

impl kokoro_text::Phonemes for Phonemes {
    fn phonemes(&self) -> &str {
        &self.phonemes
    }
}

fn speech(text: String, p: Phonemes) -> Chunk {
    let (ids, kept) = vocab::ids(&p.phonemes);
    let words = match &p.tokens {
        Some(tokens) => word_spans(tokens, &kept),
        None => text_word_spans(&text, &p.phonemes, &kept),
    };
    let dropped = kept.len() - 1 - ids.len();
    Chunk { text, phonemes: p.phonemes, ids, dropped, pause: None, words, unread: p.unread }
}

/// `_word_spans` without misaki's tokens (every pipeline but English): the text's words paired
/// with the phonemes' space-separated groups, when there are as many of each, else none.
fn text_word_spans(text: &str, phonemes: &str, kept: &[usize]) -> Vec<Word> {
    let words: Vec<&str> = text.split(kokoro_text::py::is_space).filter(|w| !w.is_empty()).collect();
    let mut groups = Vec::new();
    let mut start = None;
    for (i, c) in phonemes.chars().chain([' ']).enumerate() {
        match (kokoro_text::py::is_space(c), start) {
            (true, Some(s)) => {
                groups.push(s..i);
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if groups.len() != words.len() {
        return Vec::new();
    }
    words
        .into_iter()
        .zip(groups)
        .filter_map(|(word, phonemes)| {
            let ids = kept[phonemes.start]..kept[phonemes.end];
            (!ids.is_empty()).then(|| Word { text: word.to_owned(), phonemes, ids })
        })
        .collect()
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

/// Puts a space between each placeholder and a word right against it on either side, unless
/// there's whitespace already: the run it stands for may have been written without spaces, or
/// with only invisible characters around it.
fn separate(tokens: &mut [Token], placeholders: &[Range<usize>]) {
    let spoken = |t: &Token| t.phonemes.as_deref().is_some_and(|p| !p.is_empty()) || !t.whitespace.is_empty();
    let word = |c: Option<char>| c.is_some_and(char::is_alphabetic);
    for range in placeholders {
        if let Some(before) = tokens[..range.start].iter().rposition(spoken)
            && tokens[before].whitespace.is_empty()
            && word(tokens[before].phonemes.as_deref().and_then(|p| p.chars().next_back()))
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
        let engine = self.engine.as_ref().expect("made above");
        engine.phonemize(word).unwrap_or_else(|e| {
            self.error.get_or_insert(e);
            None
        })
    }
}

impl<'p> Request<'p> {
    fn new(pipeline: &'p Pipeline) -> Self {
        Request { pipeline, fallback: None, espeak: HashMap::new(), native: None }
    }

    /// Kokoro-FastAPI's `phonemize` for English: CJK punctuation made Western, then
    /// `en_g2p_tokens` and `tokens_to_ps`.
    ///
    /// A run with no G2P for its language or script (so far Chinese and Japanese, and the letters
    /// espeak has no voice for) is read as "<name> text" instead: once for runs of one language
    /// with only joiners, marks or format characters between them, and kept apart from the words
    /// on either side. So is a run its G2P reads as nothing, outside `foreign.py`'s scripts (where
    /// the reference reads it as nothing too).
    fn phonemize(&mut self, text: &str) -> Result<Phonemes, Error> {
        if self.pipeline.lang == Lang::Chinese {
            return self.chinese(text);
        }
        if self.pipeline.lang == Lang::Japanese {
            return self.japanese(text);
        }
        if self.pipeline.lang.espeak_voice().is_some() {
            return self.g2p_phonemes(&foreign::translate_cjk_punctuation(text));
        }
        let graphemes = foreign::translate_cjk_punctuation(text);
        let mut tokens: Vec<Token> = Vec::new();
        let mut placeholders = Vec::new();
        let chinese = self.pipeline.foreign.get(&Language::Chinese);
        let segments = foreign::split(&graphemes, |c| chinese.is_none_or(|g2p| g2p.reads(c)));
        let mut i = 0;
        while i < segments.len() {
            let (segment, language) = segments[i];
            i += 1;
            let Some(language) = language else {
                self.english(&mut tokens, segment)?;
                continue;
            };
            let run = if self.reads(language) {
                let ps = self.foreign(segment, language)?;
                let words: Vec<&str> = ps.split(kokoro_text::py::is_space).filter(|w| !w.is_empty()).collect();
                let reference = language == Language::Japanese
                    || matches!(language, Language::Script(s) if foreign::in_foreign_py(s));
                if !words.is_empty() || reference {
                    tokens.extend(words.iter().enumerate().map(|(i, w)| Token {
                        text: if i == 0 { segment.to_owned() } else { String::new() },
                        tag: String::new(),
                        whitespace: if i + 1 < words.len() { " ".to_owned() } else { String::new() },
                        phonemes: Some((*w).to_owned()),
                    }));
                    continue;
                }
                segment.to_owned()
            } else {
                let start = i - 1;
                while let [(gap, None), (_, Some(next)), ..] = segments[i..]
                    && next == language
                    && gap.chars().all(foreign::is_gap)
                {
                    i += 2;
                }
                segments[start..i].iter().map(|(s, _)| *s).collect()
            };
            let start = tokens.len();
            tokens.extend(self.placeholder(language, run)?);
            placeholders.push(start..tokens.len());
        }
        separate(&mut tokens, &placeholders);
        let phonemes = kokoro::tokens_to_ps(&tokens);
        for token in &mut tokens {
            token.phonemes.get_or_insert_with(String::new);
        }
        Ok(Phonemes { phonemes, tokens: Some(tokens), unread: placeholders.len() })
    }

    fn reads(&self, language: Language) -> bool {
        language == Language::English
            || language.espeak().is_some()
            || self.pipeline.foreign.get(&language).is_some_and(|g2p| g2p.available())
    }

    /// Kokoro-FastAPI's `phonemize` for Japanese: [`g2p_phonemes`](Self::g2p_phonemes) on the
    /// text as is (no CJK punctuation mapping), once UniDic is mapped.
    fn japanese(&mut self, text: &str) -> Result<Phonemes, Error> {
        let g2p = self.pipeline.foreign.get(&Language::Japanese).expect("every pipeline has a Japanese G2P");
        if !g2p.available() {
            return Err(Error::Dictionary(
                kokoro_g2p_ja::default_dictionary().err().map_or_else(String::new, |e| e.to_string()),
            ));
        }
        self.g2p_phonemes(text)
    }

    /// The pipeline's own G2P on a segment `ForeignScripts` didn't split out: misaki's Japanese
    /// G2P, or `EspeakG2P` in the pipeline's language, stripped.
    fn native(&mut self, segment: &str) -> Result<String, Error> {
        let pipeline = self.pipeline;
        let raw = match pipeline.lang.espeak_voice() {
            None => {
                let g2p = pipeline.foreign.get(&Language::Japanese).expect("every pipeline has a Japanese G2P");
                g2p.phonemize(segment).map_err(|message| Error::Foreign { language: Language::Japanese, message })?
            }
            Some(voice) => {
                let engine = match &mut self.native {
                    Some(engine) => engine,
                    native => native.insert(EspeakG2P::with_data(voice, pipeline.espeak.clone())?),
                };
                engine.phonemize(segment)?
            }
        };
        Ok(kokoro_text::py::strip(&raw).to_owned())
    }

    /// `KPipeline.g2p_phonemes` for Japanese and the espeak languages (Kokoro-FastAPI's
    /// `phonemize` maps CJK punctuation first for the latter): the text split into runs, where
    /// what `foreign.py` doesn't split out goes to the pipeline's own G2P (Japanese: Han, kana and
    /// the rest; espeak's language: its own script and the rest), Latin runs in another script's
    /// pipeline to misaki's American English G2P, Chinese and Japanese runs to theirs, and the
    /// other scripts to espeak, as in the English pipelines (so the scripts it has no voice for
    /// are "<name> text"). Each run's phonemes are stripped and joined with a space where the
    /// text has one at the join, or two letters meet there. There are no tokens: a chunk's words
    /// are its text's, paired with the phoneme groups ([`text_word_spans`]).
    fn g2p_phonemes(&mut self, text: &str) -> Result<Phonemes, Error> {
        let chinese = self.pipeline.foreign.get(&Language::Chinese);
        let segments = foreign::split_in(text, self.pipeline.lang.native(), |c| chinese.is_none_or(|g2p| g2p.reads(c)));
        let mut ps = String::new();
        let mut previous = "";
        let mut unread = 0;
        let mut after_placeholder = false;
        let mut i = 0;
        while i < segments.len() {
            let (segment, language) = segments[i];
            let start = i;
            i += 1;
            let mut placeholder = false;
            let p = match language {
                None => self.native(segment)?,
                Some(Language::English) => {
                    let text = foreign::prepare(segment, Language::English);
                    let raw = self.english_string(&text)?;
                    foreign::finish(&raw, Language::English)
                }
                Some(language) if self.reads(language) => self.foreign(segment, language)?,
                Some(language) => {
                    while let [(gap, None), (_, Some(next)), ..] = segments[i..]
                        && next == language
                        && gap.chars().all(foreign::is_gap)
                    {
                        i += 2;
                    }
                    unread += 1;
                    placeholder = true;
                    let run: String = segments[start..i].iter().map(|(s, _)| *s).collect();
                    kokoro::tokens_to_ps(&self.placeholder(language, run)?)
                }
            };
            let segment_start = segment;
            let last = segments[i - 1].0;
            let mut boundary = String::new();
            boundary.extend(previous.chars().next_back());
            boundary.extend(segment_start.chars().next());
            let alpha = boundary.chars().count() == 2 && boundary.chars().all(kokoro_g2p_en::is_alpha);
            // A placeholder is kept apart from its neighbours, as in the English pipelines.
            let apart = placeholder || after_placeholder;
            if !ps.is_empty() && !p.is_empty() && (boundary.chars().any(kokoro_text::py::is_space) || alpha || apart) {
                ps.push(' ');
            }
            if !p.is_empty() {
                after_placeholder = placeholder;
            }
            ps.push_str(&p);
            previous = last;
        }
        Ok(Phonemes { phonemes: kokoro_text::py::strip(&ps).to_owned(), tokens: None, unread })
    }

    /// misaki's English `G2P()(text)[0]` (American, with the espeak fallback), as
    /// `ForeignScripts` reads Latin runs in other pipelines.
    fn english_string(&mut self, segment: &str) -> Result<String, Error> {
        let pipeline = self.pipeline;
        let mut tagger = Spacy(pipeline.nlp);
        let mut fallback =
            LazyFallback { data: &pipeline.espeak, british: false, engine: &mut self.fallback, error: None };
        let (ps, _) = pipeline.g2p.phonemize(segment, &mut tagger, Some(&mut fallback))?;
        match fallback.error {
            Some(e) => Err(e.into()),
            None => Ok(ps),
        }
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
            engine.phonemize(&text)?
        };
        Ok(foreign::finish_in(&raw, language, self.pipeline.lang.english()))
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
        let (ids, kept) = vocab::ids(&ps);
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
