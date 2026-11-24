"""Mandarin names in Hanyu Pinyin without tone marks ("Jinping", "Xiaoping"), in Kokoro's sounds.

A word is read only if it splits wholly into pinyin syllables, the longest first ("Xian" is xian, not xi'an,
as pinyin itself would write xi'an with an apostrophe): Wade-Giles ("Chiang", "Hsiao") and English words that
aren't pinyin don't. Tones aren't written in English text and Kokoro's English voices can't say them.

The sounds are Mandarin's, written as an English voice says them best: b, d, g voiced as English hears them
and p, t, k, which English aspirates already, for their aspirated pairs; zh, ch, sh as English j, ch, sh; j, q,
x as the alveolo-palatal d͡ʑ, t͡ɕ, ɕ; r as ɻ; the i after z, c, s, zh, ch, sh, r as ɨ. A word of several
syllables is stressed on the last, which Mandarin's full tones make the most prominent. y and w keep a glide
(Ying is jiŋ, not English "ing"; Boyu is bo-ɥy, not "boy").
"""

INITIALS = {"zh": "d͡ʒ", "ch": "t͡ʃ", "sh": "ʃ", "b": "b", "p": "p", "m": "m", "f": "f", "d": "d", "t": "t",
            "n": "n", "l": "l", "g": "ɡ", "k": "k", "h": "x", "j": "d͡ʑ", "q": "t͡ɕ", "x": "ɕ", "r": "ɻ",
            "z": "d͡z", "c": "t͡s", "s": "s"}
FINALS = {"a": "a", "o": "o", "e": "ɤ", "ai": "a͡ɪ", "ei": "e͡ɪ", "ao": "a͡ʊ", "ou": "o͡ʊ", "an": "an", "en": "ən",
          "ang": "aŋ", "eng": "əŋ", "ong": "ʊŋ", "i": "i", "ia": "ja", "ie": "je", "iao": "ja͡ʊ", "iu": "jo͡ʊ",
          "ian": "jɛn", "in": "in", "iang": "jaŋ", "ing": "iŋ", "iong": "jʊŋ", "u": "u", "ua": "wa", "uo": "wo",
          "uai": "wa͡ɪ", "ui": "we͡ɪ", "uan": "wan", "un": "wən", "uang": "waŋ", "v": "y", "ve": "ɥe", "ue": "ɥe"}
# With j, q, x (and y), u is ü.
FRONTED = {"u": "y", "ue": "ɥe", "uan": "ɥɛn", "un": "yn"}
ZERO = {"a": "a", "o": "o", "e": "ɤ", "ai": "a͡ɪ", "ei": "e͡ɪ", "ao": "a͡ʊ", "ou": "o͡ʊ", "an": "an", "en": "ən",
        "ang": "aŋ", "eng": "əŋ", "er": "aɚ", "yi": "ji", "ya": "ja", "ye": "je", "yao": "ja͡ʊ", "you": "jo͡ʊ",
        "yan": "jɛn", "yin": "jin", "yang": "jaŋ", "ying": "jiŋ", "yong": "jʊŋ", "yu": "ɥy", "yue": "ɥe",
        "yuan": "ɥɛn", "yun": "ɥyn", "wu": "wu", "wa": "wa", "wo": "wo", "wai": "wa͡ɪ", "wei": "we͡ɪ", "wan": "wan",
        "wen": "wən", "wang": "waŋ", "weng": "wəŋ"}
# The finals each initial takes (a little more than Mandarin has is harmless: names are real syllables).
LABIAL = "a o ai ei ao an en ang eng i ie iao ian in ing u ou iu"
TAKES = {
    "b": LABIAL, "p": LABIAL, "m": LABIAL + " e", "f": "a o ei ou an en ang eng u",
    "d": "a e ai ei ao ou an en ang eng i ie iao iu ian ing u uo ui uan un ong",
    "t": "a e ai ao ou an ang eng i ie iao ian ing u uo ui uan un ong",
    "n": "a e ai ei ao ou an en ang eng i ie iao iu ian in iang ing u uo uan ong v ve",
    "l": "a e ai ei ao ou an ang eng i ia ie iao iu ian in iang ing u uo uan un ong v ve",
    "g": "a e ai ei ao ou an en ang eng u ua uo uai ui uan un uang ong",
    "k": "a e ai ao ou an en ang eng u ua uo uai ui uan un uang ong",
    "h": "a e ai ei ao ou an en ang eng u ua uo uai ui uan un uang ong",
    "j": "i ia ie iao iu ian in iang ing iong u ue uan un",
    "q": "i ia ie iao iu ian in iang ing iong u ue uan un",
    "x": "i ia ie iao iu ian in iang ing iong u ue uan un",
    "zh": "a e i ai ei ao ou an en ang eng u ua uo uai ui uan un uang ong",
    "ch": "a e i ai ao ou an en ang eng u ua uo uai ui uan un uang ong",
    "sh": "a e i ai ei ao ou an en ang eng u ua uo uai ui uan un uang",
    "r": "e i ao ou an en ang eng u ua uo ui uan un ong",
    "z": "a e i ai ei ao ou an en ang eng u uo ui uan un ong",
    "c": "a e i ai ao ou an en ang eng u uo ui uan un ong",
    "s": "a e i ai ao ou an en ang eng u uo ui uan un ong",
}
BUZZED = {"z", "c", "s", "zh", "ch", "sh", "r"}


def syllables():
    out = dict(ZERO)
    for initial, finals in TAKES.items():
        for final in finals.split():
            if initial in "jqx" and final in FRONTED:
                sound = FRONTED[final]
            elif final == "i" and initial in BUZZED:
                sound = "ɨ"
            else:
                sound = FINALS[final]
            out[initial + final] = INITIALS[initial] + sound
    return out


SYLLABLES = syllables()


def split(word, first=True):
    """The syllables of `word`, longest first, or None if it isn't wholly pinyin. Only the first may start with
    a, o or e: pinyin writes an apostrophe before a later one (xi'an), so "Chiang" isn't chi-ang."""
    if not word:
        return []
    if not first and word[0] in "aoe":
        return None
    for n in range(min(6, len(word)), 0, -1):
        if word[:n] in SYLLABLES:
            rest = split(word[n:], False)
            if rest is not None:
                return [word[:n], *rest]
    return None


def read(spelling):
    """The name's phonemes, or None if it isn't pinyin."""
    parts = [s for piece in spelling.lower().replace("ü", "v").replace("’", "'").split("'")
             for s in (split(piece) or [None])]
    parts = None if None in parts else parts
    if not parts or not spelling.isascii() and "ü" not in spelling.lower():
        return None
    sounds = [SYLLABLES[p] for p in parts]
    sounds[-1] = "ˈ" + sounds[-1]
    return "".join(sounds)
