"""Tolkien's spelling and stress rules (Appendix E of The Lord of the Rings), in one of two symbol sets.

english: misaki's American English symbols (ɹ, ɛ, ɔ, diphthongs as A I O W Y), with Tolkien's vowel qualities.
native:  Kokoro's other-language symbols where English has none: rolled r, pure e and o, x, ü (y), long vowels with ː.
"""

import json
import re
import sys
import unicodedata

from phon import relocate_stress

LONG = set("áéíóúâêîôûý")
SETS = {
    "english": {
        "units": {"ai": "I", "ae": "I", "au": "W", "aw": "W", "ei": "A", "oi": "Y", "oe": "Y", "ui": "uɪ",
                  "th": "θ", "dh": "ð", "ch": "h", "ph": "f", "hw": "w", "lh": "l", "rh": "ɹ", "qu": "kw",
                  "nc": "ŋk", "hy": "hj"},
        "vowels": {"a": "ɑ", "e": "ɛ", "i": "i", "o": "ɔ", "u": "u", "y": "i"},
        "cons": {"c": "k", "g": "ɡ", "r": "ɹ", "x": "ks"},
        "long": "", "geminates": False,
    },
    "native": {
        "units": {"ai": "ai", "ae": "ae", "au": "au", "aw": "au", "ei": "ei", "oi": "oi", "oe": "oe", "ui": "ui",
                  "th": "θ", "dh": "ð", "ch": "x", "ht": "xt", "ph": "f", "hw": "w", "lh": "l", "rh": "r",
                  "qu": "kw", "nc": "ŋk", "hy": "ç"},
        "vowels": {"a": "a", "e": "e", "i": "i", "o": "o", "u": "u", "y": "y"},
        "cons": {"c": "k", "g": "ɡ", "r": "r", "x": "ks"},
        "long": "ː", "geminates": True,
    },
}
DIPHTHONGS = {"ai", "ae", "au", "aw", "ei", "oi", "oe", "ui"}
TWO_CONSONANTS = {"ks", "ŋɡ", "ŋk", "xt"}


def segment(word, s):
    """Split a word into (phonemes, kind): 'V' short vowel, 'L' long vowel or diphthong, 'C' consonant(s)."""
    w = word.lower()
    units = sorted(s["units"], key=len, reverse=True)
    segs, i = [], 0
    while i < len(w):
        src = next((u for u in units if w.startswith(u, i)), None)
        if src:
            segs.append((s["units"][src], "L" if src in DIPHTHONGS else "C"))
            i += len(src)
            continue
        ch = w[i]
        base = unicodedata.normalize("NFD", ch)[0]
        if base == "y" and i + 1 < len(w) and unicodedata.normalize("NFD", w[i + 1])[0] in "aeiou":
            segs.append(("j", "C"))
        elif base in s["vowels"]:
            long = ch in LONG
            segs.append((s["vowels"][base] + (s["long"] if long else ""), "L" if long else "V"))
        elif base == "n" and w.startswith("ng", i):
            segs.append(("ŋ" if i + 2 == len(w) else "ŋɡ", "C"))
            i += 1
        elif base.isalpha():
            segs.append((s["cons"].get(base, base), "C"))
        i += 1
    return segs


def heavy(segs, k):
    """Whether the syllable of nucleus segs[k] is long: a long vowel, or followed by two consonants."""
    if segs[k][1] == "L":
        return True
    following = 0
    for p, kind in segs[k + 1:]:
        if kind != "C":
            break
        # Quenya ny, ly, ry... are single consonants; x and ŋɡ are two.
        following += (p != "j") + (p in TWO_CONSONANTS)
    return following >= 2


def stress_part(part, mark, s):
    segs = segment(part, s)
    nuclei = [k for k, (_, kind) in enumerate(segs) if kind != "C"]
    if len(nuclei) <= 2:
        target = nuclei[0] if nuclei else None
    else:
        target = nuclei[-2] if heavy(segs, nuclei[-2]) else nuclei[-3]
    out = "".join((mark if k == target else "") + p for k, (p, _) in enumerate(segs))
    # English has no geminates: Ossë is "Osse", not "Oss-se".
    return out if s["geminates"] else re.sub(r"(.)\1", r"\1", out)


def pronounce(name, symbols="english"):
    s = SETS[symbols]
    out = []
    for word in name.split():
        parts = word.split("-")
        out.append("".join(stress_part(p, "ˈ" if i == len(parts) - 1 else "ˌ", s) for i, p in enumerate(parts)))
    return relocate_stress(" ".join(out))


def main(key, out, symbols="english"):
    with open(out, "w") as f:
        for line in open(key):
            name = json.loads(line)["name"]
            f.write(json.dumps({"name": name, "phonemes": pronounce(name, symbols)}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:])
