"""Tolkien's spelling and stress rules (Appendix E of The Lord of the Rings), written in misaki phonemes."""

import json
import re
import sys
import unicodedata

from phon import relocate_stress

LONG = set("áéíóúâêîôûý")
# Spelling -> (phonemes, is a vowel nucleus, is long/diphthong). Longest match first.
UNITS = [
    ("ai", "I", True), ("ae", "I", True), ("au", "W", True), ("aw", "W", True), ("ei", "A", True),
    ("oi", "Y", True), ("oe", "Y", True), ("ui", "uɪ", True),
    ("th", "θ", None), ("dh", "ð", None), ("ch", "h", None), ("ph", "f", None), ("hw", "w", None),
    ("lh", "l", None), ("rh", "ɹ", None), ("qu", "kw", None), ("nc", "ŋk", None), ("hy", "hj", None),
]
VOWEL = {"a": "ɑ", "e": "ɛ", "i": "i", "o": "ɔ", "u": "u", "y": "i"}
CONS = {"c": "k", "g": "ɡ", "r": "ɹ", "x": "ks", "k": "k", "z": "z"}


def segment(word):
    """Split a word into (phonemes, kind) where kind is 'V' (short vowel), 'L' (long vowel or diphthong) or 'C'."""
    w = word.lower()
    segs, i = [], 0
    while i < len(w):
        for src, dst, diph in UNITS:
            if w.startswith(src, i):
                segs.append((dst, "L" if diph else "C"))
                i += len(src)
                break
        else:
            ch = w[i]
            base = unicodedata.normalize("NFD", ch)[0]
            if base == "y" and i + 1 < len(w) and unicodedata.normalize("NFD", w[i + 1])[0] in VOWEL:
                segs.append(("j", "C"))
            elif base in VOWEL:
                segs.append((VOWEL[base], "L" if ch in LONG else "V"))
            elif base == "n" and w.startswith("ng", i):
                segs.append(("ŋ" if i + 2 == len(w) else "ŋɡ", "C"))
                i += 1
            elif base.isalpha():
                segs.append((CONS.get(base, base), "C"))
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
        # Quenya ny, ly, ry, tw... are single consonants; x and ŋɡ are two.
        following += (p != "j") + (p in ("ks", "ŋɡ", "ŋk"))
    return following >= 2


def stress_part(part, mark):
    segs = segment(part)
    nuclei = [k for k, (_, kind) in enumerate(segs) if kind != "C"]
    if len(nuclei) <= 2:
        target = nuclei[0] if nuclei else None
    else:
        target = nuclei[-2] if heavy(segs, nuclei[-2]) else nuclei[-3]
    out = []
    for k, (p, _) in enumerate(segs):
        if k == target:
            out.append(mark)
        out.append(p)
    # English has no geminates: Ossë is "Osse", not "Oss-se".
    return re.sub(r"(.)\1", r"\1", "".join(out))


def pronounce(name):
    out = []
    for word in name.split():
        parts = word.split("-")
        out.append("".join(stress_part(p, "ˈ" if i == len(parts) - 1 else "ˌ") for i, p in enumerate(parts)))
    return relocate_stress(" ".join(out))


def main(key, out):
    with open(out, "w") as f:
        for line in open(key):
            name = json.loads(line)["name"]
            f.write(json.dumps({"name": name, "phonemes": pronounce(name)}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:])
