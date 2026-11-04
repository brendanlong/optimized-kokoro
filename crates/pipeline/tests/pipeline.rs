//! kokoro's `tests/test_foreign.py` for the English pipelines, and the chunk API.

use kokoro_espeak::{EspeakFallback, EspeakG2P};
use kokoro_g2p_en::{Fallback, G2p, TaggedToken, Tagger, kokoro};
use kokoro_pipeline::foreign::{Language, Script};
use kokoro_pipeline::{Chunk, Error, ForeignG2p, Lang, NormalizationOptions, Pipeline};
use kokoro_spacy_en::Nlp;
use std::sync::OnceLock;

/// The Japanese G2P without its dictionary, so kana runs are "Japanese text" whether or not UniDic
/// is installed; `japanese_runs_are_read_by_the_japanese_g2p` reads them with it.
struct NoDictionary;

impl ForeignG2p for NoDictionary {
    fn phonemize(&self, _: &str) -> Result<String, String> {
        Err("no dictionary".into())
    }

    fn available(&self) -> bool {
        false
    }
}

fn pipeline() -> &'static Pipeline {
    static P: OnceLock<Pipeline> = OnceLock::new();
    P.get_or_init(|| {
        Pipeline::new(Lang::AmericanEnglish).unwrap().with_foreign_g2p(Language::Japanese, Box::new(NoDictionary))
    })
}

#[test]
fn japanese_runs_are_read_by_the_japanese_g2p() {
    if !kokoro_pipeline::japanese_available() {
        return kokoro_pipeline::skip_without_dictionary("no UniDic at $KOKORO_UNIDIC");
    }
    let pipeline = Pipeline::new(Lang::AmericanEnglish).unwrap();
    for (text, expected) in [
        ("東京へ行く", "toːkʲoː e ikɯ"),
        ("The wordカナmeans hello.", "ðə wˈɜɹdkanamˈinz həlˈO."),
        ("東京\u{200d}へ", "dˈʊŋʨˈiŋe"),
    ] {
        assert_eq!(pipeline.phonemize(text).unwrap().0, expected, "{text}");
    }
    let options = NormalizationOptions::default();
    let unread: Vec<usize> =
        pipeline.chunks("ありがとう, Goodbye 你好. ゃ", &options).map(|c| c.unwrap().unread).collect();
    assert_eq!(unread, [0]);
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
fn runs_without_a_g2p_are_read_as_their_name() {
    for (text, expected) in [
        ("東京へ行く", "ʤˌæpənˈiz tˈɛkst"),
        ("カナ カナ", "ʤˌæpənˈiz tˈɛkst"),
        ("He wrote 兙 on the board.", "hˌi ɹˈOt ʧˌInˈiz tˈɛkst ˌɔn ðə bˈɔɹd."),
        ("The wordカナmeans hello.", "ðə wˈɜɹd ʤˌæpənˈiz tˈɛkst mˈinz həlˈO."),
        ("Wake Station (わけ駅, Wake-eki).", "wˈAk stˈAʃən (ʤˌæpənˈiz tˈɛkst, wˈAkˈɛki)."),
        ("Thai สวัสดี ครับ is hello.", "tˈI tˈI tˈɛkst ɪz həlˈO."),
        ("Mongolian ᠮᠣᠩᠭᠣᠯ script.", "mɑŋɡˈOljən mɑŋɡˈOljən tˈɛkst skɹˈɪpt."),
    ] {
        assert_eq!(phonemize(text), expected, "{text}");
    }
}

#[test]
fn a_placeholder_is_one_word_holding_the_run() {
    let (_, tokens) = pipeline().phonemize("Say カナ カナ now").unwrap();
    let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, ["Say", "カナ カナ", "", "now"]);
}

#[test]
fn other_scripts_are_read_by_the_voice_espeak_switches_to() {
    for (text, word, voice) in [
        ("Georgian გამარჯობა is hello.", "გამარჯობა", "ka"),
        ("Armenian Բարեւ is hello.", "Բարեւ", "hy"),
        ("Tamil வணக்கம் is hello.", "வணக்கம்", "ta"),
    ] {
        let ps = phonemize(text);
        assert!(ps.contains(&espeak(voice, word)) && !ps.contains(['(', ')']), "{text}: {ps}");
    }
}

#[test]
fn every_voice_a_run_may_go_to_loads() {
    let voices = kokoro_pipeline::foreign::espeak_voices();
    for voice in ["ar", "bn", "el", "he", "hi", "hy", "ka", "ko", "ru", "ta"] {
        assert!(voices.contains(&voice), "{voices:?}");
    }
    Pipeline::new(Lang::BritishEnglish).unwrap();
    assert!(phonemize("Shavian 𐑖𐑱𐑝𐑾𐑯.").starts_with("ʃˈAviən "));
}

#[test]
fn invisible_characters_neither_split_nor_fuse_placeholders() {
    for (text, expected) in [
        (
            "Thai \u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}\u{200b}\u{e04}\u{e23}\u{e31}\u{e1a}\u{200b}\u{e1c}\u{e21} is hello.",
            "tˈI tˈI tˈɛkst ɪz həlˈO.",
        ),
        ("\u{e2a}\u{e27}\u{e31}\u{e2a}\u{ad}\u{e14}\u{e35}", "tˈI tˈɛkst"),
        ("カナ\u{200b}カナ", "ʤˌæpənˈiz tˈɛkst"),
        ("カナ\u{200e}カナ", "ʤˌæpənˈiz tˈɛkst"),
        ("兙\u{200b}兙", "ʧˌInˈiz tˈɛkst"),
        ("東京\u{200d}へ", "dˈʊŋʨˈiŋ ʤˌæpənˈiz tˈɛkst"),
        ("Привет\u{200b}สวัสดี", "prʲivʲˈet tˈI tˈɛkst"),
        ("a\u{301}สวัสดี", "ˈæ tˈI tˈɛkst"),
        ("สวัสดี\u{200b}hello", "tˈI tˈɛkst həlˈO"),
        ("hello\u{2060}สวัสดี", "həlˈO tˈI tˈɛkst"),
        ("(สวัสดี) x", "(tˈI tˈɛkst) ˈɛks"),
    ] {
        assert_eq!(phonemize(text), expected, "{text:?}");
    }
    let (_, tokens) = pipeline().phonemize("x สวัสดี\u{200b}ครับ y").unwrap();
    assert_eq!(tokens.iter().filter(|t| t.text.contains('ส')).count(), 1, "{tokens:?}");
}

#[test]
fn scripts_a_voices_alphabet_holds_but_it_doesnt_read_are_unread() {
    for (text, name) in [
        ("ꦗꦮ", "ʤˌɑvənˈiz"),
        ("ꨌꩌ", "kˈæm"),
        ("ꯃꯤꯇꯩ", "mˈiTA mˈAɛk"),
        ("ꮳꮃꭹ", "ʧˈɛɹəkˌi"),
        ("ꬁꬂ", "ˌiθiˈɑpɪk"),
        ("ꩠꩡ", "mˈiənmˌɑɹ"),
        ("ϢϤϦ", "kˈɑptɪk"),
        ("ⴂⴀⴋⴀ", "ʤˈɔɹʤᵊn"),
    ] {
        let options = NormalizationOptions::default();
        let chunks: Vec<_> = pipeline().chunks(text, &options).map(Result::unwrap).collect();
        assert_eq!((chunks[0].phonemes.as_str(), chunks[0].unread), (format!("{name} tˈɛkst").as_str(), 1), "{text}");
    }
}

#[test]
fn the_runs_script_not_its_first_letter_picks_the_voice() {
    let word = espeak("ka", "გამარჯობა");
    assert_eq!(phonemize("ᲒᲐᲛᲐᲠᲯᲝᲑᲐ"), word);
    assert_eq!(phonemize("Ა გამარჯობა"), format!("{} {word}", espeak("ka", "ა")));
    assert_eq!(phonemize("ﬓ Բարեւ"), format!("ɑɹmˈiniən tˈɛkst {}", espeak("hy", "Բարեւ")));
}

/// A G2P that reads every run as nothing.
struct Silent;

impl ForeignG2p for Silent {
    fn phonemize(&self, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
}

#[test]
fn a_run_its_g2p_reads_as_nothing_is_unread() {
    let pipeline = Pipeline::new(Lang::AmericanEnglish)
        .unwrap()
        .with_foreign_g2p(Language::Script(Script::Thai), Box::new(Silent))
        .with_foreign_g2p(Language::Script(Script::Cyrillic), Box::new(Silent));
    assert_eq!(pipeline.phonemize("x สวัสดี y").unwrap().0, "ˈɛks tˈI tˈɛkst wˈI");
    assert_eq!(pipeline.phonemize("x Привет y").unwrap().0, "ˈɛks wˈI");
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
fn han_is_read_by_misakis_chinese_g2p() {
    for (text, expected) in [
        ("He saw 北京 today.", "hˌi sˈɔ bˈeiʨˈiŋ tədˈA."),
        ("The bank's 银行行长 said 你好.", "ðə bˈæŋks ˈinxˈaŋxˈaŋꭧˈaŋ sˈɛd ˈnixˈau."),
    ] {
        assert_eq!(phonemize(text), expected, "{text}");
    }
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
fn han_without_mandarin_reading_is_chinese_text() {
    let (ps, tokens) = with_chinese().phonemize("我兙你").unwrap();
    assert_eq!(tokens.iter().filter(|t| t.text == "我" || t.text == "你").count(), 2, "{tokens:?}");
    assert_eq!(ps, "ˈni ʧˌInˈiz tˈɛkst ˈni");
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
fn placeholders_are_counted_and_a_sentence_of_one_is_spoken() {
    let options = NormalizationOptions::default();
    let chunks: Vec<_> =
        pipeline().chunks("Hello there. 東京。 [pause:0.5s] ありがとう, Goodbye 你好. สวัสดี", &options).collect();
    assert_eq!(
        speech(&chunks),
        [Ok("Hello there."), Ok("東京."), Ok("<pause>"), Ok("ありがとう, Goodbye 你好."), Ok("สวัสดี")]
    );
    let unread: Vec<usize> = chunks.iter().map(|c| c.as_ref().unwrap().unread).collect();
    assert_eq!(unread, [0, 0, 0, 1, 1]);
}

#[test]
fn pauses_ids_and_words() {
    let options = NormalizationOptions::default();
    let chunks: Vec<_> =
        pipeline().chunks("Hello world! [pause:1.5s] ¡Привет!", &options).map(Result::unwrap).collect();
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

/// The reference fails the whole request with `OverflowError` on a number of 400 digits, after
/// "Hello there."; the number reader reads numbers past the decillions digit by digit.
#[test]
fn a_number_too_large_to_name_is_read_digit_by_digit() {
    let options = NormalizationOptions::default();
    let text = format!("Hello there. I owe 1{} things. Bye now.", "0".repeat(59));
    let texts: Vec<String> = pipeline().chunks(&text, &options).map(|c| c.unwrap().text).collect();
    assert_eq!(
        texts,
        ["Hello there.".to_owned(), format!("I owe one{} things.", " zero".repeat(59)), "Bye now.".to_owned()]
    );
}

/// A link with blank phonemes is stripped off the chunk's ends; its word has no span left and is
/// dropped, as Python's `inside` list is empty for it (this used to index past the end).
#[test]
fn links_with_blank_phonemes_at_chunk_ends() {
    let options = NormalizationOptions::default();
    let words = |text: &str| -> Vec<Vec<(String, usize, usize)>> {
        pipeline()
            .chunks(text, &options)
            .map(|c| c.unwrap().words.into_iter().map(|w| (w.text, w.ids.start, w.ids.end)).collect())
            .collect()
    };
    let say = vec![("Say".to_owned(), 0, 3)];
    assert_eq!(words("Say [word](/ /)"), std::slice::from_ref(&say));
    assert_eq!(words("[word](/ /) say"), [vec![("say".to_owned(), 0, 3)]]);
    assert_eq!(words("Say [word](/ /) [now](/ /)"), std::slice::from_ref(&say));
    assert_eq!(words("Say [word](/ /) [pause:1s] more [x](/ /)"), [say, vec![], vec![("more".to_owned(), 0, 4)]]);
    let clauses = ["the quick brown fox jumps over the lazy dog"; 7].join(", ");
    let cut = words(&format!("{clauses}, {clauses} [word](/ /)"));
    assert_eq!(cut.len(), 2);
    assert_eq!(cut[1].last(), Some(&("dog".to_owned(), 239, 243)));
}

#[test]
fn shared_across_threads() {
    fn check<T: Send + Sync>() {}
    check::<Pipeline>();
    let options = NormalizationOptions::default();
    let texts = ["One sentence here.", "Another, with Привет.", "And qzxv, a third."];
    let expected: Vec<Vec<String>> =
        texts.iter().map(|t| pipeline().chunks(t, &options).map(|c| c.unwrap().phonemes).collect()).collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    for _ in 0..20 {
                        for (t, e) in texts.iter().zip(&expected) {
                            let got: Vec<String> =
                                pipeline().chunks(t, &options).map(|c| c.unwrap().phonemes).collect();
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
