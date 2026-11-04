use kokoro_g2p_en::{Error, G2p, TaggedToken, Tagger};

struct OneToken(&'static str);

impl Tagger for OneToken {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken> {
        vec![TaggedToken { text: text.to_string(), tag: self.0.to_string(), whitespace: String::new() }]
    }
}

/// misaki's number reading is gone (the text normalizer reads numbers): digits have no reading
/// here, and no number is too large to read.
#[test]
fn numbers_have_no_reading() {
    let g2p = G2p::new(false).with_unk("?");
    let (ps, tokens) = g2p.phonemize(&"9".repeat(400), &mut OneToken("CD"), None).unwrap();
    assert_eq!(ps, "?");
    assert_eq!(tokens[0].phonemes, None);
}

#[test]
fn empty_tokens_are_errors() {
    struct Empty;
    impl Tagger for Empty {
        fn tag(&mut self, _: &str) -> Vec<TaggedToken> {
            vec![TaggedToken { text: String::new(), tag: "NN".to_string(), whitespace: String::new() }]
        }
    }
    assert_eq!(G2p::new(false).phonemize("x", &mut Empty, None), Err(Error::EmptyToken));
}

#[test]
fn unknown_words_without_fallback_have_no_phonemes() {
    let g2p = G2p::new(false).with_unk("?");
    let (ps, tokens) = g2p.phonemize("Qzxjv", &mut OneToken("NNP"), None).unwrap();
    assert_eq!(ps, "?");
    assert_eq!(tokens[0].phonemes, None);
}

/// Compounds of names or acronyms get a word boundary at their hyphens; other compounds don't.
#[test]
fn hyphen_boundaries() {
    use kokoro_g2p_en::hyphen_boundary;
    for yes in ["US-China", "NATO-EU", "nato-EU", "US-based", "EU-wide", "US-UK"] {
        assert!(hyphen_boundary(yes), "{yes}");
    }
    let no = [
        "well-crafted",
        "X-ray",
        "T-shirt",
        "A-list",
        "co-founder",
        "state-of-the-art",
        "US",
        "Paris-Berlin",
        "Smith-Jones",
        "Spider-Man",
        "Mary-Jane",
        "Anti-American",
        "Pre-Columbian",
        "Mid-Atlantic",
        "Mid-July",
        "Non-EU",
        "Pro-EU",
        "anti-US",
        "Ex-President",
        "Long-Term",
        "Co-Op",
        "No-No",
        "The-End",
        "COVID-nineteen",
        "UTF-eight",
        "Windows-ninety-five",
        "MP-three-player",
        "ISO-eight",
    ];
    for no in no {
        assert!(!hyphen_boundary(no), "{no}");
    }
}
