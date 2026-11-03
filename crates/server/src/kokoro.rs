//! [`Pipeline`] backed by `kokoro-pipeline`: one loaded pipeline per supported language.

use crate::pipeline::{Chunk, Pipeline, PipelineError};
use kokoro_pipeline::{Error, Lang};
use kokoro_text::Options;

pub struct KokoroPipelines {
    american: kokoro_pipeline::Pipeline,
    british: kokoro_pipeline::Pipeline,
}

impl KokoroPipelines {
    pub fn load() -> Result<Self, Error> {
        Ok(Self {
            american: kokoro_pipeline::Pipeline::new(Lang::AmericanEnglish)?,
            british: kokoro_pipeline::Pipeline::new(Lang::BritishEnglish)?,
        })
    }

    fn get(&self, lang: &str) -> Option<&kokoro_pipeline::Pipeline> {
        match lang {
            "a" | "en-us" => Some(&self.american),
            "b" | "en-gb" => Some(&self.british),
            _ => None,
        }
    }
}

impl Pipeline for KokoroPipelines {
    fn supports(&self, lang: &str) -> bool {
        self.get(lang).is_some()
    }

    fn chunks<'a>(
        &'a self,
        lang: &'a str,
        text: &'a str,
        options: &'a Options,
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        let Some(pipeline) = self.get(lang) else {
            return Box::new(std::iter::once(Err(PipelineError::Failed(format!("no pipeline for {lang:?}")))));
        };
        Box::new(pipeline.chunks(text, options).map(|chunk| match chunk {
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
        assert!(pipelines.supports("a") && pipelines.supports("en-gb") && !pipelines.supports("j"));
        let options = Options::default();
        let chunks: Vec<_> = pipelines.chunks("a", "Hello there. [pause:0.5s] 東京 is big.", &options).collect();
        let first = chunks[0].as_ref().unwrap();
        assert_eq!(first.text, "Hello there.");
        assert!(!first.ids.is_empty() && first.ids.len() == first.phonemes.chars().count());
        assert_eq!(chunks[1].as_ref().unwrap().pause, Some(0.5));
        assert!(matches!(chunks[2], Err(PipelineError::Unsupported(_))), "{:?}", chunks[2]);
    }
}
