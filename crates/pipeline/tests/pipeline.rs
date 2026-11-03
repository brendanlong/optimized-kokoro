//! kokoro's `tests/test_foreign.py` for the English pipelines, and the chunk API.

use kokoro_espeak::{EspeakFallback, EspeakG2P};
use kokoro_g2p_en::{Fallback, G2p, TaggedToken, Tagger, kokoro};
use kokoro_pipeline::foreign::Language;
use kokoro_pipeline::{Chunk, Error, ForeignG2p, Lang, NormalizationOptions, Pipeline};
use kokoro_spacy_en::Nlp;
use std::sync::OnceLock;

fn pipeline() -> &'static Pipeline {
    static P: OnceLock<Pipeline> = OnceLock::new();
    P.get_or_init(|| Pipeline::new(Lang::AmericanEnglish).unwrap())
}

fn phonemize(text: &str) -> String {
    pipeline().phonemize(text).unwrap().0
}

fn espeak(language: &str, text: &str) -> String {
    EspeakG2P::new(language).unwrap().phonemize(text).unwrap()
}

struct Spacy(Nlp);

impl Tagger for Spacy {
    fn tag(&mut self, text: &str) -> Vec<TaggedToken> {
        self.0
            .process(text)
            .into_iter()
            .map(|t| TaggedToken { text: t.text.into(), tag: t.tag.into(), whitespace: t.whitespace.into() })
            .collect()
    }
}

struct Espeak(EspeakFallback);

impl Fallback for Espeak {
    fn phonemize(&mut self, word: &str) -> Option<String> {
        self.0.phonemize(word).unwrap()
    }
}

/// `upstream_pipeline`: kokoro without foreign-script splitting, the whole text read by misaki.
fn upstream(text: &str) -> String {
    let mut tokens = Vec::new();
    let mut fallback = Espeak(EspeakFallback::new(false).unwrap());
    kokoro::append_segment(&G2p::new(false), &mut tokens, text, &mut Spacy(Nlp::new()), Some(&mut fallback)).unwrap();
    kokoro::tokens_to_ps(&tokens)
}

#[test]
fn text_in_native_script_unchanged() {
    let text = "The β-catenin level rose by Δ, about π percent. Call [Kokoro](/kˈOkəɹO/) now.";
    assert_eq!(phonemize(text), upstream(text));
}

#[test]
fn overrides_and_greek_symbols_unchanged() {
    for text in ["Go to [东京](/tˈOkjO/) or [Ελλάδα](/ɛlˈɑdə/) now.", "αβ T cells and ΔΔCt values."] {
        assert_eq!(phonemize(text), upstream(text));
    }
}

#[test]
fn cyrillic_uses_russian() {
    assert!(phonemize("Привет").contains(&espeak("ru", "Привет")));
}

#[test]
fn english_pipeline_reads_greek_words_not_letters() {
    assert!(phonemize("She said Καλημέρα.").contains(&espeak("el", "Καλημέρα")));
}

#[test]
fn spacing_follows_source_text() {
    let ps = phonemize("He said (Привет) and Привет.");
    let russian = espeak("ru", "Привет");
    assert!(ps.contains(&format!("({russian})")) && ps.contains(&format!(" {russian}.")), "{ps}");
}

#[test]
fn arabic_sounds_missing_from_vocab_use_nearest_phoneme() {
    let ps = phonemize("He said مرحبا to علي.");
    assert!(!ps.contains('ħ') && !ps.contains('ʕ'), "{ps}");
    assert!(ps.contains(&espeak("ar", "مرحبا").replace('ħ', "h")), "{ps}");
}

#[test]
fn hebrew_points_are_dropped() {
    assert_eq!(phonemize("שָׁלוֹם"), espeak("he", "שלום"));
}

#[test]
fn a_foreign_run_is_one_token_per_word() {
    let (ps, tokens) = pipeline().phonemize("Say Москва река now").unwrap();
    let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, ["Say", "Москва река", "", "now"]);
    assert_eq!(ps.split(' ').count(), 4);
}

#[test]
fn han_and_kana_are_unsupported_without_their_g2p() {
    for (text, language, run) in [
        ("他说：“我们走吧，明天见。”", Language::Chinese, "他说"),
        ("labeled with elements—土火水木金—rather than numbers.", Language::Chinese, "土火水木金"),
        ("東京へ行く", Language::Japanese, "東京へ行く"),
    ] {
        match pipeline().phonemize(text) {
            Err(Error::Unsupported { language: l, text: t }) => assert_eq!((l, t.as_str()), (language, run)),
            other => panic!("{text}: {other:?}"),
        }
    }
}

/// Stands in for misaki's Chinese G2P: one syllable per character, with a tone arrow.
struct FakeChinese;

impl ForeignG2p for FakeChinese {
    fn phonemize(&self, text: &str) -> Result<String, String> {
        Ok(text
            .chars()
            .map(|c| match c {
                '北' => "pei↓",
                '京' => "ʨiŋ→",
                '他' => "tʰa→",
                '、' => ",",
                _ => "ni↓",
            })
            .collect::<Vec<_>>()
            .join(" "))
    }

    fn reads(&self, c: char) -> bool {
        c != '兙'
    }
}

fn with_chinese() -> Pipeline {
    Pipeline::new(Lang::AmericanEnglish).unwrap().with_foreign_g2p(Language::Chinese, Box::new(FakeChinese))
}

#[test]
fn han_uses_a_supplied_chinese_g2p() {
    let pipeline = with_chinese();
    let ps = pipeline.phonemize("他说：“我们走吧，明天见。”").unwrap().0;
    assert!(ps.starts_with("tˈa ˈni"), "{ps}");
}

#[test]
fn chinese_tones_become_stress_for_english_voices() {
    let ps = with_chinese().phonemize("He saw 北京 today.").unwrap().0;
    assert!(!ps.contains(['↓', '↗', '↘', '→', 'ʰ']), "{ps}");
    assert!(ps.contains("bˈei ʨˈiŋ"), "{ps}");
}

#[test]
fn han_without_mandarin_reading_goes_to_english() {
    let (ps, tokens) = with_chinese().phonemize("我兙你").unwrap();
    assert_eq!(tokens.iter().filter(|t| t.text == "我" || t.text == "你").count(), 2, "{tokens:?}");
    assert!(ps.starts_with("ˈniʧˈInizl") && ps.ends_with("ˈni"), "{ps}");
}

fn speech(chunks: &[Result<Chunk, Error>]) -> Vec<Result<&str, String>> {
    chunks
        .iter()
        .map(|c| match c {
            Ok(c) if c.pause.is_some() => Ok("<pause>"),
            Ok(c) => Ok(c.text.as_str()),
            Err(e) => Err(e.to_string()),
        })
        .collect()
}

#[test]
fn an_unsupported_sentence_is_an_error_and_the_rest_follows() {
    let options = NormalizationOptions::default();
    let chunks: Vec<_> = pipeline().chunks("Hello there. They said 東京. Goodbye now.", &options).collect();
    assert_eq!(
        speech(&chunks),
        [Ok("Hello there."), Err("no G2P for zh text yet: \"東京\"".to_owned()), Ok("Goodbye now.")]
    );
}

#[test]
fn pauses_ids_and_words() {
    let options = NormalizationOptions::default();
    let chunks: Vec<_> = pipeline().chunks("Hello world! [pause:1.5s] ¡Привет!", &options).map(Result::unwrap).collect();
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[1].pause, Some(1.5));
    let hello = &chunks[0];
    assert_eq!(hello.phonemes, "həlˈO wˈɜɹld!");
    assert_eq!(hello.ids, hello.phonemes.chars().map(|c| kokoro_pipeline::id(c).unwrap()).collect::<Vec<_>>());
    assert_eq!(hello.dropped, 0);
    let words: Vec<_> = hello.words.iter().map(|w| (w.text.as_str(), w.phonemes.clone(), w.ids.clone())).collect();
    assert_eq!(words, [("Hello", 0..5, 0..5), ("world", 6..12, 6..12), ("!", 12..13, 12..13)]);
    let russian = &chunks[2];
    assert_eq!(russian.dropped, russian.phonemes.chars().filter(|&c| kokoro_pipeline::id(c).is_none()).count());
    assert_eq!(russian.ids.len() + russian.dropped, russian.phonemes.chars().count());
}

#[test]
fn shared_across_threads() {
    fn check<T: Send + Sync>() {}
    check::<Pipeline>();
    let options = NormalizationOptions::default();
    let texts = ["One sentence here.", "Another, with Привет.", "And qzxv, a third."];
    let expected: Vec<Vec<String>> = texts
        .iter()
        .map(|t| pipeline().chunks(t, &options).map(|c| c.unwrap().phonemes).collect())
        .collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    for _ in 0..20 {
                        for (t, e) in texts.iter().zip(&expected) {
                            let got: Vec<String> = pipeline().chunks(t, &options).map(|c| c.unwrap().phonemes).collect();
                            assert_eq!(&got, e);
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
    });
}
