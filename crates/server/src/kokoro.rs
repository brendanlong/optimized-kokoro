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
        Box::new(pipeline.chunks(text, &self.options).map(|chunk| {
            chunk
                .map(|c| Chunk { text: c.text, phonemes: c.phonemes, ids: c.ids, pause: c.pause, unread: c.unread })
                .map_err(|e| PipelineError(e.to_string()))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_english_and_names_what_it_cant_read() {
        let pipelines = KokoroPipelines::load().unwrap();
        let chunks: Vec<_> =
            pipelines.chunks(Lang::AmericanEnglish, "Hello there. [pause:0.5s] 東京 is big.").map(Result::unwrap).collect();
        assert_eq!(chunks[0].text, "Hello there.");
        assert!(!chunks[0].ids.is_empty() && chunks[0].ids.len() == chunks[0].phonemes.chars().count());
        assert_eq!(chunks[1].pause, Some(0.5));
        assert_eq!((chunks[2].phonemes.as_str(), chunks[2].unread), ("ʧˌInˈiz tˈɛkst ɪz bˈɪɡ.", 1));
    }

    #[test]
    fn british_voices_get_british_phonemes() {
        let pipelines = KokoroPipelines::load().unwrap();
        let phonemes = |lang| pipelines.chunks(lang, "tomato").next().unwrap().unwrap().phonemes;
        assert_ne!(phonemes(Lang::AmericanEnglish), phonemes(Lang::BritishEnglish));
    }
}
