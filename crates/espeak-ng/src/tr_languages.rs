//! tr_languages.c: the per-language `Translator` setup.
//!
//! Synthesis-only settings (stress lengths and amplitudes, encodings, length mods, tunes) are not
//! ported.

use crate::common::*;
use crate::defs::*;

// start of unicode pages for character sets
const OFFSET_GREEK: i32 = 0x380;
const OFFSET_CYRILLIC: i32 = 0x420;
const OFFSET_ARMENIAN: i32 = 0x530;
const OFFSET_HEBREW: i32 = 0x590;
const OFFSET_ARABIC: i32 = 0x600;
const OFFSET_SYRIAC: i32 = 0x700;
const OFFSET_DEVANAGARI: i32 = 0x900;
const OFFSET_BENGALI: i32 = 0x980;
const OFFSET_GURMUKHI: i32 = 0xa00;
const OFFSET_GUJARATI: i32 = 0xa80;
const OFFSET_ORIYA: i32 = 0xb00;
const OFFSET_TAMIL: i32 = 0xb80;
const OFFSET_TELUGU: i32 = 0xc00;
const OFFSET_KANNADA: i32 = 0xc80;
const OFFSET_MALAYALAM: i32 = 0xd00;
const OFFSET_SINHALA: i32 = 0x0d80;
const OFFSET_THAI: i32 = 0x0e00;
const OFFSET_LAO: i32 = 0x0e80;
const OFFSET_TIBET: i32 = 0x0f00;
const OFFSET_MYANMAR: i32 = 0x1000;
const OFFSET_GEORGIAN: i32 = 0x10a0;
const OFFSET_KOREAN: i32 = 0x1100;
const OFFSET_ETHIOPIC: i32 = 0x1200;
const OFFSET_SHAVIAN: i32 = 0x10450;

const fn alphabet(
    name: &'static [u8],
    offset: i32,
    range_min: u32,
    range_max: u32,
    language: i32,
    flags: i32,
) -> Alphabet {
    Alphabet { name, offset, range_min, range_max, language, flags }
}

// character ranges must be listed in ascending unicode order
pub static ALPHABETS: &[Alphabet] = &[
    alphabet(b"_el", OFFSET_GREEK, 0x380, 0x3ff, l2(b'e', b'l'), AL_DONT_NAME | AL_NOT_LETTERS | AL_WORDS),
    alphabet(b"_cyr", OFFSET_CYRILLIC, 0x400, 0x52f, 0, 0),
    alphabet(b"_hy", OFFSET_ARMENIAN, 0x530, 0x58f, l2(b'h', b'y'), AL_WORDS),
    alphabet(b"_he", OFFSET_HEBREW, 0x590, 0x5ff, 0, 0),
    alphabet(b"_ar", OFFSET_ARABIC, 0x600, 0x6ff, 0, 0),
    alphabet(b"_syc", OFFSET_SYRIAC, 0x700, 0x74f, 0, 0),
    alphabet(b"_hi", OFFSET_DEVANAGARI, 0x900, 0x97f, l2(b'h', b'i'), AL_WORDS),
    alphabet(b"_bn", OFFSET_BENGALI, 0x0980, 0x9ff, l2(b'b', b'n'), AL_WORDS),
    alphabet(b"_gur", OFFSET_GURMUKHI, 0xa00, 0xa7f, l2(b'p', b'a'), AL_WORDS),
    alphabet(b"_gu", OFFSET_GUJARATI, 0xa80, 0xaff, l2(b'g', b'u'), AL_WORDS),
    alphabet(b"_or", OFFSET_ORIYA, 0xb00, 0xb7f, 0, 0),
    alphabet(b"_ta", OFFSET_TAMIL, 0xb80, 0xbff, l2(b't', b'a'), AL_WORDS),
    alphabet(b"_te", OFFSET_TELUGU, 0xc00, 0xc7f, l2(b't', b'e'), 0),
    alphabet(b"_kn", OFFSET_KANNADA, 0xc80, 0xcff, l2(b'k', b'n'), AL_WORDS),
    alphabet(b"_ml", OFFSET_MALAYALAM, 0xd00, 0xd7f, l2(b'm', b'l'), AL_WORDS),
    alphabet(b"_si", OFFSET_SINHALA, 0xd80, 0xdff, l2(b's', b'i'), AL_WORDS),
    alphabet(b"_th", OFFSET_THAI, 0xe00, 0xe7f, 0, 0),
    alphabet(b"_lo", OFFSET_LAO, 0xe80, 0xeff, 0, 0),
    alphabet(b"_ti", OFFSET_TIBET, 0xf00, 0xfff, 0, 0),
    alphabet(b"_my", OFFSET_MYANMAR, 0x1000, 0x109f, 0, 0),
    alphabet(b"_ka", OFFSET_GEORGIAN, 0x10a0, 0x10ff, l2(b'k', b'a'), AL_WORDS),
    alphabet(b"_ko", OFFSET_KOREAN, 0x1100, 0x11ff, l2(b'k', b'o'), AL_WORDS),
    alphabet(b"_eth", OFFSET_ETHIOPIC, 0x1200, 0x139f, 0, 0),
    alphabet(b"_braille", 0x2800, 0x2800, 0x28ff, 0, AL_NO_SYMBOL),
    alphabet(b"_ja", 0x3040, 0x3040, 0x30ff, 0, AL_NOT_CODE),
    alphabet(b"_zh", 0x3100, 0x3100, 0x9fff, 0, AL_NOT_CODE),
    alphabet(b"_ko", 0xa700, 0xa700, 0xd7ff, l2(b'k', b'o'), AL_NOT_CODE | AL_WORDS),
    alphabet(b"_shaw", OFFSET_SHAVIAN, 0x10450, 0x1047F, l2(b'e', b'n'), 0),
];

/// `AlphabetFromChar`. C compares `int c` with the unsigned range bounds, so negative `c` never
/// matches.
pub fn alphabet_from_char(c: i32) -> Option<&'static Alphabet> {
    // Find the alphabet from a character.
    let c = c as u32;
    for alphabet in ALPHABETS {
        if c <= alphabet.range_max {
            if c >= alphabet.range_min {
                return Some(alphabet);
            } else {
                break;
            }
        }
    }
    None
}

/// The alphabets that name a language espeak switches to for their letters (`ALPHABETS` entries
/// with a `language`): each one's character range and that language's code.
pub fn alphabet_languages() -> Vec<(char, char, String)> {
    ALPHABETS
        .iter()
        .filter(|a| a.language != 0)
        .filter_map(|a| {
            let language = String::from_utf8_lossy(&word_to_string2(a.language)).into_owned();
            Some((char::from_u32(a.range_min)?, char::from_u32(a.range_max)?, language))
        })
        .collect()
}

fn set_letter_vowel(tr: &mut Translator, c: usize) {
    tr.letter_bits[c] = (tr.letter_bits[c] & 0x40) | 0x81; // keep value for group 6 (front vowels e,i,y)
}

fn reset_letter_bits(tr: &mut Translator, groups: i32) {
    // Clear all the specified groups
    let mask = !groups as u8;
    for b in tr.letter_bits.iter_mut() {
        *b &= mask;
    }
}

fn set_letter_bits(tr: &mut Translator, group: i32, string: &[u8]) {
    let bits = (1i64 << group) as u8;
    for &c in string.iter().take_while(|&&c| c != 0) {
        tr.letter_bits[c as usize] |= bits;
    }
}

fn set_letter_bits_range(tr: &mut Translator, group: i32, first: usize, last: usize) {
    let bits = (1i64 << group) as u8;
    for ix in first..=last {
        tr.letter_bits[ix] |= bits;
    }
}

fn set_letter_bits_utf8(tr: &mut Translator, group: i32, letters: &[u8], offset: i32) {
    // Add the letters to the specified letter group.
    let mut p = 0isize;
    let mut code = -1;
    while code != 0 {
        let (c, bytes) = utf8_in(letters, p);
        code = c;
        if code > 0x20
            && let Some(b) = tr.letter_bits.get_mut((code - offset) as usize)
        {
            *b |= (1i64 << group) as u8;
        }
        p += bytes;
    }
}

// ignore these characters
static CHARS_IGNORE_DEFAULT: [u16; 6] = [
    // U+00AD SOFT HYPHEN
    0xad, 1, // U+200C ZERO WIDTH NON-JOINER
    0x200c, 1, // U+200D ZERO WIDTH JOINER is not ignored.
    // End of the ignored character list.
    0, 0,
];

// alternatively, ignore characters but allow zero-width-non-joiner (lang-fa)
static CHARS_IGNORE_ZWNJ_HYPHEN: [u16; 8] = [
    // U+00AD SOFT HYPHEN
    0xad,
    1,
    // U+0640 TATWEEL (KASHIDA)
    0x640,
    1,
    // U+200C ZERO WIDTH NON-JOINER
    0x200c,
    b'-' as u16,
    // End of the ignored character list.
    0,
    0,
];

static UTF8_ORDINAL: [u8; 2] = [0xc2, 0xba]; // masculine ordinal character, UTF-8
static UTF8_NULL: [u8; 0] = []; // null string, UTF-8

static EMPTY_WSTRING: [u32; 1] = [0];
static PUNCT_IN_WORD: [u32; 2] = [b'\'' as u32, 0]; // allow hyphen within words

// Translates character codes in the range transpose_min to transpose_max to
// a number in the range 1 to 63.  0 indicates there is no translation.
// Used up to 57 (max of 63)
#[rustfmt::skip]
static TRANSPOSE_MAP_LATIN: [i8; 288] = [
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, // 0x60
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,  0,  0,  0,  0,  0, // 0x70
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x80
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x90
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xa0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xb0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xc0
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0xd0
    27, 28, 29,  0,  0, 30, 31, 32, 33, 34, 35, 36,  0, 37, 38,  0, // 0xe0
     0,  0,  0, 39,  0,  0, 40,  0, 41,  0, 42,  0, 43,  0,  0,  0, // 0xf0
     0,  0,  0, 44,  0, 45,  0, 46,  0,  0,  0,  0,  0, 47,  0,  0, // 0x100
     0, 48,  0,  0,  0,  0,  0,  0,  0, 49,  0,  0,  0,  0,  0,  0, // 0x110
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x120
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x130
     0,  0, 50,  0, 51,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x140
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 52,  0,  0,  0,  0, // 0x150
     0, 53,  0, 54,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x160
     0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 55,  0, 56,  0, 57,  0, // 0x170
];

#[allow(clippy::field_reassign_with_default)] // keeps the C's assignment order
fn new_translator() -> Translator {
    let mut langopts = LanguageOptions::default();
    langopts.max_lengthmod = 500;
    langopts.lengthen_tonic = 20;

    langopts.stress_rule = STRESSPOSN_2R;
    langopts.unstressed_wd1 = 1;
    langopts.unstressed_wd2 = 3;
    langopts.param[LOPT_SONORANT_MIN] = 95;
    langopts.param[LOPT_LONG_VOWEL_THRESHOLD] = 190 / 2;
    langopts.param[LOPT_MAXAMP_EOC] = 19;
    langopts.param[LOPT_UNPRONOUNCABLE] = b's' as i32; // don't count this character at start of word
    langopts.param[LOPT_BRACKET_PAUSE] = 4; // pause at bracket
    langopts.param[LOPT_BRACKET_PAUSE_ANNOUNCED] = 2; // pauses when announcing bracket names
    langopts.spelling_stress = false;
    langopts.max_initial_consonants = 3;
    langopts.replace_chars = None;
    langopts.alt_alphabet_lang = l2(b'e', b'n');
    langopts.roman_suffix = &UTF8_NULL;
    langopts.lowercase_sentence = false;

    langopts.long_stop = 100;

    langopts.max_roman = 49;
    langopts.min_roman = 2;
    langopts.thousands_sep = b',' as i32;
    langopts.decimal_sep = b'.' as i32;
    langopts.numbers = NUM_DEFAULT;
    langopts.break_numbers = BREAK_THOUSANDS;
    langopts.max_digits = 14;

    let mut tr = Translator {
        langopts,
        translator_name: 0,
        transpose_max: 0x17f,
        transpose_min: 0x60,
        transpose_map: Some(&TRANSPOSE_MAP_LATIN),
        dictionary_name: Vec::new(),
        phonemes_repeat: [0; 20],
        phonemes_repeat_count: 0,
        phoneme_tab_ix: 0,
        dict_condition: 0,
        dict_min_size: 0,
        char_plus_apostrophe: &EMPTY_WSTRING,
        punct_within_word: &PUNCT_IN_WORD,
        chars_ignore: &CHARS_IGNORE_DEFAULT,
        // only need lower case
        letter_bits: [0; 256],
        letter_bits_offset: 0,
        letter_groups: [None; 8],
        dict: None,
        frequent_pairs: None,
        expect_verb: 0,
        expect_past: 0,
        expect_verb_s: 0,
        expect_noun: 0,
        prev_last_stress: 0,
        clause_end: 0,
        word_vowel_count: 0,
        word_stressed_count: 0,
        clause_upper_count: 0,
        clause_lower_count: 0,
        prepause_timeout: 0,
        end_stressed_vowel: 0,
        prev_dict_flags: [0, 0],
        clause_terminator: 0,
    };

    // 0-6 sets of characters matched by A B C H F G Y  in pronunciation rules
    // these may be set differently for different languages
    set_letter_bits(&mut tr, 0, b"aeiou"); // A  vowels, except y
    set_letter_bits(&mut tr, 1, b"bcdfgjklmnpqstvxz"); // B  hard consonants, excluding h,r,w
    set_letter_bits(&mut tr, 2, b"bcdfghjklmnpqrstvwxz"); // C  all consonants
    set_letter_bits(&mut tr, 3, b"hlmnr"); // H  'soft' consonants
    set_letter_bits(&mut tr, 4, b"cfhkpqstx"); // F  voiceless consonants
    set_letter_bits(&mut tr, 5, b"bdgjlmnrvwyz"); // G voiced
    set_letter_bits(&mut tr, 6, b"eiy"); // Letter group Y, front vowels
    set_letter_bits(&mut tr, 7, b"aeiouy"); // vowels, including y

    tr
}

// common letter pairs, encode these as a single byte
//  2 bytes, using the transposed character codes
static PAIRS_RU: [i16; 30] = [
    0x010c, //  ла   21052  0x23
    0x010e, //  на   18400
    0x0113, //  та   14254
    0x0301, //  ав   31083
    0x030f, //  ов   13420
    0x060e, //  не   21798
    0x0611, //  ре   19458
    0x0903, //  ви   16226
    0x0b01, //  ак   14456
    0x0b0f, //  ок   17836
    0x0c01, //  ал   13324
    0x0c09, //  ил   16877
    0x0e01, //  ан   15359
    0x0e06, //  ен   13543  0x30
    0x0e09, //  ин   17168
    0x0e0e, //  нн   15973
    0x0e0f, //  он   22373
    0x0e1c, //  ын   15052
    0x0f03, //  во   24947
    0x0f11, //  ро   13552
    0x0f12, //  со   16368
    0x100f, //  оп   19054
    0x1011, //  рп   17067
    0x1101, //  ар   23967
    0x1106, //  ер   18795
    0x1109, //  ир   13797
    0x110f, //  ор   21737
    0x1213, //  тс   25076
    0x1220, //  яс   14310
    0x7fff,
];

// (also kazakh) offset by 0x420 -- а е ё и о у ы э ю я ә ө ұ ү і
static RU_VOWELS: [u8; 16] =
    [0x10, 0x15, 0x31, 0x18, 0x1e, 0x23, 0x2b, 0x2d, 0x2e, 0x2f, 0xb9, 0xc9, 0x91, 0x8f, 0x36, 0];
// б в г д ж з й к л м н п р с т ф х ц ч ш щ ъ ь қ ң һ
static RU_CONSONANTS: [u8; 28] = [
    0x11, 0x12, 0x13, 0x14, 0x16, 0x17, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1f, 0x20, 0x21, 0x22, 0x24, 0x25, 0x26, 0x27,
    0x28, 0x29, 0x2a, 0x2c, 0x73, 0x7b, 0x83, 0x9b, 0,
];

fn set_arabic_letters(tr: &mut Translator) {
    let arab_vowel_letters = "\u{64e}  \u{64f}  \u{650}";
    let arab_consonant_vowel_letters = "\u{627} \u{648} \u{64a}";
    let arab_consonant_letters = "\u{628} \u{67e} \u{62a} \u{629} \u{62b} \u{62c} \u{62d} \u{62e} \u{62f} \u{630} \u{631} \u{632} \u{633} \u{634} \u{635} \u{636} \u{637} \u{638} \u{639} \u{63a} \u{641} \u{642} \u{643} \u{644} \u{645} \u{646} \u{626} \u{624} \u{621} \u{623} \u{622} \u{625} \u{647}";
    let arab_thick_letters = "\u{635} \u{636} \u{637} \u{638}";
    // all three are shadda in C
    let arab_shadda_letter = " \u{651} ";
    let arab_hamza_letter = " \u{651} ";
    let arab_sukun_letter = " \u{651} ";

    set_letter_bits_utf8(tr, LETTERGP_A, arab_vowel_letters.as_bytes(), OFFSET_ARABIC);
    set_letter_bits_utf8(tr, LETTERGP_B, arab_consonant_vowel_letters.as_bytes(), OFFSET_ARABIC);
    set_letter_bits_utf8(tr, LETTERGP_C, arab_consonant_letters.as_bytes(), OFFSET_ARABIC);
    set_letter_bits_utf8(tr, LETTERGP_F, arab_thick_letters.as_bytes(), OFFSET_ARABIC);
    set_letter_bits_utf8(tr, LETTERGP_G, arab_shadda_letter.as_bytes(), OFFSET_ARABIC);
    set_letter_bits_utf8(tr, LETTERGP_H, arab_hamza_letter.as_bytes(), OFFSET_ARABIC);
    set_letter_bits_utf8(tr, LETTERGP_Y, arab_sukun_letter.as_bytes(), OFFSET_ARABIC);
}

fn set_cyrillic_letters(tr: &mut Translator) {
    // Set letter types for Cyrillic script languages: bg (Bulgarian), ru (Russian), tt (Tatar), uk (Ukrainian).

    // character codes offset by 0x420
    static CYRL_SOFT: [u8; 5] = [0x2c, 0x19, 0x27, 0x29, 0]; // letter group B  [k ts; s;] -- ь й ч щ
    static CYRL_HARD: [u8; 5] = [0x2a, 0x16, 0x26, 0x28, 0]; // letter group H  [S Z ts] -- ъ ж ц ш
    static CYRL_NOTHARD: [u8; 20] = [
        0x11, 0x12, 0x13, 0x14, 0x17, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1f, 0x20, 0x21, 0x22, 0x24, 0x25, 0x27, 0x29,
        0x2c, 0,
    ]; // б в г д з й к л м н п р с т ф х ч щ ь
    static CYRL_VOICED: [u8; 7] = [0x11, 0x12, 0x13, 0x14, 0x16, 0x17, 0]; // letter group G  (voiced obstruents) -- б в г д ж з
    static CYRL_IVOWELS: [u8; 5] = [0x2c, 0x2e, 0x2f, 0x31, 0]; // letter group Y  (iotated vowels & soft-sign) -- ь ю я ё
    tr.transpose_min = 0x430; // convert cyrillic from unicode into range 0x01 to 0x22
    tr.transpose_max = 0x451;
    tr.transpose_map = None;
    tr.frequent_pairs = Some(&PAIRS_RU);

    tr.letter_bits_offset = OFFSET_CYRILLIC;
    tr.letter_bits = [0; 256];
    set_letter_bits(tr, LETTERGP_A, &RU_VOWELS);
    set_letter_bits(tr, LETTERGP_B, &CYRL_SOFT);
    set_letter_bits(tr, LETTERGP_C, &RU_CONSONANTS);
    set_letter_bits(tr, LETTERGP_H, &CYRL_HARD);
    set_letter_bits(tr, LETTERGP_F, &CYRL_NOTHARD);
    set_letter_bits(tr, LETTERGP_G, &CYRL_VOICED);
    set_letter_bits(tr, LETTERGP_Y, &CYRL_IVOWELS);
    set_letter_bits(tr, LETTERGP_VOWEL2, &RU_VOWELS);
}

fn set_indic_letters(tr: &mut Translator) {
    // Set letter types for Devanagari (Indic) script languages: Devanagari, Tamill, etc.

    static DEVA_CONSONANTS2: [u8; 15] =
        [0x02, 0x03, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x7b, 0x7c, 0x7e, 0x7f, 0];
    static DEVA_VOWELS2: [u8; 8] = [0x60, 0x61, 0x55, 0x56, 0x57, 0x62, 0x63, 0]; // non-consecutive vowels and vowel-signs

    tr.letter_bits = [0; 256];
    set_letter_bits_range(tr, LETTERGP_A, 0x04, 0x14); // vowel letters
    set_letter_bits_range(tr, LETTERGP_A, 0x3e, 0x4d); // + vowel signs, and virama
    set_letter_bits(tr, LETTERGP_A, &DEVA_VOWELS2); // + extra vowels and vowel signs

    set_letter_bits_range(tr, LETTERGP_B, 0x3e, 0x4d); // vowel signs, and virama
    set_letter_bits(tr, LETTERGP_B, &DEVA_VOWELS2); // + extra vowels and vowel signs

    set_letter_bits_range(tr, LETTERGP_C, 0x15, 0x39); // the main consonant range
    set_letter_bits(tr, LETTERGP_C, &DEVA_CONSONANTS2); // + additional consonants

    set_letter_bits_range(tr, LETTERGP_Y, 0x04, 0x14); // vowel letters
    set_letter_bits_range(tr, LETTERGP_Y, 0x3e, 0x4c); // + vowel signs
    set_letter_bits(tr, LETTERGP_Y, &DEVA_VOWELS2); // + extra vowels and vowel signs

    tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
    tr.langopts.suffix_add_e = tr.letter_bits_offset + 0x4d; // virama
}

/// `SelectTranslator`. `name` is read as a C string (up to the first 0 byte).
pub fn select_translator(name: &[u8]) -> Translator {
    let name = cstr(name, 0);
    let mut name2: i32 = 0;

    let mut tr = new_translator();
    tr.dictionary_name = name.to_vec();

    // convert name string into a word of up to 4 characters, for the switch()
    for &c in name {
        name2 = (name2 << 8).wrapping_add(c as i8 as i32);
    }

    // `case L('x','y')` is matched on name2's bytes with leading zero bytes dropped, which is a
    // one-to-one mapping of name2.
    let key = name2.to_be_bytes();
    let first = key.iter().position(|&b| b != 0).unwrap_or(key.len());

    match &key[first..] {
        b"mi" | b"my" | b"piqd" | b"py" | b"qu" | b"quc" | b"th" | b"uz" => {
            tr.langopts.numbers = 0; // disable numbers until the definition are complete in _list file
        }
        b"af" => {
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.vowel_pause = 0x30;
            tr.langopts.param[LOPT_DIERESES] = 1;
            tr.langopts.param[LOPT_PREFIXES] = 1;
            set_letter_vowel(&mut tr, b'y' as usize); // add 'y' to vowels

            tr.langopts.numbers = NUM_SWAP_TENS | NUM_HUNDRED_AND | NUM_SINGLE_AND | NUM_ROMAN | NUM_1900;
            tr.langopts.accents = 1;
        }
        b"am" => {
            // Amharic, Ethiopia
            tr.letter_bits_offset = OFFSET_ETHIOPIC;
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM; // don't use secondary stress
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            tr.langopts.numbers = NUM_OMIT_1_HUNDRED;
        }
        b"ar" => {
            // Arabic
            tr.transpose_min = OFFSET_ARABIC; // for ar_list, use 6-bit character codes
            tr.transpose_max = 0x65f;
            tr.transpose_map = None;
            tr.letter_bits_offset = OFFSET_ARABIC;
            tr.langopts.numbers = NUM_SWAP_TENS
                | NUM_AND_UNITS
                | NUM_HUNDRED_AND
                | NUM_OMIT_1_HUNDRED
                | NUM_AND_HUNDRED
                | NUM_THOUSAND_AND
                | NUM_OMIT_1_THOUSAND;
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            set_arabic_letters(&mut tr);
        }
        b"be" => {
            // Belarusian
            // offset by 0x420 -- а е ё о у ы э ю я і
            static VOWELS_BE: [u32; 11] = [0x10, 0x15, 0x31, 0x1e, 0x23, 0x2b, 0x2d, 0x2e, 0x2f, 0x36, 0];
            // б в г д ж з й к л м н п р с т ф х ц ч ш ў
            static CONSONANTS_BE: [u8; 22] = [
                0x11, 0x12, 0x13, 0x14, 0x16, 0x17, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1f, 0x20, 0x21, 0x22, 0x24, 0x25,
                0x26, 0x27, 0x28, 0x3e, 0,
            ];
            // C passes the wchar_t array as `char *`, so SetLetterBits sees its little-endian
            // bytes and stops at the first zero byte: only 0x10 is set.
            let vowels_be: Vec<u8> = VOWELS_BE.iter().flat_map(|w| w.to_le_bytes()).collect();

            tr.langopts.stress_flags = S_NO_AUTO_2 | S_NO_DIM; // don't use secondary stress
            tr.letter_bits_offset = OFFSET_CYRILLIC;
            tr.transpose_min = 0x430; // convert cyrillic from unicode into range 0x01 to 0x2f
            tr.transpose_max = 0x45e;
            tr.letter_bits = [0; 256];
            set_letter_bits(&mut tr, LETTERGP_A, &vowels_be);
            set_letter_bits(&mut tr, LETTERGP_C, &CONSONANTS_BE);
            set_letter_bits(&mut tr, LETTERGP_VOWEL2, &vowels_be);

            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED;
            tr.langopts.numbers2 = NUM2_THOUSANDPLEX_VAR_THOUSANDS | NUM2_THOUSANDS_VAR1; // variant numbers before thousands
        }
        b"bg" => {
            // Bulgarian
            set_cyrillic_letters(&mut tr);
            set_letter_vowel(&mut tr, 0x2a);
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 0x432; // [v]  don't count this character at start of word
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x107; // devoice at end of word, and change voicing to match a following consonant (except v)
            tr.langopts.param[LOPT_REDUCE] = 2;
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_ALLOW_SPACE
                | NUM_OMIT_1_HUNDRED
                | NUM_HUNDRED_AND
                | NUM_AND_UNITS
                | NUM_SINGLE_AND
                | NUM_ROMAN
                | NUM_ROMAN_ORDINAL
                | NUM_ROMAN_CAPITALS;
            tr.langopts.thousands_sep = b' ' as i32; // don't allow dot as thousands separator
        }
        b"bn" | b"as" | b"bpy" => {
            // Bengali, Assamese, Manipuri  (temporary placement - it's not indo-european)
            static BN_CONSONANTS2: [u8; 3] = [0x70, 0x71, 0];

            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_MID_DIM | S_FINAL_DIM; // use 'diminished' for unstressed final syllable
            tr.letter_bits_offset = OFFSET_BENGALI;
            set_indic_letters(&mut tr); // call this after setting OFFSET_BENGALI
            set_letter_bits_range(&mut tr, LETTERGP_B, 0x01, 0x01); // candranindu
            set_letter_bits_range(&mut tr, LETTERGP_F, 0x3e, 0x4c); // vowel signs, but not virama
            set_letter_bits(&mut tr, LETTERGP_C, &BN_CONSONANTS2);

            tr.langopts.numbers = NUM_SWAP_TENS;
            tr.langopts.break_numbers = BREAK_LAKH_BN;

            if name2 == l3(b'b', b'p', b'y') {
                tr.langopts.numbers = NUM_DEFAULT;
                tr.langopts.numbers2 = NUM2_SWAP_THOUSANDS;
            }
        }
        b"chr" => {
            // Cherokee
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            tr.langopts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM | S_FINAL_DIM_ONLY | S_EO_CLAUSE1;
        }
        b"cy" => {
            // Welsh
            tr.langopts.stress_rule = STRESSPOSN_2R;

            // 'diminished' is an unstressed final syllable
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.unstressed_wd1 = 0;
            tr.langopts.unstressed_wd2 = 2;
            tr.langopts.param[LOPT_SONORANT_MIN] = 120; // limit the shortening of sonorants before short vowels

            tr.langopts.numbers = NUM_OMIT_1_HUNDRED;

            set_letter_vowel(&mut tr, b'w' as usize); // add letter to vowels and remove from consonants
            set_letter_vowel(&mut tr, b'y' as usize);
        }
        b"da" => {
            // Danish
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.param[LOPT_PREFIXES] = 1;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_SWAP_TENS
                | NUM_HUNDRED_AND
                | NUM_OMIT_1_HUNDRED
                | NUM_ORDINAL_DOT
                | NUM_1900
                | NUM_ROMAN
                | NUM_ROMAN_CAPITALS
                | NUM_ROMAN_ORDINAL;
        }
        b"de" => {
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.word_gap = 0x8; // don't use linking phonemes
            tr.langopts.vowel_pause = 0x30;
            tr.langopts.param[LOPT_PREFIXES] = 1;
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x100; // devoice at end of word
            tr.langopts.param[LOPT_LONG_VOWEL_THRESHOLD] = 175 / 2;

            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_SWAP_TENS | NUM_ALLOW_SPACE | NUM_ORDINAL_DOT | NUM_ROMAN;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 2; // use de_rules for unpronouncable rules
        }
        b"en" => {
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = 0x08;
            tr.langopts.numbers = NUM_HUNDRED_AND | NUM_ROMAN | NUM_1900;
            tr.langopts.max_digits = 33;
            tr.langopts.param[LOPT_COMBINE_WORDS] = 2; // allow "mc" to cmbine with the following word
            tr.langopts.suffix_add_e = b'e' as i32;
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 2; // use en_rules for unpronouncable rules
            set_letter_bits(&mut tr, 6, b"aeiouy"); // Group Y: vowels, including y
        }
        b"el" | b"grc" => {
            // Greek, Ancient Greek
            // character codes offset by 0x380
            static EL_VOWELS: [u8; 20] = [
                0x10, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x35, 0x37, 0x39, 0x3f, 0x45, 0x49, 0x4a, 0x4b, 0x4c, 0x4d,
                0x4e, 0x4f, 0,
            ];
            static EL_FVOWELS: [u8; 9] = [0x2d, 0x2e, 0x2f, 0x35, 0x37, 0x39, 0x45, 0x4d, 0]; // ε η ι υ  έ ή ί ύ _
            static EL_VOICELESS: [u8; 10] = [0x38, 0x3a, 0x3e, 0x40, 0x42, 0x43, 0x44, 0x46, 0x47, 0]; // θ κ ξ π ς σ τ φ χ _
            static EL_CONSONANTS: [u8; 19] = [
                0x32, 0x33, 0x34, 0x36, 0x38, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x40, 0x41, 0x42, 0x43, 0x44, 0x46, 0x47,
                0x48, 0,
            ];
            static EL_CHAR_APOSTROPHE: [u32; 2] = [0x3c3, 0]; // σ _

            tr.char_plus_apostrophe = &EL_CHAR_APOSTROPHE;

            tr.letter_bits_offset = OFFSET_GREEK;
            tr.letter_bits = [0; 256];
            set_letter_bits(&mut tr, LETTERGP_A, &EL_VOWELS);
            set_letter_bits(&mut tr, LETTERGP_VOWEL2, &EL_VOWELS);
            set_letter_bits(&mut tr, LETTERGP_B, &EL_VOICELESS);
            set_letter_bits(&mut tr, LETTERGP_C, &EL_CONSONANTS);
            set_letter_bits(&mut tr, LETTERGP_Y, &EL_FVOWELS); // front vowels: ε η ι υ _

            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY; // mark unstressed final syllables as diminished
            tr.langopts.unstressed_wd1 = 0;
            tr.langopts.unstressed_wd2 = 2;
            tr.langopts.param[LOPT_SONORANT_MIN] = 130; // limit the shortening of sonorants before short vowels

            tr.langopts.numbers = NUM_SINGLE_STRESS | NUM_DECIMAL_COMMA;
            tr.langopts.numbers2 = NUM2_THOUSANDPLEX_VAR_THOUSANDS | NUM2_MULTIPLE_ORDINAL | NUM2_ORDINAL_NO_AND;

            if name2 == l3(b'g', b'r', b'c') {
                // ancient greek
                tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1;
            }
        }
        b"eo" => {
            static EO_CHAR_APOSTROPHE: [u32; 2] = [b'l' as u32, 0];

            tr.char_plus_apostrophe = &EO_CHAR_APOSTROPHE;

            tr.langopts.vowel_pause = 2;
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.unstressed_wd2 = 2;

            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_ALLOW_SPACE | NUM_ROMAN;
        }
        b"es" | b"an" | b"ca" | b"ia" | b"pap" => {
            // Spanish, Aragonese, Catalan, Interlingua, Papiamento
            static CA_PUNCT_WITHIN_WORD: [u32; 3] = [b'\'' as u32, 0xb7, 0]; // ca: allow middle-dot within word

            tr.langopts.stress_rule = STRESSPOSN_2R;

            // stress last syllable if it doesn't end in vowel or "s" or "n"
            // 'diminished' is an unstressed final syllable
            tr.langopts.stress_flags = S_FINAL_SPANISH | S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.unstressed_wd1 = 0;
            tr.langopts.unstressed_wd2 = 2;
            tr.langopts.param[LOPT_SONORANT_MIN] = 120; // limit the shortening of sonorants before short vowels

            tr.langopts.numbers = NUM_SINGLE_STRESS
                | NUM_DECIMAL_COMMA
                | NUM_AND_UNITS
                | NUM_OMIT_1_HUNDRED
                | NUM_OMIT_1_THOUSAND
                | NUM_ROMAN
                | NUM_ROMAN_AFTER
                | NUM_DFRACTION_4;
            tr.langopts.numbers2 = NUM2_MULTIPLE_ORDINAL | NUM2_ORDINAL_NO_AND;

            if name2 == l2(b'c', b'a') {
                // stress last syllable unless word ends with a vowel
                tr.punct_within_word = &CA_PUNCT_WITHIN_WORD;
                tr.langopts.stress_flags =
                    S_FINAL_SPANISH | S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_NO_AUTO_2 | S_FIRST_PRIMARY;
            } else if name2 == l2(b'i', b'a') {
                tr.langopts.stress_flags = S_FINAL_SPANISH | S_FINAL_DIM_ONLY | S_FINAL_NO_2;
                tr.langopts.numbers =
                    NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_OMIT_1_THOUSAND | NUM_ROMAN | NUM_ROMAN_AFTER;
            } else if name2 == l2(b'a', b'n') {
                tr.langopts.stress_flags = S_FINAL_SPANISH | S_FINAL_DIM_ONLY | S_FINAL_NO_2;
                tr.langopts.numbers = NUM_SINGLE_STRESS
                    | NUM_DECIMAL_COMMA
                    | NUM_AND_UNITS
                    | NUM_OMIT_1_HUNDRED
                    | NUM_OMIT_1_THOUSAND
                    | NUM_ROMAN
                    | NUM_ROMAN_ORDINAL;
                tr.langopts.numbers2 = NUM2_ORDINAL_NO_AND;
                tr.langopts.roman_suffix = &UTF8_ORDINAL;
            } else if name2 == l3(b'p', b'a', b'p') {
                // stress last syllable unless word ends with a vowel
                tr.langopts.stress_rule = STRESSPOSN_1R;
                tr.langopts.stress_flags = S_FINAL_VOWEL_UNSTRESSED | S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_NO_AUTO_2;
            } else {
                tr.langopts.param[LOPT_UNPRONOUNCABLE] = 2; // use es_rules for unpronouncable rules
            }
        }
        b"eu" => {
            // basque
            tr.langopts.stress_flags = S_FINAL_VOWEL_UNSTRESSED | S_MID_DIM;
            tr.langopts.param[LOPT_SUFFIX] = 1;
            tr.langopts.numbers = NUM_SINGLE_STRESS
                | NUM_DECIMAL_COMMA
                | NUM_HUNDRED_AND
                | NUM_OMIT_1_HUNDRED
                | NUM_OMIT_1_THOUSAND
                | NUM_VIGESIMAL;
        }
        b"fa" => {
            // Farsi
            // Convert characters in the range 0x620 to 0x6cc to the range 1 to 63.
            // 0 indicates no translation for this character
            #[rustfmt::skip]
            static TRANSPOSE_MAP_FA: [i8; 173] = [
                 0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15, // 0x620
                16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,  0,  0,  0,  0,  0, // 0x630
                 0, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, // 0x640
                42, 43,  0,  0, 44,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x650
                 0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x660
                 0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 45,  0, // 0x670
                 0,  0,  0,  0,  0,  0, 46,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x680
                 0,  0,  0,  0,  0,  0,  0,  0, 47,  0,  0,  0,  0,  0,  0,  0, // 0x690
                 0,  0,  0,  0,  0,  0,  0,  0,  0, 48,  0,  0,  0,  0,  0, 49, // 0x6a0
                 0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, // 0x6b0
                50,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 51,             // 0x6c0
            ];
            tr.transpose_min = 0x620;
            tr.transpose_max = 0x6cc;
            tr.transpose_map = Some(&TRANSPOSE_MAP_FA);
            tr.letter_bits_offset = OFFSET_ARABIC;

            tr.langopts.numbers = NUM_AND_UNITS | NUM_HUNDRED_AND;
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words

            tr.chars_ignore = &CHARS_IGNORE_ZWNJ_HYPHEN; // replace ZWNJ by hyphen
        }
        b"et" | b"fi" => {
            // Estonian (only sets the encoding, then falls through), Finnish
            tr.langopts.long_stop = 130;

            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_ALLOW_SPACE | NUM_DFRACTION_2 | NUM_ORDINAL_DOT;
            set_letter_vowel(&mut tr, b'y' as usize);
        }
        b"fo" => {
            // Faroese
            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_SWAP_TENS
                | NUM_HUNDRED_AND
                | NUM_OMIT_1_HUNDRED
                | NUM_ORDINAL_DOT
                | NUM_1900
                | NUM_ROMAN
                | NUM_ROMAN_CAPITALS
                | NUM_ROMAN_ORDINAL;
        }
        b"fr" => {
            // french
            tr.langopts.stress_rule = STRESSPOSN_1R; // stress on final syllable
            tr.langopts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM; // don't use secondary stress
            tr.langopts.param[LOPT_IT_LENGTHEN] = 1; // remove lengthen indicator from unstressed syllables
            tr.langopts.accents = 2; // Say "Capital" after the letter.

            tr.langopts.numbers = NUM_SINGLE_STRESS
                | NUM_DECIMAL_COMMA
                | NUM_ALLOW_SPACE
                | NUM_OMIT_1_HUNDRED
                | NUM_NOPAUSE
                | NUM_ROMAN
                | NUM_ROMAN_CAPITALS
                | NUM_ROMAN_AFTER
                | NUM_VIGESIMAL
                | NUM_DFRACTION_4;
            set_letter_vowel(&mut tr, b'y' as usize);
        }
        b"hak" => {
            // Hakka Chinese
            tr.langopts.stress_flags = S_NO_DIM; // don't automatically set diminished stress (may be set in the intonation module)
            tr.langopts.tone_language = 1; // Tone language, use  CalcPitches_Tone() rather than CalcPitches()
            tr.langopts.tone_numbers = 1; // a number after letters indicates a tone number (eg. pinyin or jyutping)
            tr.langopts.ideographs = 1;
        }
        b"ga" | b"gd" => {
            // irish, scots gaelic
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_NO_AUTO_2; // don't use secondary stress
            tr.langopts.numbers = NUM_OMIT_1_HUNDRED | NUM_OMIT_1_THOUSAND;
            tr.langopts.accents = 2; // 'capital' after letter name
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 3; // don't count apostrophe
            tr.langopts.param[LOPT_IT_LENGTHEN] = 1; // remove [:] phoneme from non-stressed syllables (Lang=gd)
        }
        b"gn" => {
            // guarani
            tr.langopts.stress_rule = STRESSPOSN_1R; // stress on final syllable
        }
        b"hi" | b"ne" | b"or" | b"pa" | b"gu" | b"mr" => {
            // Hindi, Nepali, Oriya, Punjabi, Gujarati, Marathi
            tr.langopts.stress_rule = STRESSPOSN_1RH; // stress on last heaviest syllable, excluding final syllable
            tr.langopts.stress_flags = S_MID_DIM | S_FINAL_DIM; // use 'diminished' for unstressed final syllable
            tr.langopts.numbers = NUM_SWAP_TENS;
            tr.langopts.break_numbers = BREAK_LAKH_HI;
            tr.letter_bits_offset = OFFSET_DEVANAGARI;

            if name2 == l2(b'p', b'a') {
                tr.letter_bits_offset = OFFSET_GURMUKHI;
            } else if name2 == l2(b'g', b'u') {
                tr.letter_bits_offset = OFFSET_GUJARATI;
                tr.langopts.stress_rule = STRESSPOSN_2R;
            } else if name2 == l2(b'n', b'e') {
                tr.langopts.break_numbers = BREAK_LAKH;
                tr.langopts.max_digits = 22;
                tr.langopts.numbers2 |= NUM2_ENGLISH_NUMERALS;
            } else if name2 == l2(b'o', b'r') {
                tr.letter_bits_offset = OFFSET_ORIYA;
            }
            set_indic_letters(&mut tr);
        }
        b"hr" | b"bs" | b"sr" => {
            // Croatian, Bosnian, Serbian
            tr.dictionary_name = b"hbs".to_vec();

            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_NO_2;
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x3;
            tr.langopts.max_initial_consonants = 5;
            tr.langopts.spelling_stress = true;
            tr.langopts.accents = 1;

            tr.langopts.numbers = NUM_SINGLE_STRESS
                | NUM_HUNDRED_AND
                | NUM_OMIT_1_HUNDRED
                | NUM_DECIMAL_COMMA
                | NUM_THOUS_SPACE
                | NUM_DFRACTION_2
                | NUM_ROMAN_CAPITALS;
            tr.langopts.numbers2 =
                NUM2_THOUSANDPLEX_VAR_THOUSANDS | NUM2_THOUSANDPLEX_VAR_MILLIARDS | NUM2_THOUSANDS_VAR5;
            tr.langopts.our_alphabet = OFFSET_CYRILLIC; // don't say "cyrillic" before letter names

            set_letter_vowel(&mut tr, b'y' as usize);
            set_letter_vowel(&mut tr, b'r' as usize);
        }
        b"ht" => {
            // Haitian Creole
            tr.langopts.stress_rule = STRESSPOSN_1R; // stress on final syllable
            tr.langopts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM; // don't use secondary stress
            tr.langopts.numbers =
                NUM_SINGLE_STRESS | NUM_OMIT_1_HUNDRED | NUM_NOPAUSE | NUM_ROMAN | NUM_VIGESIMAL | NUM_DFRACTION_4;
        }
        b"hu" => {
            // Hungarian
            tr.langopts.vowel_pause = 0x20;
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_NO_AUTO_2 | 0x8000 | S_HYPEN_UNSTRESS;
            tr.langopts.unstressed_wd1 = 2;
            tr.langopts.param[LOPT_ANNOUNCE_PUNCT] = 2; // don't break clause before announcing . ? !

            tr.langopts.numbers = NUM_DFRACTION_5
                | NUM_ALLOW_SPACE
                | NUM_ROMAN
                | NUM_ROMAN_ORDINAL
                | NUM_ROMAN_CAPITALS
                | NUM_ORDINAL_DOT
                | NUM_OMIT_1_HUNDRED
                | NUM_OMIT_1_THOUSAND;
            tr.langopts.thousands_sep = b' ' as i32; // don't allow dot as thousands separator
            tr.langopts.decimal_sep = b',' as i32;
            tr.langopts.max_roman = 899;
            tr.langopts.min_roman = 1;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.spelling_stress = true;
        }
        b"hy" => {
            // Armenian
            static HY_VOWELS: [u8; 8] = [0x31, 0x35, 0x37, 0x38, 0x3b, 0x48, 0x55, 0];
            static HY_CONSONANTS: [u8; 31] = [
                0x32, 0x33, 0x34, 0x36, 0x39, 0x3a, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x46, 0x47,
                0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x56, 0,
            ];
            static HY_CONSONANTS2: [u8; 2] = [0x45, 0];

            tr.langopts.stress_rule = STRESSPOSN_1R; // default stress on final syllable

            tr.letter_bits_offset = OFFSET_ARMENIAN;
            tr.letter_bits = [0; 256];
            set_letter_bits(&mut tr, LETTERGP_A, &HY_VOWELS);
            set_letter_bits(&mut tr, LETTERGP_VOWEL2, &HY_VOWELS);
            set_letter_bits(&mut tr, LETTERGP_B, &HY_CONSONANTS); // not including 'j'
            set_letter_bits(&mut tr, LETTERGP_C, &HY_CONSONANTS);
            set_letter_bits(&mut tr, LETTERGP_C, &HY_CONSONANTS2); // add 'j'
            tr.langopts.max_initial_consonants = 6;
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_ALLOW_SPACE | NUM_OMIT_1_HUNDRED;
        }
        b"io" => {
            // Ido International Auxiliary Language
            static EO_CHAR_APOSTROPHE: [u32; 2] = [b'l' as u32, 0];

            tr.char_plus_apostrophe = &EO_CHAR_APOSTROPHE;

            tr.langopts.vowel_pause = 2;
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.unstressed_wd2 = 2;

            tr.langopts.numbers =
                NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_ALLOW_SPACE | NUM_AND_UNITS | NUM_HUNDRED_AND | NUM_ROMAN;
        }
        b"id" | b"ms" => {
            // Indonesian, Malay
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_ALLOW_SPACE | NUM_ROMAN;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.accents = 2; // "capital" after letter name
        }
        b"is" => {
            // Icelandic
            // voiceless conants, including 'þ'  ?? 's'
            static IS_LETTERGROUP_B: [u32; 9] =
                [b'c' as u32, b'f' as u32, b'h' as u32, b'k' as u32, b'p' as u32, b't' as u32, b'x' as u32, 0xfe, 0];

            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_NO_2;
            tr.langopts.param[LOPT_IT_LENGTHEN] = 0x11; // remove lengthen indicator from unstressed vowels
            tr.langopts.param[LOPT_REDUCE] = 2;

            reset_letter_bits(&mut tr, 0x18);
            set_letter_bits(&mut tr, 4, b"kpst"); // Letter group F
            set_letter_bits(&mut tr, 3, b"jvr"); // Letter group H
            tr.letter_groups[1] = Some(&IS_LETTERGROUP_B);
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_SINGLE_AND | NUM_HUNDRED_AND | NUM_AND_UNITS | NUM_1900;
            tr.langopts.numbers2 = NUM2_THOUSANDPLEX_VAR_THOUSANDS;
        }
        b"it" => {
            // Italian
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM_ONLY | S_PRIORITY_STRESS;
            tr.langopts.vowel_pause = 1;
            tr.langopts.unstressed_wd1 = 0;
            tr.langopts.unstressed_wd2 = 2;
            tr.langopts.param[LOPT_IT_LENGTHEN] = 2; // remove lengthen indicator from unstressed or non-penultimate syllables
            tr.langopts.param[LOPT_SONORANT_MIN] = 130; // limit the shortening of sonorants before short vowels
            tr.langopts.param[LOPT_REDUCE] = 1; // reduce vowels even if phonemes are specified in it_list
            tr.langopts.param[LOPT_ALT] = 2; // call ApplySpecialAttributes2() if a word has $alt or $alt2
            tr.langopts.numbers = NUM_SINGLE_VOWEL
                | NUM_OMIT_1_HUNDRED
                | NUM_DECIMAL_COMMA
                | NUM_DFRACTION_1
                | NUM_ROMAN
                | NUM_ROMAN_CAPITALS
                | NUM_ROMAN_ORDINAL;
            tr.langopts.numbers2 = NUM2_NO_TEEN_ORDINALS;
            tr.langopts.roman_suffix = &UTF8_ORDINAL;
            tr.langopts.accents = 2; // Say "Capital" after the letter.
            set_letter_vowel(&mut tr, b'y' as usize);
        }
        b"jbo" => {
            // Lojban
            // allow period and comma within a word, also stress marker (from LOPT_CAPS_IN_WORD)
            static JBO_PUNCT_WITHIN_WORD: [u32; 5] = [b'.' as u32, b',' as u32, b'\'' as u32, 0x2c8, 0];

            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.vowel_pause = 0x20c; // pause before a word which starts with a vowel, or after a word which ends in a consonant
            tr.punct_within_word = &JBO_PUNCT_WITHIN_WORD;
            tr.langopts.param[LOPT_CAPS_IN_WORD] = 1; // capitals indicate stressed syllables
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.max_lengthmod = 368;
            tr.langopts.numbers = 0; // disable numbers until the definition are complete in _list file
        }
        b"ka" => {
            // Georgian
            // character codes offset by 0x1080
            static KA_VOWELS: [u8; 8] = [0x30, 0x34, 0x38, 0x3d, 0x43, 0x55, 0x57, 0];
            static KA_CONSONANTS: [u8; 34] = [
                0x31, 0x32, 0x33, 0x35, 0x36, 0x37, 0x39, 0x3a, 0x3b, 0x3c, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x44, 0x45,
                0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x56, 0,
            ];
            tr.letter_bits = [0; 256];
            set_letter_bits(&mut tr, LETTERGP_A, &KA_VOWELS);
            set_letter_bits(&mut tr, LETTERGP_C, &KA_CONSONANTS);
            set_letter_bits(&mut tr, LETTERGP_VOWEL2, &KA_VOWELS);

            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_NO_2;
            tr.letter_bits_offset = OFFSET_GEORGIAN;
            tr.langopts.max_initial_consonants = 7;
            tr.langopts.max_digits = 32;
            tr.langopts.numbers =
                NUM_VIGESIMAL | NUM_AND_UNITS | NUM_OMIT_1_HUNDRED | NUM_OMIT_1_THOUSAND | NUM_DFRACTION_5 | NUM_ROMAN;

            tr.langopts.alt_alphabet = OFFSET_CYRILLIC;
            tr.langopts.alt_alphabet_lang = l2(b'r', b'u');
        }
        b"kk" => {
            // Kazakh
            tr.letter_bits_offset = OFFSET_CYRILLIC;
            tr.letter_bits = [0; 256];
            set_letter_bits(&mut tr, LETTERGP_A, &RU_VOWELS);
            set_letter_bits(&mut tr, LETTERGP_C, &RU_CONSONANTS);
            set_letter_bits(&mut tr, LETTERGP_VOWEL2, &RU_VOWELS);

            tr.langopts.stress_rule = STRESSPOSN_1RU; // stress on the last syllable, before any explicitly unstressed syllable
            tr.langopts.stress_flags = S_NO_AUTO_2 + S_NO_EOC_LENGTHEN; // no automatic secondary stress, don't lengthen at end-of-clause
            tr.langopts.lengthen_tonic = 0;
            tr.langopts.param[LOPT_SUFFIX] = 1;

            tr.langopts.numbers = NUM_OMIT_1_HUNDRED | NUM_DFRACTION_6;
            tr.langopts.max_initial_consonants = 2;
        }
        b"kl" => {
            // Greenlandic
            tr.langopts.stress_rule = STRESSPOSN_GREENLANDIC;
            tr.langopts.stress_flags = S_NO_AUTO_2;
            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_SWAP_TENS
                | NUM_HUNDRED_AND
                | NUM_OMIT_1_HUNDRED
                | NUM_ORDINAL_DOT
                | NUM_1900
                | NUM_ROMAN
                | NUM_ROMAN_CAPITALS
                | NUM_ROMAN_ORDINAL;
        }
        b"ko" => {
            // Korean, TEST
            static KO_IVOWELS: [u8; 9] = [0x63, 0x64, 0x67, 0x68, 0x6d, 0x72, 0x74, 0x75, 0]; // y and i vowels
            static KO_VOICED: [u8; 8] = [0x02, 0x05, 0x06, 0xab, 0xaf, 0xb7, 0xbc, 0]; // voiced consonants, l,m,n,N

            tr.letter_bits_offset = OFFSET_KOREAN;
            tr.langopts.our_alphabet = 0xa700;
            tr.letter_bits = [0; 256];
            set_letter_bits_range(&mut tr, LETTERGP_A, 0x61, 0x75);
            set_letter_bits(&mut tr, LETTERGP_Y, &KO_IVOWELS);
            set_letter_bits(&mut tr, LETTERGP_G, &KO_VOICED);

            tr.langopts.stress_rule = STRESSPOSN_2LLH; // ?? 1st syllable if it is heavy, else 2nd syllable
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            tr.langopts.numbers = NUM_OMIT_1_HUNDRED;
            tr.langopts.numbers2 = NUM2_MYRIADS;
            tr.langopts.break_numbers = BREAK_MYRIADS;
            tr.langopts.max_digits = 20;
        }
        b"ku" => {
            // Kurdish
            tr.langopts.stress_rule = STRESSPOSN_1RU; // stress on the last syllable, before any explicitly unstressed syllable

            tr.langopts.numbers = NUM_HUNDRED_AND | NUM_AND_UNITS | NUM_OMIT_1_HUNDRED | NUM_AND_HUNDRED;
            tr.langopts.max_initial_consonants = 2;
        }
        b"ky" => {
            // Kyrgyx
            tr.langopts.numbers = NUM_DEFAULT;
        }
        b"la" => {
            // Latin
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_NO_AUTO_2;
            tr.langopts.unstressed_wd1 = 0;
            tr.langopts.unstressed_wd2 = 2;
            tr.langopts.param[LOPT_DIERESES] = 1;
            tr.langopts.numbers = NUM_ROMAN;
            tr.langopts.max_roman = 5000;
        }
        b"lt" => {
            // Lithuanian
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_NO_AUTO_2;
            tr.langopts.unstressed_wd1 = 0;
            tr.langopts.unstressed_wd2 = 2;
            tr.langopts.param[LOPT_DIERESES] = 1;
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_DFRACTION_4 | NUM_ORDINAL_DOT;
            tr.langopts.numbers2 = NUM2_THOUSANDS_VAR4;
            tr.langopts.max_roman = 5000;
        }
        b"lv" | b"ltg" => {
            // latvian, latgalian
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.spelling_stress = true;
            tr.langopts.max_digits = 33;
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_DFRACTION_4 | NUM_ORDINAL_DOT;
            tr.langopts.stress_flags = S_NO_AUTO_2 | S_FINAL_DIM | S_FINAL_DIM_ONLY | S_EO_CLAUSE1;
        }
        b"mk" => {
            // Macedonian
            static VOWELS_CYRILLIC: [u32; 18] = [
                // also include 'р' [R]
                0x440, 0x430, 0x435, 0x438, 0x439, 0x43e, 0x443, 0x44b, 0x44d, 0x44e, 0x44f, 0x450, 0x451, 0x456, 0x457,
                0x45d, 0x45e, 0,
            ];

            tr.letter_groups[0] = Some(&VOWELS_CYRILLIC);
            tr.letter_groups[7] = Some(&VOWELS_CYRILLIC);
            tr.letter_bits_offset = OFFSET_CYRILLIC;

            tr.langopts.stress_rule = STRESSPOSN_3R; // antipenultimate
            tr.langopts.numbers =
                NUM_DECIMAL_COMMA | NUM_AND_UNITS | NUM_OMIT_1_HUNDRED | NUM_OMIT_1_THOUSAND | NUM_DFRACTION_2;
            tr.langopts.numbers2 =
                NUM2_THOUSANDPLEX_VAR_THOUSANDS | NUM2_THOUSANDPLEX_VAR_MILLIARDS | NUM2_THOUSANDS_VAR2;
        }
        b"mt" => {
            // Maltese
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x100; // devoice at end of word
            tr.langopts.stress_rule = STRESSPOSN_2R; // penultimate
            tr.langopts.numbers = NUM_DEFAULT;
        }
        b"nl" => {
            // Dutch
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.vowel_pause = 0x30; // ??
            tr.langopts.param[LOPT_DIERESES] = 1;
            tr.langopts.param[LOPT_PREFIXES] = 1;
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x100; // devoice at end of word
            set_letter_vowel(&mut tr, b'y' as usize);

            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_SWAP_TENS
                | NUM_OMIT_1_HUNDRED
                | NUM_OMIT_1_THOUSAND
                | NUM_ALLOW_SPACE
                | NUM_1900
                | NUM_ORDINAL_DOT;
            tr.langopts.ordinal_indicator = Some(b"e");
            tr.langopts.stress_flags = S_FIRST_PRIMARY;
        }
        b"nb" => {
            // Norwegian
            tr.langopts.stress_rule = STRESSPOSN_1L;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_HUNDRED_AND | NUM_ALLOW_SPACE | NUM_1900 | NUM_ORDINAL_DOT;
        }
        b"om" => {
            // Oromo
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2 | 0x80000;
            tr.langopts.numbers = NUM_OMIT_1_HUNDRED | NUM_HUNDRED_AND;
            tr.langopts.numbers2 = NUM2_SWAP_THOUSANDS;
        }
        b"pl" => {
            // Polish
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY; // mark unstressed final syllables as diminished
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x9;
            tr.langopts.max_initial_consonants = 7; // for example: wchrzczony :)
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_ALLOW_SPACE | NUM_DFRACTION_2;
            tr.langopts.numbers2 = NUM2_THOUSANDS_VAR3;
            tr.langopts.param[LOPT_COMBINE_WORDS] = 4 + 0x100; // combine 'nie' (marked with $alt2) with some 1-syllable (and 2-syllable) words (marked with $alt)
            set_letter_vowel(&mut tr, b'y' as usize);
        }
        b"pt" => {
            // Portuguese
            tr.langopts.stress_rule = STRESSPOSN_1R; // stress on final syllable
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_INITIAL_2 | S_PRIORITY_STRESS;
            tr.langopts.numbers =
                NUM_DECIMAL_COMMA | NUM_DFRACTION_2 | NUM_HUNDRED_AND | NUM_AND_UNITS | NUM_ROMAN_CAPITALS;
            tr.langopts.numbers2 = NUM2_MULTIPLE_ORDINAL | NUM2_NO_TEEN_ORDINALS | NUM2_ORDINAL_NO_AND;
            tr.langopts.max_roman = 5000;
            set_letter_vowel(&mut tr, b'y' as usize);
            reset_letter_bits(&mut tr, 0x2);
            set_letter_bits(&mut tr, 1, b"bcdfgjkmnpqstvxz"); // B  hard consonants, excluding h,l,r,w,y
            tr.langopts.param[LOPT_ALT] = 2; // call ApplySpecialAttributes2() if a word has $alt or $alt2
            tr.langopts.accents = 2; // 'capital' after letter name
        }
        b"ro" => {
            // Romanian
            tr.langopts.stress_rule = STRESSPOSN_1R;
            tr.langopts.stress_flags = S_FINAL_VOWEL_UNSTRESSED | S_FINAL_DIM_ONLY;

            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_ALLOW_SPACE | NUM_DFRACTION_3 | NUM_AND_UNITS | NUM_ROMAN;
            tr.langopts.numbers2 = NUM2_THOUSANDPLEX_VAR_ALL;
        }
        b"ru" => {
            // Russian
            translator_russian(&mut tr);
        }
        b"sk" | b"cs" => {
            // Slovak, Czech
            let sk_voiced = b"bdgjlmnrvwzaeiouy";

            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x3;
            tr.langopts.max_initial_consonants = 5;
            tr.langopts.spelling_stress = true;
            tr.langopts.param[LOPT_COMBINE_WORDS] = 4; // combine some prepositions with the following word

            tr.langopts.numbers = NUM_OMIT_1_HUNDRED | NUM_DFRACTION_2 | NUM_ROMAN;
            tr.langopts.numbers2 = NUM2_THOUSANDS_VAR2;
            tr.langopts.thousands_sep = 0; // no thousands separator
            tr.langopts.decimal_sep = b',' as i32;

            if name2 == l2(b'c', b's') {
                tr.langopts.numbers2 = NUM2_THOUSANDPLEX_VAR_MILLIARDS | NUM2_THOUSANDS_VAR2;
            }

            set_letter_vowel(&mut tr, b'y' as usize);
            set_letter_vowel(&mut tr, b'r' as usize);
            reset_letter_bits(&mut tr, 0x20);
            set_letter_bits(&mut tr, 5, sk_voiced);
        }
        b"si" => {
            // Sinhala
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.spelling_stress = true;

            tr.letter_bits_offset = OFFSET_SINHALA;
            tr.letter_bits = [0; 256];
            set_letter_bits_range(&mut tr, LETTERGP_A, 0x05, 0x16); // vowel letters
            set_letter_bits_range(&mut tr, LETTERGP_A, 0x4a, 0x73); // + vowel signs, and virama

            set_letter_bits_range(&mut tr, LETTERGP_B, 0x4a, 0x73); // vowel signs, and virama

            set_letter_bits_range(&mut tr, LETTERGP_C, 0x1a, 0x46); // the main consonant range

            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            tr.langopts.suffix_add_e = tr.letter_bits_offset + 0x4a; // virama
            tr.langopts.numbers = NUM_OMIT_1_THOUSAND | NUM_SINGLE_STRESS_L | NUM_DFRACTION_7;
            tr.langopts.numbers2 = NUM2_PERCENT_BEFORE;
            tr.langopts.break_numbers = BREAK_LAKH_HI;
        }
        b"sl" => {
            // Slovenian
            tr.langopts.stress_rule = STRESSPOSN_2R; // Temporary
            tr.langopts.stress_flags = S_NO_AUTO_2;
            tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x103;
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 0x76; // [v]  don't count this character at start of word
            tr.langopts.param[LOPT_ALT] = 2; // call ApplySpecialAttributes2() if a word has $alt or $alt2
            tr.langopts.param[LOPT_IT_LENGTHEN] = 1; // remove lengthen indicator from unstressed syllables
            tr.letter_bits[b'r' as usize] |= 0x80; // add 'r' to letter group 7, vowels for Unpronouncable test
            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_ALLOW_SPACE
                | NUM_SWAP_TENS
                | NUM_OMIT_1_HUNDRED
                | NUM_DFRACTION_2
                | NUM_ORDINAL_DOT
                | NUM_ROMAN;
            tr.langopts.numbers2 = NUM2_THOUSANDS_VAR4;
            tr.langopts.thousands_sep = b' ' as i32; // don't allow dot as thousands separator
        }
        b"smj" => {
            // Lule Saami
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_2_TO_HEAVY; // move secondary stress from light to a following heavy syllable
            tr.langopts.long_stop = 130;

            tr.langopts.numbers = NUM_DECIMAL_COMMA
                | NUM_ALLOW_SPACE
                | NUM_SWAP_TENS
                | NUM_OMIT_1_HUNDRED
                | NUM_DFRACTION_2
                | NUM_ORDINAL_DOT;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.spelling_stress = true;
            tr.langopts.intonation_group = 3; // less intonation, don't raise pitch at comma
        }
        b"sq" => {
            // Albanian
            tr.langopts.stress_rule = STRESSPOSN_1R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2 | S_FINAL_VOWEL_UNSTRESSED;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_HUNDRED_AND | NUM_AND_UNITS | NUM_DFRACTION_4;
            tr.langopts.accents = 2; // "capital" after letter name
        }
        b"sv" => {
            // Swedish
            tr.langopts.stress_rule = STRESSPOSN_1L;
            set_letter_vowel(&mut tr, b'y' as usize);
            tr.langopts.numbers = NUM_SINGLE_STRESS | NUM_DECIMAL_COMMA | NUM_ALLOW_SPACE | NUM_1900;
            tr.langopts.accents = 1;
        }
        b"sw" | b"tn" => {
            // Swahili, Setswana
            tr.langopts.vowel_pause = 1;
            tr.langopts.stress_rule = STRESSPOSN_2R;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2;
            tr.langopts.max_initial_consonants = 4; // for example: mwngi

            tr.langopts.numbers = NUM_AND_UNITS | NUM_HUNDRED_AND | NUM_SINGLE_AND | NUM_OMIT_1_HUNDRED;
        }
        b"ta" | b"kn" | b"ml" | b"te" => {
            // Tamil, Kannada, Malayalam, Telugu
            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.stress_flags = S_FINAL_DIM_ONLY | S_FINAL_NO_2; // use 'diminished' for unstressed final syllable
            tr.langopts.spelling_stress = true;
            tr.langopts.break_numbers = BREAK_LAKH_DV;

            if name2 == l2(b't', b'a') {
                tr.letter_bits_offset = OFFSET_TAMIL;
                tr.langopts.numbers = NUM_OMIT_1_THOUSAND;
                tr.langopts.numbers2 = NUM2_ORDINAL_AND_THOUSANDS;
                tr.langopts.param[LOPT_WORD_MERGE] = 1; // don't break vowels between words
            } else if name2 == l2(b'm', b'l') {
                tr.letter_bits_offset = OFFSET_MALAYALAM;
                tr.langopts.numbers = NUM_OMIT_1_THOUSAND | NUM_OMIT_1_HUNDRED;
                tr.langopts.numbers2 = NUM2_OMIT_1_HUNDRED_ONLY;
                tr.langopts.stress_rule = STRESSPOSN_1SL; // 1st syllable, unless 1st vowel is short and 2nd is long
            } else if name2 == l2(b'k', b'n') {
                tr.letter_bits_offset = OFFSET_KANNADA;
                tr.langopts.numbers = NUM_DEFAULT;
            } else if name2 == l2(b't', b'e') {
                tr.letter_bits_offset = OFFSET_TELUGU;
                tr.langopts.numbers = NUM_DEFAULT;
                tr.langopts.numbers2 = NUM2_ORDINAL_DROP_VOWEL;
            }
            set_indic_letters(&mut tr); // call this after setting OFFSET_
            set_letter_bits_range(&mut tr, LETTERGP_B, 0x4e, 0x4e); // chillu-virama (unofficial)
        }
        b"tr" | b"az" => {
            // Turkish, Azerbaijan
            tr.langopts.stress_rule = STRESSPOSN_1RU; // stress on the last syllable, before any explicitly unstressed syllable
            tr.langopts.stress_flags = S_NO_AUTO_2; // no automatic secondary stress
            tr.langopts.dotless_i = 1;
            tr.langopts.param[LOPT_SUFFIX] = 1;

            if name2 == l2(b'a', b'z') {
                tr.langopts.numbers = NUM_SINGLE_STRESS
                    | NUM_DECIMAL_COMMA
                    | NUM_ALLOW_SPACE
                    | NUM_OMIT_1_HUNDRED
                    | NUM_OMIT_1_THOUSAND
                    | NUM_DFRACTION_2;
            } else {
                tr.langopts.numbers =
                    NUM_SINGLE_STRESS | NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_OMIT_1_THOUSAND | NUM_DFRACTION_2;
            }
            tr.langopts.max_initial_consonants = 2;
        }
        b"tt" => {
            // Tatar
            set_cyrillic_letters(&mut tr);
            tr.langopts.stress_rule = STRESSPOSN_1R; // stress on final syllable
            tr.langopts.stress_flags = S_NO_AUTO_2; // no automatic secondary stress
            tr.langopts.numbers =
                NUM_SINGLE_STRESS | NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED | NUM_OMIT_1_THOUSAND | NUM_DFRACTION_4;
        }
        b"uk" => {
            // Ukrainian
            translator_russian(&mut tr);
        }
        b"ur" | b"sd" => {
            // Urdu, Sindhi
            tr.letter_bits_offset = OFFSET_ARABIC;
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
            tr.langopts.numbers = NUM_SWAP_TENS;
            tr.langopts.break_numbers = BREAK_LAKH_UR;
        }
        b"vi" => {
            // Vietnamese
            #[rustfmt::skip]
            static VOWELS_VI: [u32; 73] = [
                 0x61,   0xe0,   0xe1, 0x1ea3,   0xe3, 0x1ea1, // a
                0x103, 0x1eb1, 0x1eaf, 0x1eb3, 0x1eb5, 0x1eb7, // ă
                 0xe2, 0x1ea7, 0x1ea5, 0x1ea9, 0x1eab, 0x1ead, // â
                 0x65,   0xe8,   0xe9, 0x1ebb, 0x1ebd, 0x1eb9, // e
                 0xea, 0x1ec1, 0x1ebf, 0x1ec3, 0x1ec5, 0x1ec7, // i
                 0x69,   0xec,   0xed, 0x1ec9,  0x129, 0x1ecb, // i
                 0x6f,   0xf2,   0xf3, 0x1ecf,   0xf5, 0x1ecd, // o
                 0xf4, 0x1ed3, 0x1ed1, 0x1ed5, 0x1ed7, 0x1ed9, // ô
                0x1a1, 0x1edd, 0x1edb, 0x1edf, 0x1ee1, 0x1ee3, // ơ
                 0x75,   0xf9,   0xfa, 0x1ee7,  0x169, 0x1ee5, // u
                0x1b0, 0x1eeb, 0x1ee9, 0x1eed, 0x1eef, 0x1ef1, // ư
                 0x79, 0x1ef3,   0xfd, 0x1ef7, 0x1ef9, 0x1ef5, // y
                0,
            ];

            tr.langopts.stress_rule = STRESSPOSN_1L;
            tr.langopts.word_gap = 0x21; // length of a final vowel is less dependent on the next consonant, don't merge consonant with next word
            tr.letter_groups[0] = Some(&VOWELS_VI);
            tr.letter_groups[7] = Some(&VOWELS_VI);
            tr.langopts.tone_language = 1; // Tone language, use  CalcPitches_Tone() rather than CalcPitches()
            tr.langopts.unstressed_wd1 = 2;
            tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_HUNDRED_AND_DIGIT | NUM_DFRACTION_4 | NUM_ZERO_HUNDRED;
        }
        b"xex" => {
            // Xextan
            // no terminator, as in C
            static XEX_PUNCT_WITHIN_WORD: [u32; 1] = [b'\'' as u32];
            tr.langopts.numbers = 0;
            tr.langopts.lowercase_sentence = true;
            tr.punct_within_word = &XEX_PUNCT_WITHIN_WORD;
        }
        b"shn" => {
            tr.langopts.tone_language = 1; // Tone language, use  CalcPitches_Tone() rather than CalcPitches()
            tr.langopts.numbers = NUM_DEFAULT;
            tr.langopts.break_numbers = BREAK_INDIVIDUAL;
        }
        b"cmn" | b"yue" | b"zh" => {
            // zh is used for backwards compatibility. Prefer cmn or yue.
            tr.langopts.stress_rule = STRESSPOSN_1R; // stress on final syllable of a "word"
            tr.langopts.stress_flags = S_NO_DIM; // don't automatically set diminished stress (may be set in the intonation module)
            tr.langopts.vowel_pause = 0;
            tr.langopts.tone_language = 1; // Tone language, use  CalcPitches_Tone() rather than CalcPitches()
            tr.langopts.tone_numbers = 1; // a number after letters indicates a tone number (eg. pinyin or jyutping)
            tr.langopts.ideographs = 1;
            tr.langopts.our_alphabet = 0x3100;
            tr.langopts.word_gap = 0x21; // length of a final vowel is less dependent on the next consonant, don't merge consonant with next word
            tr.langopts.textmode = true;
            tr.langopts.listx = 1; // compile *_listx after *_list
            if name2 == l3(b'y', b'u', b'e') {
                tr.langopts.numbers = NUM_DEFAULT;
                tr.langopts.numbers2 = NUM2_ZERO_TENS;
                tr.langopts.break_numbers = BREAK_INDIVIDUAL;
            }
        }
        _ => {
            tr.langopts.param[LOPT_UNPRONOUNCABLE] = 1; // disable check for unpronouncable words
        }
    }

    tr.translator_name = name2;

    process_language_options(&mut tr.langopts);
    tr
}

pub fn process_language_options(langopts: &mut LanguageOptions) {
    if langopts.numbers & NUM_DECIMAL_COMMA != 0 {
        // use . and ; for thousands and decimal separators
        langopts.thousands_sep = b'.' as i32;
        langopts.decimal_sep = b',' as i32;
    }
    if langopts.numbers & NUM_THOUS_SPACE != 0 {
        langopts.thousands_sep = 0; // don't allow thousands separator, except space
    }
}

fn translator_russian(tr: &mut Translator) {
    static RU_IVOWELS: [u8; 5] = [0x15, 0x18, 0x34, 0x37, 0]; // add "е и є ї" to Y lettergroup (iotated vowels & soft-sign)

    set_cyrillic_letters(tr);
    set_letter_bits(tr, LETTERGP_Y, &RU_IVOWELS);

    tr.langopts.param[LOPT_UNPRONOUNCABLE] = 0x432; // [v]  don't count this character at start of word
    tr.langopts.param[LOPT_REGRESSIVE_VOICING] = 0x03;
    tr.langopts.param[LOPT_REDUCE] = 2;
    tr.langopts.stress_rule = STRESSPOSN_SYLCOUNT;
    tr.langopts.stress_flags = S_NO_AUTO_2;

    tr.langopts.numbers = NUM_DECIMAL_COMMA | NUM_OMIT_1_HUNDRED;
    tr.langopts.numbers2 = NUM2_THOUSANDPLEX_VAR_THOUSANDS | NUM2_THOUSANDS_VAR1; // variant numbers before thousands
    tr.langopts.max_digits = 32;
    tr.langopts.max_initial_consonants = 5;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english() {
        let tr = select_translator(b"en");
        assert_eq!(tr.langopts.stress_rule, STRESSPOSN_1L);
        assert_eq!(tr.langopts.numbers, NUM_HUNDRED_AND | NUM_ROMAN | NUM_1900);
        assert_ne!(tr.letter_bits[b'y' as usize] & (1 << 6), 0);
        assert_ne!(tr.letter_bits[b'a' as usize] & (1 << 6), 0);
        assert_eq!(tr.translator_name, l2(b'e', b'n'));
        assert_eq!(tr.dictionary_name, b"en");
        assert_eq!(tr.langopts.thousands_sep, b',' as i32);
    }

    #[test]
    fn hindi() {
        let tr = select_translator(b"hi");
        assert_eq!(tr.letter_bits_offset, 0x900);
        assert_eq!(tr.langopts.break_numbers, BREAK_LAKH_HI);
        assert_eq!(tr.langopts.suffix_add_e, 0x94d);
        assert_eq!(tr.langopts.param[LOPT_UNPRONOUNCABLE], 1);
    }

    #[test]
    fn quirks() {
        // wchar_t vowels_be read as bytes: only the first vowel is set.
        let be = select_translator(b"be");
        assert_eq!(be.letter_bits[0x10], 0x81);
        assert_eq!(be.letter_bits[0x15], 0);

        assert_eq!(select_translator(b"sr").dictionary_name, b"hbs");
        let de = select_translator(b"de");
        assert_eq!((de.langopts.thousands_sep, de.langopts.decimal_sep), (b'.' as i32, b',' as i32));
        assert_eq!(select_translator(b"hr").langopts.thousands_sep, 0);

        let ar = select_translator(b"ar");
        assert_eq!(ar.letter_bits[0x51], (1 << LETTERGP_G) | (1 << LETTERGP_H) | (1 << LETTERGP_Y));
        assert_eq!(ar.letter_bits[0x4e], 1 << LETTERGP_A);

        // name2 keeps only the last 4 bytes.
        assert_eq!(select_translator(b"xpiqd").langopts.numbers, 0);
        // unknown, empty and non-ASCII names hit the default case
        for name in [&b""[..], b"zz", b"\xff\xfe", b"en\0x"] {
            let tr = select_translator(name);
            if name == b"en\0x" {
                assert_eq!(tr.langopts.stress_rule, STRESSPOSN_1L);
            } else {
                assert_eq!(tr.langopts.param[LOPT_UNPRONOUNCABLE], 1);
            }
        }
        assert_eq!(select_translator(b"ru").frequent_pairs.unwrap().last(), Some(&0x7fff));
    }

    #[test]
    fn alphabets() {
        assert_eq!(alphabet_from_char(0x3b1).unwrap().name, b"_el");
        assert_eq!(alphabet_from_char(0x10460).unwrap().name, b"_shaw");
        assert!(alphabet_from_char(0x800).is_none());
        assert!(alphabet_from_char(b'a' as i32).is_none());
        assert!(alphabet_from_char(-1).is_none());
        let languages = alphabet_languages();
        assert!(languages.contains(&('\u{10a0}', '\u{10ff}', "ka".into())));
        assert!(languages.contains(&('\u{a00}', '\u{a7f}', "pa".into())));
        assert!(!languages.iter().any(|(lo, hi, _)| (*lo..=*hi).contains(&'ส')));
    }
}
