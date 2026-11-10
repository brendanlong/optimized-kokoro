//! [`Pipeline`] backed by `kokoro-pipeline`: one loaded pipeline per supported language.

use crate::pipeline::{Chunk, DEFAULT_MAX_TOKENS, Lang, Pipeline, PipelineError};
use kokoro_pipeline::Error;
use kokoro_text::Options;

pub struct KokoroPipelines {
    /// One per [`Lang::ALL`]. They share spaCy, misaki's lexicons and espeak's data.
    pipelines: Vec<kokoro_pipeline::Pipeline>,
    options: Options,
}

impl KokoroPipelines {
    /// Loads every pipeline, then reads one English sentence, so the statics built on first use
    /// (the normalizer's patterns, about 20 ms) are built now rather than in the first request.
    pub fn load() -> Result<Self, Error> {
        let pipelines = Lang::ALL.into_iter().map(kokoro_pipeline::Pipeline::new).collect::<Result<_, _>>()?;
        let pipelines = Self { pipelines, options: Options::default() };
        pipelines
            .chunks(Lang::AmericanEnglish, "Warm up at 3:30.", DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| false)
            .for_each(drop);
        Ok(pipelines)
    }
}

impl Pipeline for KokoroPipelines {
    fn chunks<'a>(
        &'a self,
        lang: Lang,
        text: &'a str,
        max_phonemes: usize,
        clause_groups: usize,
        cancelled: &'a (dyn Fn() -> bool + Sync),
    ) -> Box<dyn Iterator<Item = Result<Chunk, PipelineError>> + 'a> {
        let pipeline = self.pipelines.iter().find(|p| p.lang() == lang).expect("a pipeline for every language");
        Box::new(pipeline.chunks_within_groups(text, &self.options, max_phonemes, clause_groups, cancelled).map(
            |chunk| {
                chunk
                    .map(|c| Chunk {
                        text: c.text,
                        phonemes: c.phonemes,
                        ids: c.ids,
                        pause: c.pause,
                        unread: c.unread,
                        split: c.split,
                    })
                    .map_err(|e| PipelineError(e.to_string()))
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_english_and_names_what_it_cant_read() {
        let pipelines = KokoroPipelines::load().unwrap();
        let chunks: Vec<_> = pipelines
            .chunks(
                Lang::AmericanEnglish,
                "Hello there. [pause:0.5s] 東京 is big. カナ too.",
                DEFAULT_MAX_TOKENS,
                DEFAULT_MAX_TOKENS,
                &|| false,
            )
            .map(Result::unwrap)
            .collect();
        assert_eq!(chunks[0].text, "Hello there.");
        assert!(!chunks[0].ids.is_empty() && chunks[0].ids.len() == chunks[0].phonemes.chars().count());
        assert_eq!(chunks[1].pause, Some(0.5));
        assert_eq!((chunks[2].phonemes.as_str(), chunks[2].unread), ("dˈʊŋʨˈiŋ ɪz bˈɪɡ.", 0));
        let kana = if kokoro_pipeline::japanese_available() {
            ("kana tˈu.", 0)
        } else {
            ("ʤˌæpənˈiz tˈɛkst tˈu.", 1)
        };
        assert_eq!((chunks[3].phonemes.as_str(), chunks[3].unread), kana);
    }

    #[test]
    fn chinese_voices_get_chinese_phonemes() {
        let pipelines = KokoroPipelines::load().unwrap();
        let chunks: Vec<_> = pipelines
            .chunks(Lang::Chinese, "银行行长在长城上。", DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| false)
            .map(Result::unwrap)
            .collect();
        assert_eq!(chunks[0].phonemes, "i↗nxa↗ŋxa↗ŋꭧa↓ŋ ʦai↘ ꭧʰa↗ŋꭧʰə↗ŋ ʂa↘ŋ.");
        assert_eq!(chunks[0].ids.len(), chunks[0].phonemes.chars().count());
    }

    #[test]
    fn reads_japanese() {
        if !kokoro_pipeline::japanese_available() {
            return kokoro_pipeline::skip_without_dictionary("no UniDic at $KOKORO_UNIDIC");
        }
        let pipelines = KokoroPipelines::load().unwrap();
        let chunk = pipelines
            .chunks(Lang::Japanese, "東京へ行きます。", DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| false)
            .next()
            .unwrap()
            .unwrap();
        assert_eq!((chunk.phonemes.as_str(), chunk.unread), ("toːkʲoː e ikʲimasɨ", 0));
        let chunk = pipelines
            .chunks(Lang::AmericanEnglish, "She said こんにちは.", DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| {
                false
            })
            .next()
            .unwrap()
            .unwrap();
        assert_eq!((chunk.phonemes.as_str(), chunk.unread), ("ʃˌi sˈɛd koɲɲiʨiβa.", 0));
    }

    #[test]
    fn espeak_voices_get_their_languages_phonemes() {
        let pipelines = KokoroPipelines::load().unwrap();
        for (lang, text, phonemes) in [
            (Lang::Spanish, "Hola, mundo.", "ˈola, mˈundo."),
            (Lang::French, "Bonjour le monde.", "bɔ̃ʒˈuʁ lə mˈɔ̃d."),
            (Lang::Hindi, "नमस्ते दुनिया, hello.", "nəmˈʌsteː dˈʊnɪjˌaː, həlˈO."),
            (Lang::Italian, "Ciao mondo.", "ʧˈao mˈondo."),
            (Lang::BrazilianPortuguese, "Olá, mundo.", "olˈa, mˈu\u{303}ŋdʊ."),
        ] {
            let chunk = pipelines
                .chunks(lang, text, DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| false)
                .next()
                .unwrap()
                .unwrap();
            assert_eq!((chunk.phonemes.as_str(), chunk.unread), (phonemes, 0), "{lang:?}");
            assert_eq!(chunk.ids.len(), chunk.phonemes.chars().count(), "{lang:?}");
        }
    }

    #[test]
    fn british_voices_get_british_phonemes() {
        let pipelines = KokoroPipelines::load().unwrap();
        let phonemes = |lang| {
            pipelines
                .chunks(lang, "tomato", DEFAULT_MAX_TOKENS, DEFAULT_MAX_TOKENS, &|| false)
                .next()
                .unwrap()
                .unwrap()
                .phonemes
        };
        assert_ne!(phonemes(Lang::AmericanEnglish), phonemes(Lang::BritishEnglish));
    }
}
