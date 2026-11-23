use kokoro_espeak::EspeakG2P;
use kokoro_pipeline::Lang;
use kokoro_pipeline::foreign::{espeak_voice, script};
use std::collections::HashMap;

/// The espeak voices pipelines read with, made as needed.
#[derive(Default)]
pub struct Voices(HashMap<&'static str, EspeakG2P>);

impl Voices {
    /// Whether espeak, reading `text` in the `lang` pipeline, changes its letters first: composes
    /// a letter with its combining marks, or takes the accents off one the voice reading it has
    /// no name for. The reference's espeak read those letters as written (`docs/divergences.md`,
    /// "espeak: letters it lacks").
    pub fn change_letters(&mut self, lang: Lang, text: &str) -> bool {
        if kokoro_espeak::compose_marks(text) != text {
            return true;
        }
        text.chars().filter(|c| !c.is_ascii()).any(|c| {
            let voices = match script(c).and_then(espeak_voice) {
                Some(voice) => vec![voice],
                // Latin and the pipeline's own script: English, or the pipeline's voice.
                None => ["en-us", "en-gb"].into_iter().chain(lang.espeak_voice().filter(|&v| v != "en")).collect(),
            };
            voices.into_iter().any(|v| {
                let g2p = self.0.entry(v).or_insert_with(|| EspeakG2P::new(v).unwrap());
                !g2p.backend().reads_as_written(&c.to_string())
            })
        })
    }
}
