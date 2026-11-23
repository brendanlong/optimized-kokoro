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
           "ɫ": "l", "ɭ": "l", "ɗ": "d", "ε": "ɛ", "ɬ": "l"}
DROPPED = {"\u0329", "\u032f", "\u032a", "#", "-", "ʔ", "?", *"0123456789"}


def units(raw):
    """Each sound espeak wrote: a letter with its marks, and two joined by "^" as one, "a^ɪ"."""
    out = []
    for c in unicodedata.normalize("NFD", raw):
        if c in DROPPED:
            continue
        if out and (unicodedata.category(c) == "Mn" or out[-1].endswith("^")):
            out[-1] += c
        elif c == "^":
            if out:
                out[-1] += c
        else:
            out.append(c)
    return [u.rstrip("^") for u in out]


def kokoro(raw):
    out = []
    for u in units(raw):
        if u in NEAREST:
            out.append(NEAREST[u])
        elif "^" in u:
            a, b = u.split("^", 1)
            out.append(a + TIE + b if a + b in TIED else NEAREST.get(a, a) + NEAREST.get(b, b))
        else:
            out.append(NEAREST.get(u, u))
    # A palatal mark after a sound already palatal, or before i or j, adds nothing (espeak's Polish "ɲʲ", "bʲˈi").
    keep = []
    for k, sound in enumerate(out):
        after = next((u for u in out[k + 1:] if u not in "ˈˌ"), "")
        if sound == "ʲ" and (keep and keep[-1][-1] in "ɲɕʑʎjç" or after[:1] in ("i", "j")):
            continue
        keep.append(sound)
    out = keep
    return "".join(out)
