//! kokoro's `tests/test_foreign.py` for the English pipelines, and the chunk API.

use kokoro_espeak::{EspeakFallback, EspeakG2P};
use kokoro_g2p_en::{Fallback, G2p, TaggedToken, Tagger, kokoro};
use kokoro_pipeline::foreign::{Language, Script};
use kokoro_pipeline::{Chunk, Error, ForeignG2p, Lang, Pipeline};
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
        // Runs whose letters meet are words of their own.
        ("The wordカナmeans hello.", "ðə wˈɜɹd kana mˈinz həlˈO."),
        // Runs are two words even with a joiner between them.
        ("東京\u{200d}へ", "dˈʊŋʨˈiŋ e"),
    ] {
        assert_eq!(read(&pipeline.phonemize(text).unwrap().0), expected, "{text}");
    }
    let unread: Vec<usize> = pipeline.chunks("ありがとう, Goodbye 你好. ゃ").map(|c| c.unwrap().unread).collect();
    assert_eq!(unread, [0]);
}

/// What the model reads for `ipa`, in `config.json` symbols: the expectations below are written so.
fn read(ipa: &str) -> String {
    kokoro_reference::ours(ipa)
}

fn phonemize(text: &str) -> String {
    read(&pipeline().phonemize(text).unwrap().0)
}

fn espeak(language: &str, text: &str) -> String {
    read(&EspeakG2P::new(language).unwrap().phonemize(text))
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
        self.0.phonemize(word)
    }
}

/// `upstream_pipeline`: kokoro without foreign-script splitting, the whole text read by misaki.
fn upstream(text: &str) -> String {
    let mut tokens = Vec::new();
    let mut fallback = Espeak(EspeakFallback::new(false).unwrap());
    kokoro::append_segment(&G2p::new(false), &mut tokens, text, &mut Spacy(Nlp::new()), Some(&mut fallback)).unwrap();
    read(&kokoro::tokens_to_ps(&tokens))
}

#[test]
fn text_in_native_script_unchanged() {
    let text = "The β-catenin level rose by Δ, about π percent. Call [Kokoro](/ˈko͡ʊkəɹo͡ʊ/) now.";
    assert_eq!(phonemize(text), upstream(text));
}

#[test]
fn overrides_and_greek_symbols_unchanged() {
    for text in ["Go to [东京](/ˈto͡ʊkjo͡ʊ/) or [Ελλάδα](/ɛˈlɑdə/) now.", "αβ T cells and ΔΔCt values."]
    {
        assert_eq!(phonemize(text), upstream(text));
    }
}

/// A link's IPA reaches the model as written, the Spanish tap and glottal stop included.
#[test]
fn links_are_read_as_written() {
    assert_eq!(phonemize("Say [pero](/ˈpeɾo/) and [uh-oh](/ˈʔʌʔo͜ʊ/)."), "sˈA pˈeɾo ænd ʔˈʌʔO.");
    // A dot is a syllable break, not a full stop.
    assert_eq!(phonemize("Pour [water](/ˈwɔ.t̬əɹ/)."), "pˈɔɹ wˈɔTəɹ.");
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
    assert!(ps.contains(&read(&EspeakG2P::new("ar").unwrap().phonemize("مرحبا").replace('ħ', "h"))), "{ps}");
}

#[test]
fn hebrew_points_are_dropped() {
    assert_eq!(phonemize("שָׁלוֹם"), espeak("he", "שלום"));
}

#[test]
fn a_foreign_run_is_one_token_per_word() {
    let (ps, tokens) = pipeline().phonemize("Say Москва река now").unwrap();
    let texts: Vec<&str> = tokens.iter().map(|t| t.text.as_str()).collect();
    assert_eq!(texts, ["Say", "Москва", "река", "now"]);
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
        ("a\u{301}สวัสดี", "ˌAəkjˈut tˈI tˈɛkst"),
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
        let chunks: Vec<_> = pipeline().chunks(text).map(Result::unwrap).collect();
        assert_eq!((read(&chunks[0].phonemes), chunks[0].unread), (format!("{name} tˈɛkst"), 1), "{text}");
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
    assert_eq!(read(&pipeline.phonemize("x สวัสดี y").unwrap().0), "ˈɛks tˈI tˈɛkst wˈI");
    assert_eq!(read(&pipeline.phonemize("x Привет y").unwrap().0), "ˈɛks wˈI");
}

/// Stands in for misaki's Chinese G2P: one syllable per character, with a tone arrow.
struct FakeChinese;

impl ForeignG2p for FakeChinese {
    fn phonemize(&self, text: &str) -> Result<String, String> {
        Ok(text
            .chars()
            .map(|c| match c {
                '北' => "pei↓",
                '京' => "t͡ɕiŋ→",
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
        // The tone's stress mark moves from before n to the vowel (docs/divergences.md).
        ("The bank's 银行行长 said 你好.", "ðə bˈæŋks ˈinxˈaŋxˈaŋꭧˈaŋ sˈɛd nˈixˈau."),
    ] {
        assert_eq!(phonemize(text), expected, "{text}");
    }
}

#[test]
fn han_uses_a_supplied_chinese_g2p() {
    let pipeline = with_chinese();
    let ps = pipeline.phonemize("他说：“我们走吧，明天见。”").unwrap().0;
    assert!(read(&ps).starts_with("tˈa nˈi"), "{ps}");
}

#[test]
fn chinese_tones_become_stress_for_english_voices() {
    let ps = with_chinese().phonemize("He saw 北京 today.").unwrap().0;
    assert!(!ps.contains(['↓', '↗', '↘', '→', 'ʰ']), "{ps}");
    assert!(read(&ps).contains("bˈei ʨˈiŋ"), "{ps}");
}

#[test]
fn han_without_mandarin_reading_is_chinese_text() {
    let (ps, tokens) = with_chinese().phonemize("我兙你").unwrap();
    assert_eq!(tokens.iter().filter(|t| t.text == "我" || t.text == "你").count(), 2, "{tokens:?}");
    assert_eq!(read(&ps), "nˈi ʧˌInˈiz tˈɛkst nˈi");
}

#[test]
fn the_chinese_pipeline_reads_unread_runs_as_the_english_ones_do() {
    let pipeline = Pipeline::new(Lang::Chinese).unwrap().with_foreign_g2p(Language::Japanese, Box::new(NoDictionary));
    for (text, expected) in
        [("カナ カナ", "ʤˌæpənˈiz tˈɛkst"), ("カナ\u{200b}カナ", "ʤˌæpənˈiz tˈɛkst"), ("，カナ", ", ʤˌæpənˈiz tˈɛkst")]
    {
        assert_eq!(read(&pipeline.phonemize(text).unwrap().0), expected, "{text:?}");
    }
    let unread: Vec<usize> = pipeline.chunks("他说カナ カナ。").map(|c| c.unwrap().unread).collect();
    assert_eq!(unread, [1]);
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
    let chunks: Vec<_> = pipeline().chunks("Hello there. 東京。 [pause:0.5s] ありがとう, Goodbye 你好. สวัสดี").collect();
    assert_eq!(
        speech(&chunks),
        [Ok("Hello there."), Ok("東京."), Ok("<pause>"), Ok("ありがとう, Goodbye 你好."), Ok("สวัสดี")]
    );
    let unread: Vec<usize> = chunks.iter().map(|c| c.as_ref().unwrap().unread).collect();
    assert_eq!(unread, [0, 0, 0, 1, 1]);
}

#[test]
fn pauses_ids_and_words() {
    let chunks: Vec<_> = pipeline().chunks("Hello world! [pause:1.5s] ¡Привет!").map(Result::unwrap).collect();
    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[1].pause, Some(1.5));
    let hello = &chunks[0];
    assert_eq!(hello.phonemes, "həˈlo͡ʊ ˈwɜɹld!");
    assert_eq!(kokoro_reference::ours(&hello.phonemes), "həlˈO wˈɜɹld!");
    assert_eq!((hello.ids.len(), hello.dropped), (13, 0));
    let words = hello.words();
    let words: Vec<_> = words.iter().map(|w| (w.text.as_str(), w.phonemes.clone(), w.ids.clone())).collect();
    assert_eq!(words, [("Hello", 0..7, 0..5), ("world", 8..14, 6..12), ("!", 14..15, 12..13)]);
    // "¡" isn't in the vocabulary.
    let russian = &chunks[2];
    assert!(russian.phonemes.starts_with('¡'));
    assert_eq!(russian.dropped, 1);
}

/// The reference fails the whole request with `OverflowError` on a number of 400 digits, after
/// "Hello there."; the number reader reads numbers past the decillions digit by digit.
#[test]
fn a_number_too_large_to_name_is_read_digit_by_digit() {
    let text = format!("Hello there. I owe 1{} things. Bye now.", "0".repeat(59));
    let texts: Vec<String> = pipeline().chunks(&text).map(|c| c.unwrap().text).collect();
    assert_eq!(
        texts,
        ["Hello there.".to_owned(), format!("I owe one{} things.", " zero".repeat(59)), "Bye now.".to_owned()]
    );
}

/// A link with blank phonemes is stripped off the chunk's ends; its word has no span left and is
/// dropped, as Python's `inside` list is empty for it (this used to index past the end).
#[test]
fn links_with_blank_phonemes_at_chunk_ends() {
    let words = |text: &str| -> Vec<Vec<(String, usize, usize)>> {
        pipeline()
            .chunks(text)
            .map(|c| c.unwrap().words().into_iter().map(|w| (w.text, w.ids.start, w.ids.end)).collect())
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
    let texts = ["One sentence here.", "Another, with Привет.", "And qzxv, a third."];
    let expected: Vec<Vec<String>> =
        texts.iter().map(|t| pipeline().chunks(t).map(|c| c.unwrap().phonemes).collect()).collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    for _ in 0..20 {
                        for (t, e) in texts.iter().zip(&expected) {
                            let got: Vec<String> = pipeline().chunks(t).map(|c| c.unwrap().phonemes).collect();
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

/// Cancelling stops at the next G2P call, even inside one sentence cut at its clauses.
#[test]
fn cancelling_stops_at_the_next_g2p_call() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // One sentence of 40 clauses, far over the chunk limit: one G2P call for it, then one per clause.
    let text = vec!["the quick brown fox jumps over the lazy dog"; 40].join(", ") + ".";
    let asked = AtomicUsize::new(0);
    let cancelled = || asked.fetch_add(1, Ordering::SeqCst) >= 3;
    let mut chunks = pipeline().chunks_within_groups(&text, 100, 100, &cancelled);
    assert!(matches!(chunks.next(), Some(Err(Error::Cancelled))));
    assert_eq!(asked.load(Ordering::SeqCst), 4, "asked once per G2P call, up to the one cancelled");
    assert!(chunks.all(|c| matches!(c, Err(Error::Cancelled))));
}

fn name(spelling: &str) -> String {
    read(
        kokoro_reference::divergence::name(spelling).unwrap_or_else(|| panic!("{spelling} isn't a name in names.json")),
    )
}

#[test]
fn names_are_read_in_their_own_language() {
    let british = Pipeline::new(Lang::BritishEnglish).unwrap();
    for spelling in ["Celebrimbor", "Minas Tirith", "Gil-galad", "Eärendil", "Khazad-dûm", "Glóin", "Théoden", "Nazgûl"]
    {
        let text = format!("Then {spelling} came.");
        let expected = format!("{} kˈAm.", name(spelling));
        assert!(phonemize(&text).ends_with(&expected), "{}", phonemize(&text));
        assert!(read(&british.phonemize(&text).unwrap().0).contains(&name(spelling)), "{spelling} in British English");
    }
    assert_eq!(phonemize("Earendil"), phonemize("Eärendil"));
    assert_eq!(phonemize("Khazad-dum"), phonemize("Khazad-dûm"));
    // misaki's dictionary has "Thrain", but only as its guess at the name.
    assert_eq!(phonemize("Thrain"), name("Thráin"));
}

#[test]
fn real_names_are_read_in_their_own_language_where_their_bearers_are() {
    for spelling in ["Wojciech", "Brzeziński", "Räikkönen", "Dvořák"] {
        let text = format!("Then {spelling} came.");
        assert!(phonemize(&text).ends_with(&format!("{} kˈAm.", name(spelling))), "{}", phonemize(&text));
    }
    // A name's letters inside IPA aren't a word of their own.
    assert!(!phonemize("Sauronɔ [sauronˈɔ]").contains(&name("Sauron")), "{}", phonemize("Sauronɔ [sauronˈɔ]"));
    // These spellings belong mostly to people in English-speaking countries and France, so English reads them.
    for (plain, accented) in [("Brzezinski", "Brzeziński"), ("Dvorak", "Dvořák")] {
        assert!(!phonemize(plain).contains(&name(accented)), "{plain}: {}", phonemize(plain));
    }
}

#[test]
fn the_words_around_a_link_are_read_in_their_sentence() {
    // Outside English, the text around a link is read whole, with a stand-in for the link (issue #79):
    // Portuguese and Spanish "e" and "y" are "i", not letter names, and French keeps its liaison.
    for (lang, with_name, with_word) in [
        (Lang::BrazilianPortuguese, "Smolyar e Grégoire chegou.", "Smolyar e Pedro chegou."),
        (Lang::Spanish, "lo vio y Wojciech llegó.", "lo vio y Pedro llegó."),
        (Lang::French, "Les Aaltonen sont là.", "Les amis sont là."),
        (Lang::Italian, "Il Wojciech è qui.", "Il Pietro è qui."),
    ] {
        let pipeline = Pipeline::new(lang).unwrap();
        let read = |t: &str| read(&pipeline.phonemize(t).unwrap().0);
        let (named, plain) = (read(with_name), read(with_word));
        let before = |ps: &str| ps.split(' ').take(with_name.split(' ').count() - 3).collect::<Vec<_>>().join(" ");
        assert_eq!(before(&named), before(&plain), "{lang:?}: {named} / {plain}");
    }
    let portuguese = Pipeline::new(Lang::BrazilianPortuguese).unwrap();
    let ps = read(&portuguese.phonemize("e [Kokoro](/kokˈɔɾu/) e [Galadriel](/ɡaˈladriel/).").unwrap().0);
    assert!(ps.starts_with("i kokˈɔɾu i "), "{ps}");
    // Liaison goes by the link's first sound, not its letter.
    let french = Pipeline::new(Lang::French).unwrap();
    let ps = read(&french.phonemize("Les [Hélène](/elˈɛn/) et les [Yoda](/jˈoda/).").unwrap().0);
    assert!(ps.starts_with("lez elˈɛn e le jˈoda"), "{ps}");
    // The text's own "Okoro", the stand-in for a link starting with a vowel, keeps its reading, and the link its
    // phonemes.
    let spanish = Pipeline::new(Lang::Spanish).unwrap();
    let ps = read(&spanish.phonemize("Okoro-[Aaltonen](/ˈaːltonen/) y Okoro.").unwrap().0);
    assert_eq!(ps.matches("ˈaːltonen").count(), 1, "{ps}");
    assert_eq!(ps.matches("okˈoɾo").count(), 2, "{ps}");
    // Words keep their own timestamps where another word reads as several ("125").
    let chunk = spanish.chunks("Vi 125 casas con [Gandalf](/ɡˈændɑlf/) ayer.").next().unwrap().unwrap();
    let words: Vec<String> = chunk.words().into_iter().map(|w| w.text).collect();
    assert!(words.contains(&"Gandalf".to_owned()) && words.contains(&"ayer.".to_owned()), "{words:?}");
}

#[test]
fn a_name_takes_possessive_and_plural_endings() {
    assert_eq!(phonemize("Sauron's"), format!("{}z", name("Sauron")));
    assert_eq!(phonemize("Minas Tirith's"), format!("{}s", name("Minas Tirith")));
    // A plural is its own reviewed entry, not a rule: "Estes" isn't Este's plural.
    assert_eq!(phonemize("Silmarils"), name("Silmarils"));
    assert!(!phonemize("Estes Park").contains(&format!("{}z", name("Este"))), "{}", phonemize("Estes Park"));
}

#[test]
fn a_possessive_apostrophe_after_a_silent_s_is_said_as_its_s() {
    for (bare, full) in [("Louis’ patricide", "Louis's patricide"), ("Descartes' ideas", "Descartes's ideas")] {
        assert_eq!(phonemize(bare), phonemize(full), "{bare}");
    }
    assert_eq!(phonemize("Dumas’ novels"), phonemize("Dumas novels").replacen(' ', "z ", 1));
    // A spoken s takes nothing.
    for text in ["James’ hat", "Jesus' name", "the dogs' bowls"] {
        assert_eq!(phonemize(text), phonemize(&text.replace(['\'', '’'], "")), "{text}");
    }
    // A name's own phonemes take the ending too.
    assert_eq!(phonemize("Alès' house"), format!("{}z hˈWs", name("Alès")));
    // A quote it closes stays a quote.
    let quoted = phonemize("He said 'ask Louis' and left.");
    assert!(quoted.contains("lˈui” "), "{quoted}");
}

#[test]
fn names_are_found_in_any_case_but_lowercase_and_lose_to_overrides() {
    let gondor = name("Gondor");
    for text in ["GONDOR", "OF GONDOR AND ROHAN", "GoNdOr"] {
        assert!(phonemize(text).contains(&gondor), "{text}: {}", phonemize(text));
    }
    // Text in capitals keeps its text even where a token leaves a character out ("X'1").
    for text in ["THE CAT'S X'1 THING IS HERE. YES IT IS.", "THE O'-NEIL CAT. YES IT IS.", "THE CAT‘S X’1 HAT. YES."]
    {
        assert!(Pipeline::new(Lang::AmericanEnglish).unwrap().phonemize(text).is_ok(), "{text}");
    }
    for text in ["Gondorian", "[Gondor](/ɡˈɑndɔɹ/)", "gondor"] {
        assert!(!phonemize(text).contains(&gondor), "{text}: {}", phonemize(text));
    }
    // "Minas" alone is left out as ambiguous (Minas Gerais); "Minas Tirith" is a name.
    let minas = name("Minas Tirith").split(' ').next().unwrap().to_string();
    assert!(!phonemize("Minas Gerais").contains(&minas), "{}", phonemize("Minas Gerais"));
    // "ash" is Black Speech only in "ash nazg".
    let ash = name("ash nazg").split(' ').next().unwrap().to_string();
    assert!(phonemize("Ash nazg durbatulûk.").starts_with(&ash), "{}", phonemize("Ash nazg durbatulûk."));
    assert!(!phonemize("The ash fell.").contains(&ash), "{}", phonemize("The ash fell."));
}

#[test]
fn a_name_starting_with_a_vowel_takes_an_and_thee() {
    assert_eq!(phonemize("the Eldar"), format!("ði {}", name("Eldar")));
    assert_eq!(phonemize("the Noldor"), format!("ðə {}", name("Noldor")));
}

#[test]
fn names_match_with_accents_as_combining_marks() {
    assert_eq!(phonemize("Ea\u{308}rendil"), name("Eärendil"));
}

#[test]
fn a_word_reads_the_same_with_accents_as_combining_marks() {
    for lang in [Lang::AmericanEnglish, Lang::Hindi, Lang::French] {
        let pipeline = Pipeline::new(lang).unwrap();
        for (composed, decomposed) in [
            ("Nguyễn", "Nguye\u{302}\u{303}n"),
            ("Nguyễn.", "Nguye\u{302}\u{303}n."),
            ("Йорк", "\u{418}\u{306}\u{43e}\u{440}\u{43a}"),
            ("άλφα.", "\u{3b1}\u{301}\u{3bb}\u{3c6}\u{3b1}."),
        ] {
            assert_eq!(
                pipeline.phonemize(decomposed).unwrap().0,
                pipeline.phonemize(composed).unwrap().0,
                "{lang:?} {decomposed:?}"
            );
        }
    }
}

#[test]
fn a_links_phonemes_keep_their_combining_marks() {
    let chunk = pipeline().chunks("[word](/a\u{303}/)").next().unwrap().unwrap();
    assert_eq!(chunk.phonemes, "a\u{303}");
    assert_eq!(chunk.ids.len(), 2);
}

#[test]
fn names_and_links_are_read_in_every_language() {
    let galadriel = name("Galadriel");
    for lang in [Lang::French, Lang::Spanish, Lang::Italian, Lang::BrazilianPortuguese, Lang::Hindi, Lang::Japanese] {
        if lang == Lang::Japanese && !kokoro_pipeline::japanese_available() {
            continue;
        }
        let pipeline = Pipeline::new(lang).unwrap();
        let ps = read(&pipeline.phonemize("Galadriel [Kokoro](/ˈko͡ʊkəɹo͡ʊ/)").unwrap().0);
        assert_eq!(ps, format!("{galadriel} kˈOkəɹO"), "{lang:?}");
    }
}

#[test]
fn a_name_that_is_a_common_word_in_the_pipelines_language_is_read_as_the_word() {
    let portuguese = Pipeline::new(Lang::BrazilianPortuguese).unwrap();
    let ps = read(&portuguese.phonemize("Este livro").unwrap().0);
    assert!(!ps.starts_with(&name("Este")), "{ps}");
    assert!(phonemize("Este").starts_with(&name("Este")));
}

#[test]
fn names_beside_brackets_contractions_and_other_scripts() {
    let sauron = name("Sauron");
    // A stray "[" before a name keeps the words between them.
    assert!(phonemize("[see Sauron.").starts_with("sˈi "), "{}", phonemize("[see Sauron."));
    // A contraction makes the name part of another word, as spaCy reads it; "'s" is its possessive.
    assert!(!phonemize("Sauron'll come.").contains(&sauron), "{}", phonemize("Sauron'll come."));
    assert_eq!(phonemize("Sauron's"), format!("{sauron}z"));
    // A link misaki doesn't read is its text, which is a name.
    assert_eq!(phonemize("[Sauron](foo)"), sauron);
    // Japanese and Chinese write a Latin word without spaces around it.
    if kokoro_pipeline::japanese_available() {
        let japanese = Pipeline::new(Lang::Japanese).unwrap();
        assert!(
            read(&japanese.phonemize("私はGaladrielです").unwrap().0).contains(&format!(" {} ", name("Galadriel")))
        );
    }
    assert!(phonemize("我和Galadriel去").contains(&name("Galadriel")));
}

/// Only `/phonemes/` links are read; any other (misaki's stress links, which the normalizer turns
/// into words anyway) is its text, split by script like the rest.
#[test]
fn a_link_without_phonemes_is_read_as_its_text() {
    let chinese = Pipeline::new(Lang::Chinese).unwrap();
    let ps = read(&chinese.phonemize("你好[hello](+1)世界").unwrap().0);
    assert!(ps.contains("həlˈO"), "{ps}");
    assert_eq!(phonemize("[read](+2) it"), phonemize("read it"));
}

#[test]
fn a_skipped_name_matches_with_its_accent_as_a_combining_mark() {
    let french = Pipeline::new(Lang::French).unwrap();
    for text in ["Sûr.", "Su\u{302}r."] {
        let ps = read(&french.phonemize(text).unwrap().0);
        assert!(!ps.contains(&name("Sûr")), "{text:?}: {ps}");
    }
}
