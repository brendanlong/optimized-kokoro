//! Constants and structures from translate.h, phoneme.h and synthesize.h.

#![allow(dead_code)]

use crate::data::{Dict, PhonemeTab};
use std::sync::Arc;

pub const REPLACED_E: u8 = b'E';

pub const N_WORD_PHONEMES: usize = 200;
pub const N_WORD_BYTES: usize = 160;
pub const N_PHONEME_BYTES: usize = 160;
pub const N_CLAUSE_WORDS: usize = 300;
pub const N_TR_SOURCE: usize = 800;
/// The size of phoneme-string buffers that C fills with unbounded `strcpy`/`sprintf`. C's own
/// buffers are smaller and overflow; making ours large keeps the whole string, as C usually ends
/// up reading it. Where C bounds a copy, the port keeps C's bound.
pub const PH_BUF: usize = 256;

pub const N_RULE_GROUP2: usize = 120;
pub const N_HASH_DICT: usize = 1024;
pub const N_LETTER_GROUPS: usize = 95;

pub const FLAG_SKIPWORDS: u32 = 0x80;
pub const FLAG_PREPAUSE: u32 = 0x100;
pub const FLAG_STRESS_END: u32 = 0x200;
pub const FLAG_STRESS_END2: u32 = 0x400;
pub const FLAG_UNSTRESS_END: u32 = 0x800;
pub const FLAG_SPELLWORD: u32 = 0x1000;
pub const FLAG_ACCENT_BEFORE: u32 = 0x1000;
pub const FLAG_ABBREV: u32 = 0x2000;
pub const FLAG_DOUBLING: u32 = 0x4000;
pub const BITNUM_FLAG_ALT: u32 = 14;
pub const FLAG_ALT_TRANS: u32 = 0x8000;
pub const FLAG_ALT2_TRANS: u32 = 0x10000;
pub const FLAG_ALT3_TRANS: u32 = 0x20000;
pub const FLAG_ALT7_TRANS: u32 = 0x200000;
pub const FLAG_COMBINE: u32 = 0x800000;
pub const FLAG_ALLOW_DOT: u32 = 0x01000000;
pub const FLAG_NEEDS_DOT: u32 = 0x02000000;
pub const FLAG_WAS_UNPRONOUNCABLE: u32 = 0x04000000;
pub const FLAG_MAX3: u32 = 0x08000000;
pub const FLAG_PAUSE1: u32 = 0x10000000;
pub const FLAG_TEXTMODE: u32 = 0x20000000;
pub const FLAG_FOUND_ATTRIBUTES: u32 = 0x40000000;
pub const FLAG_FOUND: u32 = 0x80000000;

pub const FLAG_VERBF: u32 = 0x1;
pub const FLAG_VERBSF: u32 = 0x2;
pub const FLAG_NOUNF: u32 = 0x4;
pub const FLAG_PASTF: u32 = 0x8;
pub const FLAG_VERB: u32 = 0x10;
pub const FLAG_NOUN: u32 = 0x20;
pub const FLAG_PAST: u32 = 0x40;
pub const FLAG_VERB_EXT: u32 = 0x100;
pub const FLAG_CAPITAL: u32 = 0x200;
pub const FLAG_ALLCAPS: u32 = 0x400;
pub const FLAG_ACCENT: u32 = 0x800;
pub const FLAG_SENTENCE: u32 = 0x2000;
pub const FLAG_ONLY: u32 = 0x4000;
pub const FLAG_ONLY_S: u32 = 0x8000;
pub const FLAG_STEM: u32 = 0x10000;
pub const FLAG_ATEND: u32 = 0x20000;
pub const FLAG_ATSTART: u32 = 0x40000;
pub const FLAG_NATIVE: u32 = 0x80000;
pub const FLAG_LOOKUP_SYMBOL: u32 = 0x40000000;

pub const FLAG_ALL_UPPER: u32 = 0x1;
pub const FLAG_FIRST_UPPER: u32 = 0x2;
pub const FLAG_UPPERS: u32 = 0x3;
pub const FLAG_HAS_PLURAL: u32 = 0x4;
pub const FLAG_PHONEMES: u32 = 0x8;
pub const FLAG_LAST_WORD: u32 = 0x10;
pub const FLAG_EMBEDDED: u32 = 0x40;
pub const FLAG_HYPHEN: u32 = 0x80;
pub const FLAG_NOSPACE: u32 = 0x100;
pub const FLAG_FIRST_WORD: u32 = 0x200;
pub const FLAG_FOCUS: u32 = 0x400;
pub const FLAG_EMPHASIZED: u32 = 0x800;
pub const FLAG_EMPHASIZED2: u32 = 0xc00;
pub const FLAG_DONT_SWITCH_TRANSLATOR: u32 = 0x1000;
pub const FLAG_SUFFIX_REMOVED: u32 = 0x2000;
pub const FLAG_HYPHEN_AFTER: u32 = 0x4000;
pub const FLAG_ORDINAL: u32 = 0x8000;
pub const FLAG_HAS_DOT: u32 = 0x10000;
pub const FLAG_COMMA_AFTER: u32 = 0x20000;
pub const FLAG_MULTIPLE_SPACES: u32 = 0x40000;
pub const FLAG_INDIVIDUAL_DIGITS: u32 = 0x80000;
pub const FLAG_DELETE_WORD: u32 = 0x100000;
pub const FLAG_CHAR_REPLACED: u32 = 0x200000;
pub const FLAG_TRANSLATOR2: u32 = 0x400000;
pub const FLAG_PREFIX_REMOVED: u32 = 0x800000;

pub const FLAG_SUFFIX_VOWEL: u32 = 0x08000000;
pub const FLAG_NO_TRACE: u32 = 0x10000000;
pub const FLAG_NO_PREFIX: u32 = 0x20000000;
pub const FLAG_UNPRON_TEST: u32 = 0x80000000;

pub const SUFX_E: u32 = 0x0100;
pub const SUFX_I: u32 = 0x0200;
pub const SUFX_P: u32 = 0x0400;
pub const SUFX_V: u32 = 0x0800;
pub const SUFX_D: u32 = 0x1000;
pub const SUFX_F: u32 = 0x2000;
pub const SUFX_Q: u32 = 0x4000;
pub const SUFX_T: u32 = 0x10000;
pub const SUFX_B: u32 = 0x20000;
pub const SUFX_A: u32 = 0x40000;
pub const SUFX_M: u32 = 0x80000;
pub const SUFX_UNPRON: u32 = 0x8000;

pub const FLAG_ALLOW_TEXTMODE: u32 = 0x02;
pub const FLAG_SUFX: u32 = 0x04;
pub const FLAG_SUFX_S: u32 = 0x08;
pub const FLAG_SUFX_E_ADDED: u32 = 0x10;

pub const RULE_PRE: u8 = 1;
pub const RULE_POST: u8 = 2;
pub const RULE_PHONEMES: u8 = 3;
pub const RULE_PH_COMMON: u8 = 4;
pub const RULE_CONDITION: u8 = 5;
pub const RULE_GROUP_START: u8 = 6;
pub const RULE_GROUP_END: u8 = 7;
pub const RULE_PRE_ATSTART: u8 = 8;
pub const RULE_LINENUM: u8 = 9;
pub const RULE_STRESSED: u8 = 10;
pub const RULE_DOUBLE: u8 = 11;
pub const RULE_INC_SCORE: u8 = 12;
pub const RULE_DEL_FWD: u8 = 13;
pub const RULE_ENDING: u8 = 14;
pub const RULE_DIGIT: u8 = 15;
pub const RULE_NONALPHA: u8 = 16;
pub const RULE_LETTERGP: u8 = 17;
pub const RULE_LETTERGP2: u8 = 18;
pub const RULE_CAPITAL: u8 = 19;
pub const RULE_REPLACEMENTS: u8 = 20;
pub const RULE_SYLLABLE: u8 = 21;
pub const RULE_SKIPCHARS: u8 = 23;
pub const RULE_NO_SUFFIX: u8 = 24;
pub const RULE_NOTVOWEL: u8 = 25;
pub const RULE_IFVERB: u8 = 26;
pub const RULE_DOLLAR: u8 = 28;
pub const RULE_NOVOWELS: u8 = 29;
pub const RULE_SPELLING: u8 = 31;
pub const RULE_LAST_RULE: u8 = 31;
pub const RULE_SPACE: u8 = 32;
pub const RULE_DEC_SCORE: u8 = 60;

pub const DOLLAR_UNPR: u8 = 0x01;
pub const DOLLAR_NOPREFIX: u8 = 0x02;
pub const DOLLAR_LIST: u8 = 0x03;

pub const LETTERGP_A: i32 = 0;
pub const LETTERGP_B: i32 = 1;
pub const LETTERGP_C: i32 = 2;
pub const LETTERGP_H: i32 = 3;
pub const LETTERGP_F: i32 = 4;
pub const LETTERGP_G: i32 = 5;
pub const LETTERGP_Y: i32 = 6;
pub const LETTERGP_VOWEL2: i32 = 7;

pub const CLAUSE_PAUSE: i32 = 0x00000FFF;
pub const CLAUSE_INTONATION_TYPE: i32 = 0x00007000;
pub const CLAUSE_OPTIONAL_SPACE_AFTER: i32 = 0x00008000;
pub const CLAUSE_TYPE: i32 = 0x000F0000;
pub const CLAUSE_PUNCTUATION_IN_WORD: i32 = 0x00100000;
pub const CLAUSE_SPEAK_PUNCTUATION_NAME: i32 = 0x00200000;
pub const CLAUSE_DOT_AFTER_LAST_WORD: i32 = 0x00400000;
pub const CLAUSE_PAUSE_LONG: i32 = 0x00800000;

pub const CLAUSE_INTONATION_FULL_STOP: i32 = 0x00000000;
pub const CLAUSE_INTONATION_COMMA: i32 = 0x00001000;
pub const CLAUSE_INTONATION_QUESTION: i32 = 0x00002000;
pub const CLAUSE_INTONATION_EXCLAMATION: i32 = 0x00003000;
pub const CLAUSE_INTONATION_NONE: i32 = 0x00004000;

pub const CLAUSE_TYPE_NONE: i32 = 0x00000000;
pub const CLAUSE_TYPE_EOF: i32 = 0x00010000;
pub const CLAUSE_TYPE_VOICE_CHANGE: i32 = 0x00020000;
pub const CLAUSE_TYPE_CLAUSE: i32 = 0x00040000;
pub const CLAUSE_TYPE_SENTENCE: i32 = 0x00080000;

pub const CLAUSE_NONE: i32 = CLAUSE_INTONATION_NONE | CLAUSE_TYPE_NONE;
pub const CLAUSE_PARAGRAPH: i32 = 70 | CLAUSE_INTONATION_FULL_STOP | CLAUSE_TYPE_SENTENCE;
pub const CLAUSE_EOF: i32 = 40 | CLAUSE_INTONATION_FULL_STOP | CLAUSE_TYPE_SENTENCE | CLAUSE_TYPE_EOF;
pub const CLAUSE_VOICE: i32 = CLAUSE_INTONATION_NONE | CLAUSE_TYPE_VOICE_CHANGE;
pub const CLAUSE_PERIOD: i32 = 40 | CLAUSE_INTONATION_FULL_STOP | CLAUSE_TYPE_SENTENCE;
pub const CLAUSE_COMMA: i32 = 20 | CLAUSE_INTONATION_COMMA | CLAUSE_TYPE_CLAUSE;
pub const CLAUSE_SHORTCOMMA: i32 = 4 | CLAUSE_INTONATION_COMMA | CLAUSE_TYPE_CLAUSE;
pub const CLAUSE_SHORTFALL: i32 = 4 | CLAUSE_INTONATION_FULL_STOP | CLAUSE_TYPE_CLAUSE;
pub const CLAUSE_QUESTION: i32 = 40 | CLAUSE_INTONATION_QUESTION | CLAUSE_TYPE_SENTENCE;
pub const CLAUSE_EXCLAMATION: i32 = 45 | CLAUSE_INTONATION_EXCLAMATION | CLAUSE_TYPE_SENTENCE;
pub const CLAUSE_COLON: i32 = 30 | CLAUSE_INTONATION_FULL_STOP | CLAUSE_TYPE_CLAUSE;
pub const CLAUSE_SEMICOLON: i32 = 30 | CLAUSE_INTONATION_COMMA | CLAUSE_TYPE_CLAUSE;

pub const SAYAS_CHARS: i32 = 0x12;
pub const SAYAS_GLYPHS: i32 = 0x13;
pub const SAYAS_SINGLE_CHARS: i32 = 0x14;
pub const SAYAS_KEY: i32 = 0x24;
pub const SAYAS_DIGITS: i32 = 0x40;
pub const SAYAS_DIGITS1: i32 = 0xc1;

pub const CHAR_EMPHASIS: i32 = 0x0530;
pub const CHAR_COMMA_BREAK: i32 = 0x0557;
pub const CTRL_EMBEDDED: i32 = 0x01;

pub const AL_DONT_NAME: i32 = 0x01;
pub const AL_NOT_LETTERS: i32 = 0x02;
pub const AL_WORDS: i32 = 0x04;
pub const AL_NOT_CODE: i32 = 0x08;
pub const AL_NO_SYMBOL: i32 = 0x10;

pub const N_LOPTS: usize = 18;
pub const LOPT_DIERESES: usize = 0;
pub const LOPT_IT_LENGTHEN: usize = 1;
pub const LOPT_PREFIXES: usize = 2;
pub const LOPT_REGRESSIVE_VOICING: usize = 3;
pub const LOPT_UNPRONOUNCABLE: usize = 4;
pub const LOPT_SONORANT_MIN: usize = 5;
pub const LOPT_WORD_MERGE: usize = 6;
pub const LOPT_MAXAMP_EOC: usize = 7;
pub const LOPT_REDUCE: usize = 8;
pub const LOPT_COMBINE_WORDS: usize = 9;
pub const LOPT_CAPS_IN_WORD: usize = 10;
pub const LOPT_ALT: usize = 11;
pub const LOPT_BRACKET_PAUSE: usize = 12;
pub const LOPT_ANNOUNCE_PUNCT: usize = 13;
pub const LOPT_LONG_VOWEL_THRESHOLD: usize = 14;
pub const LOPT_SUFFIX: usize = 15;
pub const LOPT_APOSTROPHE: usize = 16;
pub const LOPT_BRACKET_PAUSE_ANNOUNCED: usize = 17;

pub const STRESSPOSN_1L: i32 = 0;
pub const STRESSPOSN_2L: i32 = 1;
pub const STRESSPOSN_2R: i32 = 2;
pub const STRESSPOSN_1R: i32 = 3;
pub const STRESSPOSN_3R: i32 = 4;
pub const STRESSPOSN_SYLCOUNT: i32 = 5;
pub const STRESSPOSN_1RH: i32 = 6;
pub const STRESSPOSN_1RU: i32 = 7;
pub const STRESSPOSN_2LLH: i32 = 8;
pub const STRESSPOSN_ALL: i32 = 9;
pub const STRESSPOSN_GREENLANDIC: i32 = 12;
pub const STRESSPOSN_1SL: i32 = 13;
pub const STRESSPOSN_EU: i32 = 15;

pub const S_NO_DIM: i32 = 0x02;
pub const S_FINAL_DIM: i32 = 0x04;
pub const S_FINAL_DIM_ONLY: i32 = 0x06;
pub const S_FINAL_NO_2: i32 = 0x10;
pub const S_NO_AUTO_2: i32 = 0x20;
pub const S_2_TO_HEAVY: i32 = 0x40;
pub const S_FIRST_PRIMARY: i32 = 0x80;
pub const S_FINAL_VOWEL_UNSTRESSED: i32 = 0x100;
pub const S_FINAL_SPANISH: i32 = 0x200;
pub const S_2_SYL_2: i32 = 0x1000;
pub const S_INITIAL_2: i32 = 0x2000;
pub const S_MID_DIM: i32 = 0x10000;
pub const S_PRIORITY_STRESS: i32 = 0x20000;
pub const S_EO_CLAUSE1: i32 = 0x40000;
pub const S_FINAL_LONG: i32 = 0x80000;
pub const S_HYPEN_UNSTRESS: i32 = 0x100000;
pub const S_NO_EOC_LENGTHEN: i32 = 0x200000;

pub const NUM_DEFAULT: i32 = 0x00000001;
pub const NUM_THOUS_SPACE: i32 = 0x00000004;
pub const NUM_DECIMAL_COMMA: i32 = 0x00000008;
pub const NUM_SWAP_TENS: i32 = 0x00000010;
pub const NUM_AND_UNITS: i32 = 0x00000020;
pub const NUM_HUNDRED_AND: i32 = 0x00000040;
pub const NUM_SINGLE_AND: i32 = 0x00000080;
pub const NUM_SINGLE_STRESS: i32 = 0x00000100;
pub const NUM_SINGLE_VOWEL: i32 = 0x00000200;
pub const NUM_OMIT_1_HUNDRED: i32 = 0x00000400;
pub const NUM_1900: i32 = 0x00000800;
pub const NUM_ALLOW_SPACE: i32 = 0x00001000;
pub const NUM_DFRACTION_BITS: i32 = 0x0000e000;
pub const NUM_ORDINAL_DOT: i32 = 0x00010000;
pub const NUM_NOPAUSE: i32 = 0x00020000;
pub const NUM_AND_HUNDRED: i32 = 0x00040000;
pub const NUM_THOUSAND_AND: i32 = 0x00080000;
pub const NUM_VIGESIMAL: i32 = 0x00100000;
pub const NUM_OMIT_1_THOUSAND: i32 = 0x00200000;
pub const NUM_ZERO_HUNDRED: i32 = 0x00400000;
pub const NUM_HUNDRED_AND_DIGIT: i32 = 0x00800000;
pub const NUM_ROMAN: i32 = 0x01000000;
pub const NUM_ROMAN_CAPITALS: i32 = 0x02000000;
pub const NUM_ROMAN_AFTER: i32 = 0x04000000;
pub const NUM_ROMAN_ORDINAL: i32 = 0x08000000;
pub const NUM_SINGLE_STRESS_L: i32 = 0x10000000;

pub const NUM_DFRACTION_1: i32 = 0x00002000;
pub const NUM_DFRACTION_2: i32 = 0x00004000;
pub const NUM_DFRACTION_3: i32 = 0x00006000;
pub const NUM_DFRACTION_4: i32 = 0x00008000;
pub const NUM_DFRACTION_5: i32 = 0x0000a000;
pub const NUM_DFRACTION_6: i32 = 0x0000c000;
pub const NUM_DFRACTION_7: i32 = 0x0000e000;

pub const NUM2_THOUSANDS_VAR_BITS: i32 = 0x000001c0;
pub const NUM2_SWAP_THOUSANDS: i32 = 0x00000200;
pub const NUM2_ORDINAL_NO_AND: i32 = 0x00000800;
pub const NUM2_MULTIPLE_ORDINAL: i32 = 0x00001000;
pub const NUM2_NO_TEEN_ORDINALS: i32 = 0x00002000;
pub const NUM2_MYRIADS: i32 = 0x00004000;
pub const NUM2_ENGLISH_NUMERALS: i32 = 0x00008000;
pub const NUM2_PERCENT_BEFORE: i32 = 0x00010000;
pub const NUM2_OMIT_1_HUNDRED_ONLY: i32 = 0x00020000;
pub const NUM2_ORDINAL_AND_THOUSANDS: i32 = 0x00040000;
pub const NUM2_ORDINAL_DROP_VOWEL: i32 = 0x00080000;
pub const NUM2_ZERO_TENS: i32 = 0x00100000;

pub const NUM2_THOUSANDPLEX_VAR_THOUSANDS: i32 = 0x00000002;
pub const NUM2_THOUSANDPLEX_VAR_MILLIARDS: i32 = 0x00000008;
pub const NUM2_THOUSANDPLEX_VAR_ALL: i32 = 0x0000001e;

pub const NUM2_THOUSANDS_VAR1: i32 = 0x00000040;
pub const NUM2_THOUSANDS_VAR2: i32 = 0x00000080;
pub const NUM2_THOUSANDS_VAR3: i32 = 0x000000c0;
pub const NUM2_THOUSANDS_VAR4: i32 = 0x00000100;
pub const NUM2_THOUSANDS_VAR5: i32 = 0x00000140;

pub const BREAK_THOUSANDS: u32 = 0x49249248;
pub const BREAK_MYRIADS: u32 = 0x11111110;
pub const BREAK_LAKH: u32 = 0xaaaaaaa8;
pub const BREAK_LAKH_BN: u32 = 0x24924aa8;
pub const BREAK_LAKH_DV: u32 = 0x000014a8;
pub const BREAK_LAKH_HI: u32 = 0x00014aa8;
pub const BREAK_LAKH_UR: u32 = 0x000052a8;
pub const BREAK_INDIVIDUAL: u32 = 0x00000018;

pub const OPTION_EMPHASIZE_ALLCAPS: i32 = 0x100;
pub const OPTION_EMPHASIZE_PENULTIMATE: i32 = 0x200;

// phoneme.h
pub const PH_PAUSE: u8 = 0;
pub const PH_STRESS: u8 = 1;
pub const PH_VOWEL: u8 = 2;
pub const PH_LIQUID: u8 = 3;
pub const PH_STOP: u8 = 4;
pub const PH_VSTOP: u8 = 5;
pub const PH_FRICATIVE: u8 = 6;
pub const PH_VFRICATIVE: u8 = 7;
pub const PH_NASAL: u8 = 8;
pub const PH_VIRTUAL: u8 = 9;
pub const PH_DELETED: u8 = 14;
pub const PH_INVALID: u8 = 15;

pub const PH_UNSTRESSED: u32 = 1 << 1;
pub const PH_VOICELESS: u32 = 1 << 3;
pub const PH_VOICED: u32 = 1 << 4;
pub const PH_SIBILANT: u32 = 1 << 5;
pub const PH_NOLINK: u32 = 1 << 6;
pub const PH_TRILL: u32 = 1 << 7;
pub const PH_PALATAL: u32 = 1 << 9;
pub const PH_BRKAFTER: u32 = 1 << 14;
pub const PH_NONSYLLABIC: u32 = 1 << 20;
pub const PH_LONG: u32 = 1 << 21;
pub const PH_LENGTHENSTOP: u32 = 1 << 22;
pub const PH_RHOTIC: u32 = 1 << 23;
pub const PH_NOPAUSE: u32 = 1 << 24;
pub const PH_PREVOICE: u32 = 1 << 25;

pub const PHON_CONTROL: u8 = 1;
pub const PHON_STRESS_U: u8 = 2;
pub const PHON_STRESS_D: u8 = 3;
pub const PHON_STRESS_2: u8 = 4;
pub const PHON_STRESS_3: u8 = 5;
pub const PHON_STRESS_P: u8 = 6;
pub const PHON_STRESS_P2: u8 = 7;
pub const PHON_STRESS_PREV: u8 = 8;
pub const PHON_PAUSE: u8 = 9;
pub const PHON_PAUSE_SHORT: u8 = 10;
pub const PHON_PAUSE_NOLINK: u8 = 11;
pub const PHON_LENGTHEN: u8 = 12;
pub const PHON_SCHWA: u8 = 13;
pub const PHON_SCHWA_SHORT: u8 = 14;
pub const PHON_END_WORD: u8 = 15;
pub const PHON_DEFAULTTONE: u8 = 17;
pub const PHON_CAPITAL: u8 = 18;
pub const PHON_GLOTTALSTOP: u8 = 19;
pub const PHON_SYLLABIC: u8 = 20;
pub const PHON_SWITCH: u8 = 21;
pub const PHON_X1: u8 = 22;
pub const PHON_PAUSE_VSHORT: u8 = 23;
pub const PHON_PAUSE_LONG: u8 = 24;
pub const PHON_T_REDUCED: u8 = 25;
pub const PHON_STRESS_TONIC: u8 = 26;
pub const PHON_PAUSE_CLAUSE: u8 = 27;
pub const PHON_VOWELTYPES: u8 = 28;

pub const N_PHONEME_TAB: usize = 256;
pub const N_REPLACE_PHONEMES: usize = 60;

// synthesize.h
pub const N_PHONEME_LIST: usize = 1000;

pub const SFLAG_SEQCONTINUE: u16 = 0x01;
pub const SFLAG_EMBEDDED: u16 = 0x02;
pub const SFLAG_SYLLABLE: u16 = 0x04;
pub const SFLAG_LENGTHEN: u16 = 0x08;
pub const SFLAG_DICTIONARY: u16 = 0x10;
pub const SFLAG_SWITCHED_LANG: u16 = 0x20;
pub const SFLAG_PROMOTE_STRESS: u16 = 0x40;
pub const SFLAG_NEXT_PAUSE: u16 = 0x2000;

pub const EMBED_P: i32 = 1;
pub const EMBED_S: i32 = 2;
pub const EMBED_A: i32 = 3;
pub const EMBED_R: i32 = 4;
pub const EMBED_H: i32 = 5;
pub const EMBED_T: i32 = 6;
pub const EMBED_I: i32 = 7;
pub const EMBED_S2: i32 = 8;
pub const EMBED_Y: i32 = 9;
pub const EMBED_M: i32 = 10;
pub const EMBED_U: i32 = 11;
pub const EMBED_B: i32 = 12;
pub const EMBED_F: i32 = 13;
pub const N_EMBEDDED_VALUES: usize = 15;
pub const ESPEAK_RATE_NORMAL: i32 = 175;
pub const EMBEDDED_DEFAULT: [i32; N_EMBEDDED_VALUES] =
    [0, 50, ESPEAK_RATE_NORMAL, 100, 50, 0, 0, 0, ESPEAK_RATE_NORMAL, 0, 0, 0, 0, 0, 0];

pub const PHLIST_START_OF_WORD: u8 = 1;
pub const PHLIST_END_OF_CLAUSE: u8 = 2;
pub const PHLIST_START_OF_SENTENCE: u8 = 4;
pub const PHLIST_START_OF_CLAUSE: u8 = 8;

pub const N_PHONEME_DATA_PARAM: usize = 16;
pub const PD_FMT: usize = 0;
pub const PD_WAV: usize = 1;
pub const PD_VWLSTART: usize = 2;
pub const PD_VWLEND: usize = 3;
pub const PD_ADDWAV: usize = 4;
pub const PD_FORNEXTPH: i32 = 0x2;

pub const INSTN_RETURN: u16 = 0x0001;
pub const INSTN_CONTINUE: u16 = 0x0002;

pub const I_CHANGE_PHONEME: usize = 0x01;
pub const I_REPLACE_NEXT_PHONEME: usize = 0x02;
pub const I_INSERT_PHONEME: usize = 0x03;
pub const I_APPEND_PHONEME: usize = 0x04;
pub const I_APPEND_IFNEXTVOWEL: usize = 0x05;
pub const I_VOICING_SWITCH: usize = 0x06;
pub const I_PAUSE_BEFORE: usize = 0x07;
pub const I_PAUSE_AFTER: usize = 0x08;
pub const I_LENGTH_MOD: usize = 0x09;
pub const I_SET_LENGTH: usize = 0x0a;
pub const I_LONG_LENGTH: usize = 0x0b;
pub const I_ADD_LENGTH: usize = 0x0c;
pub const I_IPA_NAME: usize = 0x0d;

pub const I_NOT: u16 = 0x0003;
pub const I_JUMP_FALSE: u16 = 0x6800;

pub const CONDITION_IS_PHONEME_TYPE: u32 = 0x00;
pub const CONDITION_IS_PLACE_OF_ARTICULATION: u32 = 0x20;
pub const CONDITION_IS_PHFLAG_SET: u32 = 0x40;
pub const CONDITION_IS_OTHER: u32 = 0x80;

pub const STRESS_IS_DIMINISHED: i32 = 0;
pub const STRESS_IS_UNSTRESSED: i32 = 1;
pub const STRESS_IS_NOT_STRESSED: i32 = 2;
pub const STRESS_IS_SECONDARY: i32 = 3;
pub const STRESS_IS_PRIMARY: i32 = 4;
pub const STRESS_IS_PRIORITY: i32 = 5;

pub const IS_AFTER_STRESS: u32 = 9;
pub const IS_NOT_VOWEL: u32 = 10;
pub const IS_FINAL_VOWEL: u32 = 11;
pub const IS_VOICED: u32 = 12;
pub const IS_FIRST_VOWEL: u32 = 13;
pub const IS_SECOND_VOWEL: u32 = 14;
pub const IS_TRANSLATION_GIVEN: u32 = 16;
pub const IS_BREAK: u32 = 17;
pub const IS_WORD_START: u32 = 18;
pub const IS_WORD_END: u32 = 19;

pub const ESPEAK_PHONEMES_IPA: i32 = 0x02;
pub const ESPEAK_PHONEMES_TIE: i32 = 0x80;

pub const ESPEAKNG_DEFAULT_VOICE: &[u8] = b"en";

/// `WORD_TAB`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WordTab {
    pub flags: u32,
    pub start: u16,
    pub pre_pause: u8,
    pub sourceix: u16,
    pub length: u8,
}

/// `ALPHABET`.
#[derive(Debug, Clone, Copy)]
pub struct Alphabet {
    pub name: &'static [u8],
    pub offset: i32,
    pub range_min: u32,
    pub range_max: u32,
    pub language: i32,
    pub flags: i32,
}

/// `LANGUAGE_OPTIONS`, without the fields only synthesis reads (lengths, amplitudes, tunes).
#[derive(Debug, Clone)]
pub struct LanguageOptions {
    pub word_gap: i32,
    pub vowel_pause: i32,
    pub stress_rule: i32,
    pub stress_flags: i32,
    pub unstressed_wd1: i32,
    pub unstressed_wd2: i32,
    pub param: [i32; N_LOPTS],
    pub numbers: i32,
    pub numbers2: i32,
    pub break_numbers: u32,
    pub max_roman: i32,
    pub min_roman: i32,
    pub thousands_sep: i32,
    pub decimal_sep: i32,
    pub max_digits: i32,
    pub ordinal_indicator: Option<&'static [u8]>,
    pub roman_suffix: &'static [u8],
    pub accents: i32,
    pub tone_language: i32,
    pub intonation_group: i32,
    pub long_stop: i32,
    pub max_initial_consonants: i32,
    pub spelling_stress: bool,
    pub tone_numbers: i32,
    pub ideographs: i32,
    pub textmode: bool,
    pub dotless_i: i32,
    pub listx: i32,
    /// Offset of the `.replace` section in the dictionary's rules.
    pub replace_chars: Option<usize>,
    pub our_alphabet: i32,
    pub alt_alphabet: i32,
    pub alt_alphabet_lang: i32,
    pub max_lengthmod: i32,
    pub lengthen_tonic: i32,
    pub suffix_add_e: i32,
    pub lowercase_sentence: bool,
}

impl Default for LanguageOptions {
    fn default() -> Self {
        LanguageOptions {
            word_gap: 0,
            vowel_pause: 0,
            stress_rule: 0,
            stress_flags: 0,
            unstressed_wd1: 0,
            unstressed_wd2: 0,
            param: [0; N_LOPTS],
            numbers: 0,
            numbers2: 0,
            break_numbers: 0,
            max_roman: 0,
            min_roman: 0,
            thousands_sep: 0,
            decimal_sep: 0,
            max_digits: 0,
            ordinal_indicator: None,
            roman_suffix: b"",
            accents: 0,
            tone_language: 0,
            intonation_group: 0,
            long_stop: 0,
            max_initial_consonants: 0,
            spelling_stress: false,
            tone_numbers: 0,
            ideographs: 0,
            textmode: false,
            dotless_i: 0,
            listx: 0,
            replace_chars: None,
            our_alphabet: 0,
            alt_alphabet: 0,
            alt_alphabet_lang: 0,
            max_lengthmod: 0,
            lengthen_tonic: 0,
            suffix_add_e: 0,
            lowercase_sentence: false,
        }
    }
}

/// `Translator`. Pointers into the dictionary are offsets into `dict`.
#[derive(Debug, Clone)]
pub struct Translator {
    pub langopts: LanguageOptions,
    pub translator_name: i32,
    pub transpose_max: i32,
    pub transpose_min: i32,
    pub transpose_map: Option<&'static [i8]>,
    pub dictionary_name: Vec<u8>,

    pub phonemes_repeat: [u8; 20],
    pub phonemes_repeat_count: i32,
    pub phoneme_tab_ix: i32,

    pub dict_condition: i32,
    pub dict_min_size: i32,
    pub char_plus_apostrophe: &'static [u32],
    pub punct_within_word: &'static [u32],
    pub chars_ignore: &'static [u16],

    pub letter_bits: [u8; 256],
    pub letter_bits_offset: i32,
    pub letter_groups: [Option<&'static [u32]>; 8],

    pub dict: Option<Arc<Dict>>,
    pub frequent_pairs: Option<&'static [i16]>,

    pub expect_verb: i32,
    pub expect_past: i32,
    pub expect_verb_s: i32,
    pub expect_noun: i32,
    pub prev_last_stress: i32,
    /// `clause_end`: index of the last byte of the clause in the clause buffer.
    pub clause_end: isize,

    pub word_vowel_count: i32,
    pub word_stressed_count: i32,

    pub clause_upper_count: i32,
    pub clause_lower_count: i32,

    pub prepause_timeout: i32,
    pub end_stressed_vowel: i32,
    pub prev_dict_flags: [u32; 2],
    pub clause_terminator: i32,
}

/// `PHONEME_LIST2`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PhonemeList2 {
    pub synthflags: u16,
    pub phcode: u8,
    pub stresslevel: u8,
    pub sourceix: u16,
    pub wordstress: u8,
    pub tone_ph: u8,
}

/// `PHONEME_LIST`, without the synthesis-only fields.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PhonemeList {
    pub synthflags: u16,
    pub phcode: u8,
    pub stresslevel: u8,
    pub sourceix: u16,
    pub wordstress: u8,
    pub tone_ph: u8,
    /// `ph`: `None` is C's NULL pointer.
    pub ph: Option<PhonemeTab>,
    pub length: u32,
    pub typ: u8,
    pub newword: u8,
    pub std_length: u8,
}

impl PhonemeList {
    pub fn from2(p: &PhonemeList2) -> Self {
        PhonemeList {
            synthflags: p.synthflags,
            phcode: p.phcode,
            stresslevel: p.stresslevel,
            sourceix: p.sourceix,
            wordstress: p.wordstress,
            tone_ph: p.tone_ph,
            ..Default::default()
        }
    }
}

/// `PHONEME_DATA`, without the synthesis-only fields.
#[derive(Debug, Clone, Copy, Default)]
pub struct PhonemeData {
    pub pd_control: i32,
    pub pd_param: [i32; N_PHONEME_DATA_PARAM],
    pub sound_addr: [i32; 5],
    pub sound_param: [i32; 5],
    pub ipa_string: [u8; 18],
}
