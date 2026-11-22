//! Kokoro-82M's input vocabulary and the tokenizer that reads IPA into it.
//!
//! The model's `config.json` (hexgrad/Kokoro-82M, Apache-2.0) gives each id a single character, so
//! sounds IPA writes with two letters have stand-ins there: `A` for e͡ɪ, `ʧ` for t͡ʃ, `T` for the
//! English flap. Everything before the model writes standard IPA, and [`tokenize`] reads it:
//!
//! - Two letters joined by a tie bar (U+0361) are one id when the model has one for them, and two
//!   ids otherwise. Untied letters are always separate: "nutshell" is n ʌ t ʃ ɛ l.
//! - `t̬`, the voiced t of American dictionaries, is the English flap. `ɾ` is the tap of Spanish
//!   and the other espeak languages; the model learned them as different sounds.
//! - Stress marks may sit at the start of the syllable, as IPA writes them, or before its vowel;
//!   the model was trained on the latter, so each mark moves to just before the next vowel in its
//!   word.
#![forbid(unsafe_code)]

/// `(id, config.json symbol, IPA)` for every id but the padding id 0.
const VOCAB: [(u8, char, &str); 114] = [
    (1, ';', ";"),
    (2, ':', ":"),
    (3, ',', ","),
    (4, '.', "."),
    (5, '!', "!"),
    (6, '?', "?"),
    (9, '—', "—"),
    (10, '…', "…"),
    (11, '\"', "\""),
    (12, '(', "("),
    (13, ')', ")"),
    (14, '“', "“"),
    (15, '”', "”"),
    (16, '\u{20}', "\u{20}"),
    (17, '\u{303}', "\u{303}"),
    (18, 'ʣ', "d͡z"),
    (19, 'ʥ', "d͡ʑ"),
    (20, 'ʦ', "t͡s"),
    (21, 'ʨ', "t͡ɕ"),
    (22, 'ᵝ', "ᵝ"),
    (23, 'ꭧ', "ʈ͡ʂ"),
    (24, 'A', "e͡ɪ"),
    (25, 'I', "a͡ɪ"),
    (31, 'O', "o͡ʊ"),
    (33, 'Q', "ə͡ʊ"),
    (35, 'S', "s͡s"),
    (36, 'T', "t̬"),
    (39, 'W', "a͡ʊ"),
    (41, 'Y', "ɔ͡ɪ"),
    (42, 'ᵊ', "ᵊ"),
    (43, 'a', "a"),
    (44, 'b', "b"),
    (45, 'c', "c"),
    (46, 'd', "d"),
    (47, 'e', "e"),
    (48, 'f', "f"),
    (50, 'h', "h"),
    (51, 'i', "i"),
    (52, 'j', "j"),
    (53, 'k', "k"),
    (54, 'l', "l"),
    (55, 'm', "m"),
    (56, 'n', "n"),
    (57, 'o', "o"),
    (58, 'p', "p"),
    (59, 'q', "q"),
    (60, 'r', "r"),
    (61, 's', "s"),
    (62, 't', "t"),
    (63, 'u', "u"),
    (64, 'v', "v"),
    (65, 'w', "w"),
    (66, 'x', "x"),
    (67, 'y', "y"),
    (68, 'z', "z"),
    (69, 'ɑ', "ɑ"),
    (70, 'ɐ', "ɐ"),
    (71, 'ɒ', "ɒ"),
    (72, 'æ', "æ"),
    (75, 'β', "β"),
    (76, 'ɔ', "ɔ"),
    (77, 'ɕ', "ɕ"),
    (78, 'ç', "ç"),
    (80, 'ɖ', "ɖ"),
    (81, 'ð', "ð"),
    (82, 'ʤ', "d͡ʒ"),
    (83, 'ə', "ə"),
    (85, 'ɚ', "ɚ"),
    (86, 'ɛ', "ɛ"),
    (87, 'ɜ', "ɜ"),
    (90, 'ɟ', "ɟ"),
    (92, 'ɡ', "ɡ"),
    (99, 'ɥ', "ɥ"),
    (101, 'ɨ', "ɨ"),
    (102, 'ɪ', "ɪ"),
    (103, 'ʝ', "ʝ"),
    (110, 'ɯ', "ɯ"),
    (111, 'ɰ', "ɰ"),
    (112, 'ŋ', "ŋ"),
    (113, 'ɳ', "ɳ"),
    (114, 'ɲ', "ɲ"),
    (115, 'ɴ', "ɴ"),
    (116, 'ø', "ø"),
    (118, 'ɸ', "ɸ"),
    (119, 'θ', "θ"),
    (120, 'œ', "œ"),
    (123, 'ɹ', "ɹ"),
    (125, 'ɾ', "ɾ"),
    (126, 'ɻ', "ɻ"),
    (128, 'ʁ', "ʁ"),
    (129, 'ɽ', "ɽ"),
    (130, 'ʂ', "ʂ"),
    (131, 'ʃ', "ʃ"),
    (132, 'ʈ', "ʈ"),
    (133, 'ʧ', "t͡ʃ"),
    (135, 'ʊ', "ʊ"),
    (136, 'ʋ', "ʋ"),
    (138, 'ʌ', "ʌ"),
    (139, 'ɣ', "ɣ"),
    (140, 'ɤ', "ɤ"),
    (142, 'χ', "χ"),
    (143, 'ʎ', "ʎ"),
    (147, 'ʒ', "ʒ"),
    (148, 'ʔ', "ʔ"),
    (156, 'ˈ', "ˈ"),
    (157, 'ˌ', "ˌ"),
    (158, 'ː', "ː"),
    (162, 'ʰ', "ʰ"),
    (164, 'ʲ', "ʲ"),
    (169, '↓', "↓"),
    (171, '→', "→"),
    (172, '↗', "↗"),
    (173, '↘', "↘"),
    (177, 'ᵻ', "ᵻ"),
];

pub const PRIMARY_STRESS: char = 'ˈ';
pub const SECONDARY_STRESS: char = 'ˌ';
const TIE: char = '\u{361}';
/// The tie bar written below the letters, which IPA allows too.
const TIE_BELOW: char = '\u{35c}';
const VOICED: char = '\u{32c}';

/// IPA's vowels, those the vocabulary lacks too (Russian ё is ɵ): a stress mark before one stays
/// with its syllable.
const VOWELS: &str = "aeiouyæɐɑɒɔəɚɝɛɜɞɘɵɤɨʉɪʏɯʊʌøœɶᵊᵻᵿ";
/// Marks a consonant as a syllable's nucleus (n̩, m̍): a stress mark stops there too.
const SYLLABIC: [char; 2] = ['\u{329}', '\u{30d}'];

/// What stops a stress mark looking for its vowel: whitespace, punctuation or another stress mark.
fn is_boundary(c: char, id: Option<u8>) -> bool {
    c.is_whitespace() || id.is_some_and(|id| id <= 16 || id == 156 || id == 157)
}

/// Each IPA string's id, sorted for binary search: one letter, or a letter and a mark.
static BY_IPA: std::sync::LazyLock<Vec<(&str, u8)>> = std::sync::LazyLock::new(|| {
    let mut v: Vec<(&str, u8)> = VOCAB.iter().map(|&(id, _, ipa)| (ipa, id)).collect();
    v.sort_unstable();
    v
});

fn lookup(ipa: &str) -> Option<u8> {
    BY_IPA.binary_search_by_key(&ipa, |&(s, _)| s).ok().map(|i| BY_IPA[i].1)
}

fn single(c: char) -> Option<u8> {
    lookup(c.encode_utf8(&mut [0; 4]))
}

fn joined(first: char, mark: char, second: Option<char>) -> Option<u8> {
    let mut buf = [0; 12];
    let mut n = first.encode_utf8(&mut buf[..]).len();
    n += mark.encode_utf8(&mut buf[n..]).len();
    if let Some(second) = second {
        n += second.encode_utf8(&mut buf[n..]).len();
    }
    lookup(std::str::from_utf8(&buf[..n]).expect("encoded above"))
}

struct Unit {
    start: usize,
    vowel: bool,
    boundary: bool,
    id: Option<u8>,
}

fn units(chars: &[char]) -> (Vec<Unit>, usize) {
    let mut units = Vec::with_capacity(chars.len());
    let mut dropped = 0;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut vowel = VOWELS.contains(c);
        let (len, id) = match chars.get(i + 1) {
            Some(&(TIE | TIE_BELOW)) if let Some(id) = chars.get(i + 2).and_then(|&d| joined(c, TIE, Some(d))) => {
                (3, Some(id))
            }
            Some(&VOICED) if let Some(id) = joined(c, VOICED, None) => (2, Some(id)),
            // The model has no syllabic mark: it places the stress, and the letter is read.
            Some(m) if SYLLABIC.contains(m) => {
                vowel = true;
                (2, single(c))
            }
            _ => (1, single(c)),
        };
        if id.is_none() && c != TIE && c != TIE_BELOW {
            dropped += 1;
        }
        units.push(Unit { start: i, vowel, boundary: is_boundary(c, id), id });
        i += len;
    }
    (units, dropped)
}

/// IPA read into Kokoro-82M ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tokens {
    pub ids: Vec<u8>,
    /// `kept[i]`: how many ids the first `i` characters gave. A stress mark counts where it was
    /// written, which is in the same word as where it moves to.
    pub kept: Vec<usize>,
    /// Letters and marks the vocabulary lacks, left out as `KModel` leaves out symbols it doesn't
    /// know. Tie bars and syllabic marks aren't counted: the tokenizer reads them (letters tied
    /// with no one symbol for them are read apart).
    pub dropped: usize,
}

pub fn tokenize(ipa: &str) -> Tokens {
    let chars: Vec<char> = ipa.chars().collect();
    let (units, dropped) = units(&chars);
    let mut ids = Vec::with_capacity(units.len());
    let mut held: Option<(usize, u8)> = None;
    for (k, unit) in units.iter().enumerate() {
        if let Some((at, mark)) = held
            && at == k
        {
            ids.push(mark);
            held = None;
        }
        let Some(id) = unit.id else { continue };
        if id == 156 || id == 157 {
            let rest = &units[k + 1..];
            let vowel = rest.iter().position(|u| u.vowel || u.boundary);
            if let Some(v) = vowel.filter(|&v| rest[v].vowel && v > 0) {
                held = Some((k + 1 + v, id));
                continue;
            }
        }
        ids.push(id);
    }
    let mut kept = Vec::with_capacity(chars.len() + 1);
    let mut n = 0;
    let mut next = units.iter().peekable();
    for i in 0..=chars.len() {
        while let Some(u) = next.next_if(|u| u.start < i) {
            n += usize::from(u.id.is_some());
        }
        kept.push(n);
    }
    Tokens { ids, kept, dropped }
}

/// How many ids `ipa` reads as, which is what chunk limits count.
pub fn count(ipa: &str) -> usize {
    tokenize(ipa).ids.len()
}

/// The model's `config.json` symbol for an id, as the reference pipeline writes it.
pub fn symbol(id: u8) -> Option<char> {
    VOCAB.iter().find(|&&(i, _, _)| i == id).map(|&(_, c, _)| c)
}

/// The id of a `config.json` symbol: how the reference pipeline's phonemes become ids.
pub fn symbol_id(symbol: char) -> Option<u8> {
    VOCAB.iter().find(|&&(_, c, _)| c == symbol).map(|&(id, _, _)| id)
}

/// `ids` as `config.json` symbols, comparable with the reference pipeline's phonemes.
pub fn symbols(ids: &[u8]) -> String {
    ids.iter().filter_map(|&id| symbol(id)).collect()
}

/// The vocabulary's IPA for every id, for checking G2P data against it.
pub fn ipa(id: u8) -> Option<&'static str> {
    VOCAB.iter().find(|&&(i, _, _)| i == id).map(|&(_, _, ipa)| ipa)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(ipa: &str) -> String {
        symbols(&tokenize(ipa).ids)
    }

    #[test]
    fn tied_letters_are_one_sound_and_untied_two() {
        assert_eq!(read("t͡ʃɜɹt͡ʃ"), "ʧɜɹʧ");
        assert_eq!(read("nʌtʃɛl"), "nʌtʃɛl");
        assert_eq!(read("ta͡ɪm"), "tIm");
        assert_eq!(read("sɔɪŋ"), "sɔɪŋ");
        assert_eq!(read("ʈ͡ʂʰa↘ŋ"), "ꭧʰa↘ŋ");
        // A tie the model has no symbol for leaves both letters.
        assert_eq!(read("k͡p"), "kp");
        assert_eq!(read("t͜ʃa͜ɪ"), "ʧI");
        assert_eq!(tokenize("t͜ʃa͜ɪ").dropped, 0);
    }

    #[test]
    fn the_english_flap_is_voiced_t() {
        assert_eq!(read("bˈʌt̬əɹ"), "bˈʌTəɹ");
        assert_eq!(read("pˈeɾo"), "pˈeɾo");
    }

    #[test]
    fn stress_moves_to_the_vowel() {
        assert_eq!(read("ˈpɹistʃɪp"), "pɹˈistʃɪp");
        assert_eq!(read("pɹˈistʃɪp"), "pɹˈistʃɪp");
        assert_eq!(read("ˌʌndəɹˈstænd"), "ˌʌndəɹstˈænd");
        assert_eq!(read("ˈt͡ʃa͡ɪnə"), "ʧˈInə");
        // No vowel before the word ends: the mark stays.
        assert_eq!(read("ˈst ˈa"), "ˈst ˈa");
        assert_eq!(read("ˈst\u{a0}a"), "ˈsta");
        assert_eq!(read("hˈm."), "hˈm.");
        // A vowel the vocabulary lacks still holds its syllable's stress (Russian ё).
        assert_eq!(read("fʲˈɵdʌr"), "fʲˈdʌr");
        assert_eq!(read("ˈhɝdəl"), "hˈdəl");
        // So does a syllabic consonant, read without its mark.
        assert_eq!(read("ˈn̩ðə"), "ˈnðə");
        assert_eq!(tokenize("ˈn̩").dropped, 0);
    }

    #[test]
    fn kept_counts_ids_per_character() {
        let t = tokenize("ˈta͡ɪ ¿");
        assert_eq!(t.ids, [62, 156, 25, 16]);
        assert_eq!(t.kept, [0, 1, 2, 3, 3, 3, 4, 4]);
        assert_eq!(t.dropped, 1);
        assert_eq!(count("ˈta͡ɪ ¿"), 4);
    }

    #[test]
    fn every_symbol_reads_back_as_its_id() {
        for (id, symbol, ipa) in VOCAB {
            assert_eq!(tokenize(ipa).ids, [id], "{symbol} {ipa}");
            assert_eq!(symbol_id(symbol), Some(id));
        }
    }
}
