"""Shared phoneme handling: Kokoro's vocabulary, native IPA to misaki's American English set, and scoring."""

import re
from pathlib import Path

VOCAB_RS = Path(__file__).resolve().parents[2] / "crates/pipeline/src/vocab.rs"
VOCAB = {chr(int(h, 16)) for h in re.findall(r"\('\\u\{([0-9a-f]+)\}', \d+\)", VOCAB_RS.read_text())}

VOWELS = set("AIOWYɑæəɛɪiʊuʌɔɜᵊᵻaeo")
STRESS = "ˈˌ"

# Native (Elvish) IPA -> American English approximations in misaki's symbols. Longest match first.
IPA_MAP = [
    ("a͡ɪ", "I"), ("aɪ", "I"), ("ai", "I"), ("a͡ɛ", "I"), ("aɛ", "I"), ("ae", "I"),
    ("a͡ʊ", "W"), ("aʊ", "W"), ("au", "W"),
    ("ɛ͡ɪ", "A"), ("ɛɪ", "A"), ("eɪ", "A"), ("ei", "A"),
    ("ɔɪ", "Y"), ("oɪ", "Y"), ("oi", "Y"), ("ɔi", "Y"), ("oe", "Y"),
    ("rʲ", "ɹj"), ("ʲ", ""), ("kʷ", "kw"), ("ɡʷ", "ɡw"), ("ŋʷ", "ŋ"), ("ʷ", "w"),
    ("a", "ɑ"), ("e", "ɛ"), ("o", "ɔ"), ("y", "i"), ("ʏ", "ʊ"), ("ü", "u"),
    ("r", "ɹ"), ("ɾ", "ɹ"), ("x", "h"), ("ç", "h"), ("ʎ", "lj"), ("ɲ", "nj"), ("ɬ", "l"),
    ("ʍ", "w"), ("β", "v"), ("ɟ", "ɡ"), ("g", "ɡ"), ("c", "k"),
]
DROP = "/[]{}.ːˑ͡"


def relocate_stress(p):
    """Move each stress mark to just before its syllable's vowel, where misaki puts it."""
    out, pending = [], ""
    for ch in p:
        if ch in STRESS:
            pending = ch
        elif ch in VOWELS and pending:
            out += [pending, ch]
            pending = ""
        else:
            if ch == " " and pending:
                out.append(pending)
                pending = ""
            out.append(ch)
    return "".join(out) + pending


def ipa_to_misaki(ipa):
    s = ipa.replace("'", "ˈ").replace("ˈ", "ˈ")
    s = "".join(c for c in s if c not in DROP)
    out, i = [], 0
    while i < len(s):
        for src, dst in IPA_MAP:
            if s.startswith(src, i):
                out.append(dst)
                i += len(src)
                break
        else:
            out.append(s[i])
            i += 1
    return relocate_stress(re.sub(r"(.)\1", r"\1", "".join(out)))  # English has no geminates


def clean_misaki(p):
    """Strip delimiters an LLM may add, put stress where misaki does, and report characters Kokoro doesn't know."""
    p = p.strip().strip("/[]").replace("'", "ˈ").replace("ɾ", "T")
    p = relocate_stress(p)
    return p, sorted({c for c in p if c not in VOCAB})


def words(p):
    return [w for w in re.split(r"[ ]+", p.strip(" .,")) if w]


def stress_index(word):
    """Which vowel (0-based) carries the primary stress, or None if none is marked."""
    n = 0
    for ch in word:
        if ch == "ˈ":
            return n
        if ch in VOWELS:
            n += 1
    return None


def vowel_count(word):
    return sum(ch in VOWELS for ch in word)


# Sounds that count as the same consonant: flaps are t, and Tolkien's [x] may be said as h or k.
CONS_CLASS = {"T": "t", "ɾ": "t", "r": "ɹ", "ʤ": "ʤ", "x": "h"}
SOFT = {("h", "k"), ("k", "h")}


def consonants(word):
    return [CONS_CLASS.get(c, c) for c in word if c not in VOWELS and c not in STRESS and c not in "-ː"]


def edit_distance(a, b, key_had_x=False):
    d = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        prev, d[0] = d[0], i
        for j, y in enumerate(b, 1):
            cost = 0 if x == y or (key_had_x and (x, y) in SOFT) else 1
            prev, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, prev + cost)
    return d[-1]
