//! The Chinese G2P loads on the first Han character, not with the pipeline. Its own test binary,
//! so no other test has loaded it first.

use kokoro_pipeline::{Lang, Pipeline, chinese_g2p_loaded};

#[test]
fn english_text_never_loads_the_chinese_g2p() {
    let pipeline = Pipeline::new(Lang::AmericanEnglish).unwrap();
    pipeline.phonemize("Hello there, Привет, カナ and ー.").unwrap();
    assert!(!chinese_g2p_loaded());
    assert_eq!(pipeline.phonemize("He saw 北京.").unwrap().0, "hˌi sˈɔ bˈeiʨˈiŋ.");
    assert!(chinese_g2p_loaded());
}
