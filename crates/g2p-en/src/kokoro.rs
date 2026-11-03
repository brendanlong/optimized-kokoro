//! What kokoro's `KPipeline` (and Kokoro-FastAPI's `phonemize`) do around misaki for English text.

use crate::{Error, Fallback, G2p, Tagger, Token, text};

/// The English part of `KPipeline.en_g2p_tokens`: appends the tokens for one segment of the pipeline's
/// own language. Leading whitespace goes onto the previous token, as the segment's G2P never sees it.
pub fn append_segment(
    g2p: &G2p,
    tokens: &mut Vec<Token>,
    segment: &str,
    tagger: &mut dyn Tagger,
    fallback: Option<&mut dyn Fallback>,
) -> Result<(), Error> {
    let stripped = text::lstrip(segment);
    if let Some(last) = tokens.last_mut() {
        last.whitespace.push_str(&segment[..segment.len() - stripped.len()]);
    }
    if !stripped.is_empty() {
        tokens.extend(g2p.phonemize(stripped, tagger, fallback)?.1);
    }
    Ok(())
}

/// `KPipeline.tokens_to_ps`, with Kokoro-FastAPI's `None` phonemes read as empty.
pub fn tokens_to_ps(tokens: &[Token]) -> String {
    let mut ps = String::new();
    for t in tokens {
        ps.push_str(t.phonemes.as_deref().unwrap_or(""));
        if !t.whitespace.is_empty() {
            ps.push(' ');
        }
    }
    text::strip(&ps).to_string()
}
