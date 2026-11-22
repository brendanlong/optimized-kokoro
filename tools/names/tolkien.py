"""Tolkien's Sindarin and Quenya names from Eldamo, pronounced by his rules (Appendix E of The Lord of the Rings).

The sounds are the languages' own, written in the symbols Kokoro's other languages use where English has none:
rolled r, pure e and o, x for ch and ht, y for Sindarin y (French u), ç for Quenya hy, ː after long vowels.
"""

import hashlib
import re
import unicodedata
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

WORLD = "Tolkien's legendarium"
ELDAMO_COMMIT = "35c48268993dcab6365c731d7b04883f3166a644"
ELDAMO_URL = f"https://raw.githubusercontent.com/pfstrack/eldamo/{ELDAMO_COMMIT}/src/data/eldamo-data.xml"
ELDAMO_SHA256 = "5f35fdd3a51a7a0f2f055cf7ae740413885e668c92f7370f320795dafec3fa85"
# Sindarin and Quenya as in the published works, not Tolkien's earlier conceptions (Noldorin, Gnomish, Qenya).
LANGUAGES = {"s", "q"}
NAME_KINDS = {"masc-name", "fem-name", "place-name", "proper-name", "collective-name"}
LETTERS = set("abcdefghijklmnopqrstuvwxyzáéíóúâêîôûäëïöüý")

LONG = set("áéíóúâêîôûý")
DIPHTHONGS = {"ai": "ai", "ae": "ae", "au": "au", "aw": "au", "ei": "ei", "oi": "oi", "oe": "oe", "ui": "ui"}
CONSONANT_UNITS = {"th": "θ", "dh": "ð", "ch": "x", "ht": "xt", "ph": "f", "hw": "w", "lh": "l", "rh": "r",
                   "qu": "kw", "nc": "ŋk", "hy": "ç"}
UNITS = sorted({**DIPHTHONGS, **CONSONANT_UNITS}.items(), key=lambda kv: -len(kv[0]))
VOWELS = {"a": "a", "e": "e", "i": "i", "o": "o", "u": "u", "y": "y"}
CONSONANTS = {"c": "k", "g": "ɡ", "x": "ks"}
TWO_CONSONANTS = {"ks", "ŋɡ", "ŋk", "xt"}
KOKORO_VOWELS = set("aeiouyAIOWYɑæəɛɪʊʌɔɜᵊᵻ")
# Unstressed little words inside names: "Gwaith-i-Mírdain", "Taur-im-Duinath", "Haudh-en-Elleth".
PARTICLES = {"i", "in", "im", "en", "na", "nan", "e", "o"}


def fetch(cache: Path) -> Path:
    path = cache / f"eldamo-{ELDAMO_COMMIT[:12]}.xml"
    if not path.exists():
        cache.mkdir(parents=True, exist_ok=True)
        data = urllib.request.urlopen(ELDAMO_URL).read()
        if hashlib.sha256(data).hexdigest() != ELDAMO_SHA256:
            raise SystemExit("eldamo-data.xml doesn't match its pinned hash")
        path.write_bytes(data)
    return path


def base(ch):
    return unicodedata.normalize("NFD", ch)[0]


def segment(word):
    """(phonemes, kind) pieces: 'V' short vowel, 'L' long vowel or diphthong, 'C' consonant."""
    w = word.lower()
    segs, i = [], 0
    while i < len(w):
        unit = next(((src, dst) for src, dst in UNITS if w.startswith(src, i)), None)
        if unit:
            segs.append((unit[1], "L" if unit[0] in DIPHTHONGS else "C"))
            i += len(unit[0])
            continue
        ch, b = w[i], base(w[i])
        if b == "y" and i + 1 < len(w) and base(w[i + 1]) in "aeiou":
            segs.append(("j", "C"))
        elif b in VOWELS:
            segs.append((VOWELS[b] + ("ː" if ch in LONG else ""), "L" if ch in LONG else "V"))
        elif b == "n" and w.startswith("ng", i):
            segs.append(("ŋ" if i + 2 == len(w) else "ŋɡ", "C"))
            i += 1
        else:
            segs.append((CONSONANTS.get(b, b), "C"))
        i += 1
    return segs


def heavy(segs, k):
    """Whether nucleus segs[k]'s syllable is long: a long vowel or diphthong, or two consonants after it."""
    if segs[k][1] == "L":
        return True
    following = 0
    for p, kind in segs[k + 1:]:
        if kind != "C":
            break
        following += (p != "j") + (p in TWO_CONSONANTS)  # ny, ly, ry are one consonant; x, ŋɡ two
    return following >= 2


def stressed(part, mark):
    segs = segment(part)
    nuclei = [k for k, (_, kind) in enumerate(segs) if kind != "C"]
    if part.lower() in PARTICLES or not nuclei:
        target = None
    elif len(nuclei) <= 2:
        target = nuclei[0]
    else:
        target = nuclei[-2] if heavy(segs, nuclei[-2]) else nuclei[-3]
    return "".join((mark if k == target else "") + p for k, (p, _) in enumerate(segs))


def relocate_stress(p):
    """Put each stress mark just before its vowel, where misaki puts it."""
    out, pending = [], ""
    for ch in p:
        if ch in "ˈˌ":
            pending = ch
        elif ch in KOKORO_VOWELS and pending:
            out += [pending, ch]
            pending = ""
        else:
            out.append(ch)
    return "".join(out)


def pronounce(name):
    words = []
    for word in name.split():
        parts = word.split("-")
        main = max((i for i, p in enumerate(parts) if p.lower() not in PARTICLES), default=len(parts) - 1)
        words.append("".join(stressed(p, "ˈ" if i == main else "ˌ") for i, p in enumerate(parts)))
    return relocate_stress(" ".join(words))


def names(cache: Path):
    """Every name as Eldamo spells it, with its pronunciation."""
    out = {}
    for w in ET.parse(fetch(cache)).getroot().iter("word"):
        if w.get("l") not in LANGUAGES or not NAME_KINDS & set(w.get("speech", "").split()):
            continue
        name = re.sub("[¹²³]", "", w.get("v", "")).strip()
        # Leave out reconstructed forms ("Argol(do)"), names that start lowercase, and letters the rules don't cover.
        if not name[:1].isupper() or not all(c.lower() in LETTERS or c in " -" for c in name):
            continue
        out[name] = pronounce(name)
    return out
