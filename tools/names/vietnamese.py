"""Vietnamese names in their own spelling (chữ Quốc ngữ: "Nguyễn", "Thị", "Phạm"), in Kokoro's sounds.

Each word is one syllable: an onset, a rhyme of vowels with an optional glide, and an optional final consonant. A
word that doesn't parse so isn't read. The sounds are Hanoi's, the standard: d, gi and r are z, tr and ch t͡ɕ,
x s, đ d, g and gh ɣ, kh x, nh ɲ, ng and ngh ŋ, ph f, th tʰ; a is long aː but in ay, au, anh and ach, and the
rhymes Hanoi changes are changed (ong awŋ, ông əwŋ, ênh əjŋ, ưu iw). The tones, which the marks ` ´ ̉ ˜ ̣ write, are left
out: Kokoro's English voices can't say them. Each word is stressed, as each Vietnamese syllable is full.
"""

import unicodedata

ONSETS = [("ngh", "ŋ"), ("ng", "ŋ"), ("nh", "ɲ"), ("gh", "ɣ"), ("gi", "z"), ("kh", "x"), ("ph", "f"), ("th", "tʰ"),
          ("tr", "t͡ɕ"), ("ch", "t͡ɕ"), ("qu", "kw"), ("b", "b"), ("c", "k"), ("d", "z"), ("đ", "d"), ("g", "ɣ"),
          ("h", "h"), ("k", "k"), ("l", "l"), ("m", "m"), ("n", "n"), ("p", "p"), ("r", "z"), ("s", "s"), ("t", "t"),
          ("v", "v"), ("x", "s")]
# Rhymes' vowels, with their glides (o and u before a vowel are w, i and y after one j, o and u after one w).
NUCLEI = {"a": "aː", "ă": "a", "â": "ə", "e": "ɛ", "ê": "e", "i": "i", "y": "i", "o": "ɔ", "ô": "o", "ơ": "əː",
          "u": "u", "ư": "ɨ", "ia": "iə", "iê": "iə", "yê": "iə", "ya": "iə", "ua": "uə", "uô": "uə", "ưa": "ɨə",
          "ươ": "ɨə", "oa": "waː", "oă": "wa", "oe": "wɛ", "uy": "wi", "uê": "we", "uâ": "wə", "uyê": "wiə",
          "uya": "wiə"}
GLIDES = {"i": "j", "y": "j", "o": "w", "u": "w"}
CODAS = [("ch", "k"), ("nh", "ŋ"), ("ng", "ŋ"), ("c", "k"), ("m", "m"), ("n", "n"), ("p", "p"), ("t", "t")]
TONES = {"̀", "́", "̉", "̃", "̣"}


def untoned(word):
    """The word without its tone marks, keeping the marks that make letters (â, ă, ê, ô, ơ, ư)."""
    return unicodedata.normalize("NFC", "".join(c for c in unicodedata.normalize("NFD", word) if c not in TONES))


def read(spelling):
    """The name's phonemes, or None if it isn't a Vietnamese syllable."""
    # Ð (U+00D0, Icelandic's eth) is often typed for Đ.
    word = untoned(spelling.lower().replace("ð", "đ"))
    onset = next(((o, s) for o, s in ONSETS if word.startswith(o)), ("", ""))
    rest = word[len(onset[0]):]
    if onset[0] == "gi":
        if rest.startswith("oa"):
            return None  # Gioan is two syllables, Gio-an.
        if not rest or rest[0] not in "aăâeoôơuưy":
            rest = "i" + rest  # gi with i of its own (gìn) or of iê (giếng).
    if onset[0] == "qu" and rest.startswith("ô"):
        onset, rest = ("c", "k"), "u" + rest  # Quốc is k-uô-c.
    coda = next(((c, s) for c, s in CODAS if rest.endswith(c) and len(rest) > len(c)), ("", ""))
    vowels = rest[:len(rest) - len(coda[0])]
    glide = ""
    if vowels not in NUCLEI and vowels[-1:] in GLIDES and vowels[:-1] in NUCLEI and not coda[0]:
        vowels, glide = vowels[:-1], GLIDES[vowels[-1]]
    nucleus = NUCLEI.get(vowels)
    if nucleus is None:
        return None
    # a is long but before the glides of ay and au (short ă); Hanoi's a before nh and ch is short aj (anh, ach,
    # oanh), its ê before them əj (kênh, ếch); o and ô before ng and c are aw and əw (Trọng, Công, Ngọc); ưu is iw.
    if vowels in ("a", "oa") and (glide and spelling.lower()[-1:] in "yu" and not spelling.lower().endswith("ao")):
        nucleus = nucleus.replace("aː", "a")
    if vowels in ("a", "oa") and coda[0] in ("nh", "ch"):
        nucleus = nucleus.replace("aː", "a") + "j"
    if vowels == "ê" and coda[0] in ("nh", "ch"):
        nucleus = "əj"
    if vowels in ("o", "ô") and coda[0] in ("ng", "c"):
        nucleus = "aw" if vowels == "o" else "əw"
    if vowels == "ư" and glide == "w":
        nucleus = "i"
    return "ˈ" + onset[1] + nucleus + glide + coda[1]
