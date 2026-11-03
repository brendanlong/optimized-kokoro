use kokoro_g2p_en::{Error, G2p, TaggedToken, Tagger};

struct OneToken(&'static str);

impl Tagger for OneToken {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken> {
        vec![TaggedToken { text: text.to_string(), tag: self.0.to_string(), whitespace: String::new() }]
    }
}

#[test]
fn numbers_num2words_cannot_name_are_errors() {
    let g2p = G2p::new(false);
    let huge = "9".repeat(400);
    assert!(matches!(g2p.phonemize(&huge, &mut OneToken("CD"), None), Err(Error::Number(_))));
    let (ps, _) = g2p.phonemize(&"9".repeat(306), &mut OneToken("CD"), None).unwrap();
    assert!(!ps.is_empty());
}

#[test]
fn unknown_words_without_fallback_have_no_phonemes() {
    let g2p = G2p::new(false).with_unk("?");
    let (ps, tokens) = g2p.phonemize("Qzxjv", &mut OneToken("NNP"), None).unwrap();
    assert_eq!(ps, "?");
    assert_eq!(tokens[0].phonemes, None);
}
