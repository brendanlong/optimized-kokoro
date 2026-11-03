//! Chinese: misaki's legacy Chinese G2P ([`kokoro_g2p_zh`]) as a [`ForeignG2p`] for Han runs in the
//! other pipelines, and kokoro's `KPipeline.g2p_phonemes` for the Chinese pipeline (`lang` z).

use crate::foreign::{self, Language};
use crate::{Error, ForeignG2p, Phonemes, Request};
use kokoro_g2p_zh::ZhG2p;
use kokoro_text::py;
use std::sync::{Arc, LazyLock};

/// The dictionaries, loaded once per process on first use and shared by every pipeline.
pub fn shared() -> Arc<ZhG2p> {
    static G2P: LazyLock<Arc<ZhG2p>> = LazyLock::new(|| Arc::new(ZhG2p::new()));
    G2P.clone()
}

/// `ZHG2P(version=None)` for runs of Han characters.
pub struct Chinese(pub Arc<ZhG2p>);

impl ForeignG2p for Chinese {
    fn phonemize(&self, text: &str) -> Result<String, String> {
        self.0.phonemize(text).map_err(|e| e.to_string())
    }

    fn reads(&self, c: char) -> bool {
        self.0.reads(c)
    }
}

impl Request<'_> {
    /// Kokoro-FastAPI's `phonemize` in the Chinese pipeline: `g2p_phonemes(text).strip()`, with
    /// no punctuation mapping (`ZHG2P` does its own). Han runs, and whatever isn't a run of another
    /// script, go to the Chinese G2P; Latin runs to misaki's English G2P; kana runs to the
    /// Japanese G2P; other scripts to espeak as in the English pipelines. Each segment's phonemes
    /// are stripped and joined with a space where the text has whitespace at the join, or two
    /// letters meet there.
    pub(crate) fn chinese(&mut self, text: &str) -> Result<Phonemes, Error> {
        let mut ps = String::new();
        let mut previous = "";
        let mut unread = 0;
        for (segment, language) in foreign::split_chinese(text) {
            let p = match language {
                None => self.native_chinese(segment)?,
                Some(Language::English) => {
                    let mut tokens = Vec::new();
                    self.english(&mut tokens, segment)?;
                    let joined: String = tokens
                        .iter()
                        .map(|t| format!("{}{}", t.phonemes.as_deref().unwrap_or(""), t.whitespace))
                        .collect();
                    foreign::finish(&joined, Language::English)
                }
                Some(language) if self.reads(language) => self.foreign(segment, language)?,
                Some(language) => {
                    unread += 1;
                    let mut tokens = Vec::new();
                    self.english(&mut tokens, &format!("{} text", language.name()))?;
                    kokoro_g2p_en::kokoro::tokens_to_ps(&tokens)
                }
            };
            let p = py::strip(&p);
            let boundary: Vec<char> = previous.chars().next_back().into_iter().chain(segment.chars().next()).collect();
            if !ps.is_empty()
                && !p.is_empty()
                && (boundary.iter().any(|&c| py::is_space(c))
                    || (boundary.len() == 2 && boundary.iter().all(|&c| kokoro_g2p_en::is_alpha(c))))
            {
                ps.push(' ');
            }
            ps.push_str(p);
            previous = segment;
        }
        Ok(Phonemes { phonemes: py::strip(&ps).to_owned(), tokens: Vec::new(), unread })
    }

    fn native_chinese(&self, segment: &str) -> Result<String, Error> {
        let g2p = self.pipeline.foreign.get(&Language::Chinese).expect("the Chinese pipeline has its G2P");
        g2p.phonemize(segment).map_err(|message| Error::Foreign { language: Language::Chinese, message })
    }
}
