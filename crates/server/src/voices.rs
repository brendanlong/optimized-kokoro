//! Voice expressions in Kokoro-FastAPI's syntax (`process_and_validate_voices`): a voice name, or
//! a blend like `af_bella(2)+af_sky`, with OpenAI's voice names mapped to Kokoro voices.

use crate::protocol::MAX_VOICES_PER_MIX;
use crate::worker::Connection;
use kokoro_text::py;
use std::sync::Arc;

/// `core/openai_mappings.json`.
const OPENAI_VOICES: &[(&str, &str)] = &[
    ("alloy", "am_adam"),
    ("ash", "af_nicole"),
    ("coral", "bf_emma"),
    ("echo", "af_bella"),
    ("fable", "af_sarah"),
    ("onyx", "bm_george"),
    ("nova", "bf_isabella"),
    ("sage", "am_michael"),
    ("shimmer", "af_sky"),
    ("marin", "af_heart"),
    ("cedar", "am_fenrir"),
    ("verse", "am_puck"),
    ("ballad", "bm_fable"),
];

/// Longest voice expression accepted, in bytes: room for 8 voices with long weights.
pub const MAX_EXPRESSION: usize = 512;

/// A resolved voice blend.
#[derive(Debug, Clone, PartialEq)]
pub struct Mix {
    /// Voice indices and weights, as the worker takes them. The worker normalizes the weights.
    pub voices: Arc<[(u16, f32)]>,
    /// The language a voice implies: its name's first letter.
    pub lang: String,
}

/// Parses a positive, finite weight. Python's `float()` also takes `_` digit separators; this
/// doesn't.
fn parse_weight(text: &str) -> Option<f64> {
    let w: f64 = py::strip(text).parse().ok()?;
    (w.is_finite() && w > 0.0).then_some(w)
}

pub fn resolve(expression: &str, connection: &Connection) -> Result<Mix, String> {
    let compact: String = expression.chars().filter(|&c| c != ' ').collect();
    let compact = py::strip(&compact);
    if compact.is_empty() {
        return Err("Voice name cannot be empty".into());
    }
    if compact.starts_with(['+', '-']) || compact.ends_with(['+', '-']) || compact.contains("++") || compact.contains("+-")
        || compact.contains("-+") || compact.contains("--")
    {
        return Err("Voice combination contains empty combine items".into());
    }
    if compact.contains('-') {
        return Err("Subtracting voices (a-b) is not supported".into());
    }
    let items: Vec<&str> = compact.split('+').collect();
    if items.len() > MAX_VOICES_PER_MIX {
        return Err(format!("A voice blend may combine at most {MAX_VOICES_PER_MIX} voices"));
    }
    let mut voices = Vec::with_capacity(items.len());
    let mut lang = String::new();
    for item in items {
        let invalid = || format!("Voice '{item}' is not a valid voice or voice(weight)");
        let (name, weight) = match item.split_once('(') {
            None if !item.contains(')') => (item, 1.0),
            Some((name, rest)) => {
                let weight = rest.strip_suffix(')').filter(|w| !w.contains(['(', ')'])).ok_or_else(invalid)?;
                if name.is_empty() || name.contains(')') {
                    return Err(invalid());
                }
                (name, parse_weight(weight).ok_or_else(|| format!("Voice '{item}' must use a positive weight"))?)
            }
            None => return Err(invalid()),
        };
        let name = py::strip(name);
        let name = OPENAI_VOICES.iter().find(|(openai, _)| *openai == name).map_or(name, |(_, kokoro)| kokoro);
        let index = connection.voice_index(name).ok_or_else(|| format!("Voice '{name}' not found"))?;
        let weight = weight as f32;
        if !(weight.is_finite() && weight > 0.0) {
            return Err(format!("Voice '{item}' has a weight out of range"));
        }
        if lang.is_empty() {
            lang = name.chars().next().expect("voice names aren't empty").to_ascii_lowercase().to_string();
        }
        voices.push((index, weight));
    }
    Ok(Mix { voices: voices.into(), lang })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Hello;
    use tokio::sync::mpsc;

    fn connection() -> Connection {
        let hello = Hello {
            vocab_size: 178,
            max_phonemes: 510,
            max_batch: 8,
            max_chunk_samples: 2_880_000,
            voices: ["af_heart", "af_bella", "af_sky", "am_adam", "bf_emma"].map(String::from).to_vec(),
        };
        Connection::new(hello, mpsc::channel(1).0, 1, crate::worker::AudioBudget::new(1))
    }

    fn ok(expr: &str) -> (Vec<(u16, f32)>, String) {
        let mix = resolve(expr, &connection()).unwrap_or_else(|e| panic!("{expr}: {e}"));
        (mix.voices.to_vec(), mix.lang)
    }

    #[test]
    fn resolves_names_blends_and_openai_names() {
        assert_eq!(ok("af_heart"), (vec![(0, 1.0)], "a".into()));
        assert_eq!(ok("af_bella(2)+af_sky"), (vec![(1, 2.0), (2, 1.0)], "a".into()));
        assert_eq!(ok(" bf_emma ( 0.5 ) + af_heart"), (vec![(4, 0.5), (0, 1.0)], "b".into()));
        assert_eq!(ok("alloy"), (vec![(3, 1.0)], "a".into()));
        assert_eq!(ok("coral+alloy(3)"), (vec![(4, 1.0), (3, 3.0)], "b".into()));
    }

    #[test]
    fn rejects_bad_expressions() {
        for expr in [
            "", "  ", "+af_heart", "af_heart+", "af_heart++af_sky", "af_heart-af_sky", "af_heart(0)", "af_heart(-1)",
            "af_heart(x)", "af_heart(inf)", "af_heart(nan)", "af_heart(1e300)", "af_heart(1", "af_heart)", "(1)",
            "af_heart((1))", "nobody", "af_heart(1)x",
            "af_heart+af_heart+af_heart+af_heart+af_heart+af_heart+af_heart+af_heart+af_heart",
        ] {
            assert!(resolve(expr, &connection()).is_err(), "{expr:?}");
        }
    }
}
