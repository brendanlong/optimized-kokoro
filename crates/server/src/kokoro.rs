//! [`Pipeline`] backed by `kokoro-pipeline`: one loaded pipeline per supported language.

use crate::pipeline::{Chunk, Lang, Pipeline, PipelineError};
use kokoro_pipeline::Error;
use kokoro_text::Options;

pub struct KokoroPipelines {
    american: kokoro_pipeline::Pipeline,
    british: kokoro_pipeline::Pipeline,
    options: Options,
}

impl KokoroPipelines {
    pub fn load() -> Result<Self, Error> {
        Ok(Self {
            american: kokoro_pipeline::Pipeline::new(Lang::AmericanEnglish)?,
            british: kokoro_pipeline::Pipeline::new(Lang::BritishEnglish)?,
            options: Options::default(),
        })
    }
}

impl Pipeline for KokoroPipelines {
    fn chunks<'a>(&'a self, lang: Lang, text: &'a str) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        let pipeline = match lang {
            Lang::AmericanEnglish => &self.american,
            Lang::BritishEnglish => &self.british,
        };
        Box::new(pipeline.chunks(text, &self.options).map(|chunk| match chunk {
            Ok(c) => Ok(Chunk { text: c.text, phonemes: c.phonemes, ids: c.ids, pause: c.pause }),
            Err(e @ Error::Unsupported { .. }) => Err(PipelineError::Unsupported(e.to_string())),
            Err(e) => Err(PipelineError::Failed(e.to_string())),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_english_and_skips_what_it_cant() {
        let pipelines = KokoroPipelines::load().unwrap();
        let chunks: Vec<_> = pipelines.chunks(Lang::AmericanEnglish, "Hello there. [pause:0.5s] 東京 is big.").collect();
        let first = chunks[0].as_ref().unwrap();
        assert_eq!(first.text, "Hello there.");
        assert!(!first.ids.is_empty() && first.ids.len() == first.phonemes.chars().count());
        assert_eq!(chunks[1].as_ref().unwrap().pause, Some(0.5));
        assert!(matches!(chunks[2], Err(PipelineError::Unsupported(_))), "{:?}", chunks[2]);
    }

    #[test]
    fn british_voices_get_british_phonemes() {
        let pipelines = KokoroPipelines::load().unwrap();
        let phonemes = |lang| pipelines.chunks(lang, "tomato").next().unwrap().unwrap().phonemes;
        assert_ne!(phonemes(Lang::AmericanEnglish), phonemes(Lang::BritishEnglish));
    }
}
