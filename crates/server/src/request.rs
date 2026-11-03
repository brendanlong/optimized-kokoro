//! `POST /v1/audio/speech`'s body, and its checks before any audio is made.

use crate::config::Config;
use crate::pipeline::Pipeline;
use crate::voices::{self, Mix};
use crate::worker::Connection;
use kokoro_text::tags::{RATE_MAX, RATE_MIN, clamp_rate};
use kokoro_text::{Limits, Options, check_pause_budget, check_speakable, split_by_voice};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

/// The request body. Fields Kokoro-FastAPI accepts that this server doesn't support are left out,
/// so a request using them is refused rather than silently handled differently.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechRequest {
    /// Accepted and ignored: there is one model.
    #[serde(rename = "model")]
    _model: Option<String>,
    input: String,
    voice: Option<String>,
    response_format: Option<String>,
    speed: Option<f64>,
    /// Only `true` is accepted: the response is always streamed.
    stream: Option<bool>,
    lang_code: Option<String>,
    volume_multiplier: Option<f64>,
    normalization_options: Option<NormalizationOptions>,
    allow_voice_tags: Option<bool>,
    /// Only `false` is accepted: SSML was removed (`docs/divergences.md`).
    ssml: Option<bool>,
    /// OpenAI's style prompt for its own models; accepted and ignored, as Kokoro has no such input.
    #[serde(rename = "instructions")]
    _instructions: Option<String>,
    /// OpenAI's choice between raw audio and server-sent events; only `audio` is supported.
    stream_format: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NormalizationOptions {
    normalize: Option<bool>,
    unit_normalization: Option<bool>,
    url_normalization: Option<bool>,
    email_normalization: Option<bool>,
    optional_pluralization_normalization: Option<bool>,
    phone_normalization: Option<bool>,
    caps_normalization: Option<bool>,
    replace_remaining_symbols: Option<bool>,
    remove_emoji: Option<bool>,
}

impl NormalizationOptions {
    fn resolve(self) -> Options {
        let d = Options::default();
        Options {
            normalize: self.normalize.unwrap_or(d.normalize),
            unit_normalization: self.unit_normalization.unwrap_or(d.unit_normalization),
            url_normalization: self.url_normalization.unwrap_or(d.url_normalization),
            email_normalization: self.email_normalization.unwrap_or(d.email_normalization),
            optional_pluralization_normalization: self
                .optional_pluralization_normalization
                .unwrap_or(d.optional_pluralization_normalization),
            phone_normalization: self.phone_normalization.unwrap_or(d.phone_normalization),
            caps_normalization: self.caps_normalization.unwrap_or(d.caps_normalization),
            replace_remaining_symbols: self.replace_remaining_symbols.unwrap_or(d.replace_remaining_symbols),
            remove_emoji: self.remove_emoji.unwrap_or(d.remove_emoji),
        }
    }
}

/// A run of text in one voice and pace.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub lang: String,
    pub text: String,
    pub voices: Arc<[(u16, f32)]>,
    pub speed: f64,
}

/// A request that passed every check.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub volume: f64,
    pub options: Options,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Invalid(String),
    Forbidden(String),
}

fn invalid<T>(message: impl Into<String>) -> Result<T, Refusal> {
    Err(Refusal::Invalid(message.into()))
}

const VOLUME_MAX: f64 = 10.0;

/// The checks that need nothing but the request, cheapest first: none of them looks at the text
/// beyond counting its characters.
pub fn check_fields(request: &SpeechRequest, config: &Config) -> Result<(), Refusal> {
    let chars = request.input.chars().count();
    if chars > config.max_input_chars {
        return invalid(format!("input is {chars} characters, the limit is {}", config.max_input_chars));
    }
    match request.response_format.as_deref() {
        Some("pcm") => {}
        Some(other) => return invalid(format!("response_format {other:?} is not supported: only \"pcm\" (16-bit mono 24 kHz) is")),
        None => return invalid("response_format is required, and must be \"pcm\" (16-bit mono 24 kHz)"),
    }
    if request.stream_format.as_deref().is_some_and(|f| f != "audio") {
        return invalid("stream_format must be \"audio\"");
    }
    if request.stream == Some(false) {
        return invalid("stream must be true: the PCM response is always streamed");
    }
    if request.ssml == Some(true) {
        return invalid("SSML is not supported");
    }
    if request.speed.is_some_and(|s| !(RATE_MIN..=RATE_MAX).contains(&s)) {
        return invalid(format!("speed must be between {RATE_MIN} and {RATE_MAX}"));
    }
    if request.volume_multiplier.is_some_and(|v| !(0.0..=VOLUME_MAX).contains(&v)) {
        return invalid(format!("volume_multiplier must be between 0 and {VOLUME_MAX}"));
    }
    if request.voice.as_ref().is_some_and(|v| v.len() > voices::MAX_EXPRESSION) {
        return invalid(format!("voice is longer than {} bytes", voices::MAX_EXPRESSION));
    }
    if request.allow_voice_tags == Some(true) && !config.enable_voice_tags {
        return Err(Refusal::Forbidden("Voice tags are disabled on this server".into()));
    }
    Ok(())
}

/// Resolves voices and languages and runs the reference's text checks. Call after
/// [`check_fields`].
pub fn plan(
    request: SpeechRequest,
    config: &Config,
    connection: &Connection,
    pipeline: &dyn Pipeline,
) -> Result<Speech, Refusal> {
    let allow_voice_tags = request.allow_voice_tags.unwrap_or(false);
    let options = request.normalization_options.map_or_else(Options::default, NormalizationOptions::resolve);
    let speed = request.speed.unwrap_or(1.0);
    if let Some(lang) = &request.lang_code
        && !pipeline.supports(lang)
    {
        return invalid(format!("unsupported lang_code {lang:?}"));
    }

    let voice = request.voice.as_deref().unwrap_or(&config.default_voice);
    // Each distinct expression is resolved once; segments share the resolved ids.
    let mut mixes: HashMap<Arc<str>, Mix> = HashMap::new();
    let mut resolve = |expression: &Arc<str>| -> Result<Mix, Refusal> {
        if expression.len() > voices::MAX_EXPRESSION {
            return invalid(format!("a voice is longer than {} bytes", voices::MAX_EXPRESSION));
        }
        if let Some(mix) = mixes.get(expression) {
            return Ok(mix.clone());
        }
        let mix = voices::resolve(expression, connection).map_err(Refusal::Invalid)?;
        mixes.insert(expression.clone(), mix.clone());
        Ok(mix)
    };

    let runs = if allow_voice_tags {
        split_by_voice(&request.input, voice).into_iter().map(|s| (s.voice, s.rate, s.text)).collect()
    } else {
        vec![(Arc::from(voice), 1.0, request.input.clone())]
    };
    resolve(&Arc::from(voice))?;
    let mut segments = Vec::with_capacity(runs.len());
    for (expression, rate, text) in runs {
        let mix = resolve(&expression)?;
        let lang = request.lang_code.clone().unwrap_or(mix.lang);
        if !pipeline.supports(&lang) {
            return invalid(format!("voice {expression:?} implies language {lang:?}, which has no pipeline"));
        }
        segments.push(Segment { lang, text, voices: mix.voices, speed: clamp_rate(speed * rate) });
    }

    check_pause_budget(&request.input, &Limits::default()).map_err(|e| Refusal::Invalid(e.to_string()))?;
    check_speakable(&request.input, allow_voice_tags, Some(&options)).map_err(|e| Refusal::Invalid(e.to_string()))?;

    Ok(Speech {
        volume: request.volume_multiplier.unwrap_or(1.0),
        options,
        segments,
    })
}
