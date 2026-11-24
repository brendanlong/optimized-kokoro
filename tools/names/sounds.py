"""espeak-ng's phonemes for a name, as `read_words` gives them, in the symbols of Kokoro's vocabulary.

espeak joins the parts of an affricate or diphthong with "^": those Kokoro has as one symbol keep a tie bar
(t͡ʃ, d͡ʑ, a͡ɪ), and the rest are two sounds. A sound Kokoro has no symbol for becomes the nearest one it has:
Czech ř is r and ʒ run together, Polish ź and ż are ʒ, Norwegian and Swedish ʉ is y. Stress and length are
kept; Vietnamese tones (espeak's digits), marks Kokoro doesn't read (syllabic, dental, non-syllabic), Danish
stød and German's glottal onsets are dropped.
"""

import unicodedata

TIED = {"tʃ", "ts", "dʒ", "dz", "tɕ", "dʑ", "ʈʂ", "aɪ", "aʊ", "eɪ", "oʊ", "əʊ", "ɔɪ", "ss"}
TIE = "\u0361"
NEAREST = {"r\u031d\u030a": "rʃ", "r\u031d": "rʒ", "s^x": "ʃ", "ʐ": "ʒ", "ʑ": "ʒ", "ʉ": "y", "ʏ": "y", "ɵ": "ø",
           "ɫ": "l", "ɭ": "l", "ɗ": "d", "ε": "ɛ", "ɬ": "l", "ɱ": "m",
           # Romanian's rising diphthongs (Gheorghe, Oana, Badea).
           "e^a": "ja", "o^a": "wa", "ɔ^a": "wa", "e^o": "jo"}
DROPPED = {"\u0329", "\u032f", "\u032a", "#", "-", "ʔ", "?", *"0123456789"}


VOWELS = set("aeiouyɑɐɒæɛɜəɘɪɨʉʊʌɔøœɤɯɵʏε")


def units(raw):
    """Each sound espeak wrote, in NFD: a letter with its marks, and two letters joined by "^" as one, "a^ɪ"
    (a "^" beside anything else, as where a dropped stød was, joins nothing)."""
    out, join = [], False
    for c in unicodedata.normalize("NFD", raw):
        if c in DROPPED:
            continue
        letter = unicodedata.category(c).startswith("L")
        if c == "^":
            join = bool(out) and "^" not in out[-1] and unicodedata.category(out[-1][0]).startswith("L")
        elif out and unicodedata.category(c) == "Mn":
            out[-1] += c
        elif join and letter:
            out[-1] += "^" + c
            join = False
        else:
            out.append(c)
            join = False
    return out


def kokoro(raw):
    """The reading in Kokoro's symbols: decomposed, as Kokoro has ̃ but not ũ, except ç, which it has whole."""
    us = units(raw)
    # A palatal mark after a sound already palatal, or before i or j, adds nothing (espeak's Polish "ɲʲ", "bʲˈi").
    keep = []
    for k, u in enumerate(us):
        after = next((v for v in us[k + 1:] if v not in "ˈˌ"), "")
        if u == "ʲ" and (keep and unicodedata.normalize("NFC", keep[-1])[-1] in "ɲɕʑʎjçʲ" or after[:1] in ("i", "j")):
            continue
        keep.append(u)
    out = []
    for u in keep:
        if u in NEAREST:
            out.append(NEAREST[u])
        elif "^" in u:
            a, b = u.split("^")
            out.append(a + TIE + b if a + b in TIED else NEAREST.get(a, a) + NEAREST.get(b, b))
        else:
            out.append(NEAREST.get(u, u))
    return unicodedata.normalize("NFD", "".join(out)).replace("c\u0327", "ç")


def has_vowel(phonemes):
    return any(c in VOWELS for c in unicodedata.normalize("NFD", phonemes))
