"""Rewrites misaki 0.9.4's lexicons (misaki/data/{us,gb}_{gold,silver}.json) in standard IPA.

usage: lexicon_to_ipa.py MISAKI_DATA_DIR OUT_DIR

misaki writes for Kokoro's vocabulary: one letter per sound (A for e͡ɪ, ʧ for t͡ʃ), the American flap
as ɾ and some t's as ʔ (which kokoro's `version=None` makes the English flap and t), and stress marks
just before the vowel. This writes what kokoro reads as IPA: tie bars for sounds of two letters, t̬
for the flap, t for ʔ, and stress marks
at the start of the syllable, taking the longest onset English allows. kokoro-phonemes reads the
result as the same ids as misaki's, but for 3 entries whose stress mark misaki put before a
consonant (docs/divergences.md, "Phonemes: stress marks before a consonant"); that was checked
once, when converting. `cargo test -p kokoro-g2p-en --test lexicon_ipa` checks the result is IPA
the vocabulary reads whole.
"""

import hashlib
import json
import sys
from pathlib import Path

SHA256 = {
    "us_gold.json": "dc414872a49a28ae6c141463d502fd945f3b2fde040484fdc47d00cc4612686f",
    "us_silver.json": "de8f67be911bb6c659187b4a65fd966b6a30e56350e0f790d763210b053ac475",
    "gb_gold.json": "29e62f4b60261c88f7f3c2c7811ca3825978948090b72d2b27d565b729282f71",
    "gb_silver.json": "48131e2d92ccc41655f4543e87e0f938e71463eb5a54be7f0693bb712ebb6bce",
}

SYMBOLS = {"A": "e͡ɪ", "I": "a͡ɪ", "O": "o͡ʊ", "Q": "ə͡ʊ", "W": "a͡ʊ", "Y": "ɔ͡ɪ", "ʤ": "d͡ʒ", "ʧ": "t͡ʃ", "ɾ": "t̬", "ʔ": "t"}
VOWELS = set("AIOQWYaeiouæɑɒɔəɛɜɪʊʌᵊᵻ")
CONSONANTS = set("bdfhjklmnpstvwzðŋɡɹɾʃʒʔθʤʧ")
STRESS = "ˈˌ"
# English syllable onsets of more than one consonant, in misaki's letters.
CLUSTERS = {
    *"pl pɹ pj bl bɹ bj tɹ tw tj dɹ dw dj kl kɹ kw kj ɡl ɡɹ ɡw ɡj fl fɹ fj vj θɹ θw θj ʃɹ".split(),
    *"sl sw sm sn sp st sk sj sf mj nj hj lj spl spɹ spj stɹ stj skɹ skw skl skj".split(),
}


def onset_ok(onset):
    if len(onset) == 1:
        return onset != "ŋ"
    return onset in CLUSTERS


def restress(ps):
    """Moves each stress mark from before its vowel to before the longest onset ahead of it."""
    out = list(ps)
    i = 0
    while i < len(out):
        if out[i] in STRESS and i + 1 < len(out) and out[i + 1] in VOWELS:
            start = i
            while start > 0 and out[start - 1] in CONSONANTS and onset_ok("".join(out[start - 1 : i])):
                start -= 1
            out.insert(start, out.pop(i))
        i += 1
    return "".join(out)


def to_ipa(ps):
    if ps is None:
        return None
    return "".join(SYMBOLS.get(c, c) for c in restress(ps))


def convert(value):
    if isinstance(value, dict):
        return {tag: to_ipa(ps) for tag, ps in value.items()}
    return to_ipa(value)


def main(src, out):
    for name, sha in SHA256.items():
        raw = (Path(src) / name).read_bytes()
        if hashlib.sha256(raw).hexdigest() != sha:
            raise SystemExit(f"{name} isn't misaki 0.9.4's")
        lexicon = {word: convert(value) for word, value in json.loads(raw).items()}
        text = json.dumps(lexicon, ensure_ascii=False, indent=2)
        (Path(out) / name).write_text(text, encoding="utf-8")


if __name__ == "__main__":
    main(*sys.argv[1:])
