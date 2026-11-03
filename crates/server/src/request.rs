//! `POST /v1/audio/speech`'s body, and its checks before any audio is made.
//!
//! Only what Lion Reader and clawed-abode send is accepted (Brendan's decision, 2026-11-03):
//! `model` (ignored), `input`, `voice`, `speed`, `response_format: "pcm"` and `stream: true`.
//! Any other field is refused, so a client never gets something other than it asked for.

use crate::config::Config;
use crate::pipeline::{Lang, voice_lang};
use crate::protocol::{MAX_VOICE_NAME, Speed};
use crate::worker::Connection;
use kokoro_text::{Limits, Options, check_pause_budget, check_speakable};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechRequest {
    /// Accepted and ignored: there is one model.
    #[serde(rename = "model")]
    _model: Option<String>,
    input: String,
    /// One voice from `/v1/audio/voices`; its first letter sets the language.
    voice: Option<String>,
    speed: Option<f64>,
    /// Required, and only `pcm`.
    response_format: Option<String>,
    /// Only `true` is accepted: the response is always streamed.
    stream: Option<bool>,
}

/// A request that passed every check.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub lang: Lang,
    pub text: String,
    pub voice: u16,
    pub speed: Speed,
}

fn invalid<T>(message: impl Into<String>) -> Result<T, String> {
    Err(message.into())
}

/// The checks that need nothing but the request, cheapest first: none of them looks at the text
/// beyond counting its characters.
pub fn check_fields(request: &SpeechRequest, config: &Config) -> Result<(), String> {
    let chars = request.input.chars().count();
    if chars > config.max_input_chars {
        return invalid(format!("input is {chars} characters, the limit is {}", config.max_input_chars));
    }
    match request.response_format.as_deref() {
        Some("pcm") => {}
        Some(other) => return invalid(format!("response_format {other:?} is not supported: only \"pcm\" (16-bit mono 24 kHz) is")),
        None => return invalid("response_format is required, and must be \"pcm\" (16-bit mono 24 kHz)"),
    }
    if request.stream == Some(false) {
        return invalid("stream must be true: the PCM response is always streamed");
    }
    if request.speed.is_some_and(|s| Speed::from_f64(s).is_none()) {
        return invalid(format!("speed must be between {} and {}", Speed::MIN.as_f64(), Speed::MAX.as_f64()));
    }
    if request.voice.as_ref().is_some_and(|v| v.len() > MAX_VOICE_NAME) {
        return invalid(format!("voice is longer than {MAX_VOICE_NAME} bytes"));
    }
    Ok(())
}

/// Resolves the voice and runs the reference's text checks. Call after [`check_fields`].
pub fn plan(request: SpeechRequest, config: &Config, connection: &Connection) -> Result<Speech, String> {
    let name = request.voice.as_deref().unwrap_or(&config.default_voice);
    let Some(voice) = connection.voice_index(name) else { return invalid(format!("Voice '{name}' not found")) };
    let Some(lang) = voice_lang(name) else {
        return invalid(format!(
            "Voice '{name}' speaks a language this server doesn't support: a voice name starts with its language, one of a (American English), b (British English), e (Spanish), f (French), h (Hindi), i (Italian), j (Japanese), p (Brazilian Portuguese) or z (Chinese)"
        ));
    };
    let speed = request.speed.map_or(Some(Speed::NORMAL), Speed::from_f64).expect("checked by check_fields");
    check_pause_budget(&request.input, &Limits::default()).map_err(|e| e.to_string())?;
    check_speakable(&request.input, &Options::default()).map_err(|e| e.to_string())?;
    Ok(Speech { lang, text: request.input, voice, speed })
}
