"""Japanese names in Hepburn romanization ("Kurosawa", "Ryūnosuke", "Ohtani"), in Kokoro's sounds.

A word is read only if it is wholly Japanese morae: a consonant (or none) and a vowel, a doubled consonant before
one, or n. Long vowels are written several ways, all read long: ō, ô, oo, ou (not before a vowel: Inoue is
i-no-u-e), oh before a consonant or at the end (Ohtani, Saitoh); ū, uu; ā, aa; ē, ee; ī, ii; ei is eː. Japanese
has a pitch accent, not stress, which names don't show in writing, so the readings have no stress marks, as
misaki's Japanese G2P writes none.

The sounds: u is ɯ, r the tap ɾ, f ɸ, shi ɕi, chi t͡ɕi, ji d͡ʑi, tsu t͡sɯ, hi çi, and ky, ny, ry and the rest
palatalized; n before a vowel or y is n, and otherwise ends its mora (ŋ before k and g, m before b, p and m).
"""

import re
import unicodedata

VOWELS = {"a": "a", "i": "i", "u": "ɯ", "e": "e", "o": "o"}
LONG = {"ā": "a", "ī": "i", "ū": "u", "ē": "e", "ō": "o", "â": "a", "î": "i", "û": "u", "ê": "e", "ô": "o"}
# Consonants, longest first; each is followed by a vowel.
ONSETS = {"ky": "kʲ", "gy": "ɡʲ", "sh": "ɕ", "ch": "t͡ɕ", "j": "d͡ʑ", "ts": "t͡s", "ny": "ɲ", "hy": "ç", "by": "bʲ",
          "py": "pʲ", "my": "mʲ", "ry": "ɾʲ", "k": "k", "g": "ɡ", "s": "s", "z": "z", "t": "t", "d": "d", "n": "n",
          "h": "h", "f": "ɸ", "b": "b", "p": "p", "m": "m", "y": "j", "r": "ɾ", "w": "w"}
# Morae Japanese hasn't, which make a word not Japanese ("ti", "wu", "ye").
NOT_JAPANESE = {"ti", "di", "tu", "du", "si", "zi", "hu", "wu", "wi", "we", "yi", "ye", "ci", "ce"}
SPECIAL = {"shi": "ɕi", "chi": "t͡ɕi", "ji": "d͡ʑi", "tsu": "t͡sɯ", "fu": "ɸɯ", "hi": "çi", "wo": "o"}


def normalize(word):
    word = unicodedata.normalize("NFC", word.lower())
    for long, short in LONG.items():
        word = word.replace(long, short * 2)
    word = re.sub(r"oh(?=[^aeiouy]|$)", "oo", word)
    word = re.sub(r"ou(?![aeiou])", "oo", word)
    return word


def read(spelling):
    """The name's phonemes, or None if it isn't Hepburn Japanese."""
    word = normalize(spelling)
    if not word.isascii() or not word.isalpha():
        return None
    out, i = [], 0
    while i < len(word):
        c = word[i]
        if c in VOWELS:
            before = out[-1][-1] if out and out[-1][0] in "VM" else ""
            if before == c or before == "e" and c == "i":
                out[-1] = out[-1] + "ː"
            else:
                out.append("V" + c)
            i += 1
            continue
        if c == "n" and (i + 1 == len(word) or word[i + 1] not in "aeiouy"):
            out.append("N")
            i += 1
            continue
        if i + 1 < len(word) and word[i + 1] == c and c not in "n" or word.startswith("tch", i):
            out.append("Q")
            i += 1
            continue
        onset = next((o for o in sorted(ONSETS, key=len, reverse=True) if word.startswith(o, i)), None)
        if onset is None or i + len(onset) >= len(word) or word[i + len(onset)] not in VOWELS:
            return None
        mora = onset + word[i + len(onset)]
        if mora in NOT_JAPANESE:
            return None
        out.append("M" + mora)
        i += len(mora)
    return "".join(sound(out, k) for k in range(len(out))) or None


def sound(morae, k):
    m = morae[k]
    kind, body = m[0], m[1:]
    if kind == "V":
        long = body.endswith("ː")
        return VOWELS[body[0]] + ("ː" if long else "")
    if kind == "N":
        after = morae[k + 1][1:] if k + 1 < len(morae) else ""
        return "ŋ" if after[:1] in ("k", "g") else "m" if after[:1] in ("b", "p", "m") else "n"
    if kind == "Q":
        return ""
    long = body.endswith("ː")
    body = body.rstrip("ː")
    mora = SPECIAL.get(body) or ONSETS[body[:-1]] + VOWELS[body[-1]]
    if k > 0 and morae[k - 1] == "Q":
        mora = mora[0] + "ː" + mora[1:] if not mora.startswith(("t͡", "d͡")) else mora[:3] + "ː" + mora[3:]
    return mora + ("ː" if long else "")
