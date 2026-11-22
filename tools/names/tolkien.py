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
# Sindarin and Quenya, and the Noldorin and Middle Quenya of the 1930s-50s that much of the published
# Silmarillion spells its names in (Valinor, Valmar); not the earliest conceptions (Gnomish, Qenya).
LANGUAGES = {"s", "q", "n", "mq"}
SINDARIN = {"s", "n"}
LETTERS = set("abcdefghijklmnopqrstuvwxyzáéíóúâêîôûäëïöüý")

LONG = set("áéíóúâêîôûý")
DIPHTHONGS = {"ai": "ai", "ae": "ae", "au": "au", "aw": "au", "ei": "ei", "oi": "oi", "oe": "oe", "ui": "ui"}
CONSONANT_UNITS = {"nqu": "ŋkw", "th": "θ", "dh": "ð", "ch": "x", "ht": "xt", "ph": "f", "hw": "w", "lh": "l", "rh": "r",
                   "qu": "kw", "nc": "ŋk", "hy": "ç"}
UNITS = sorted({**DIPHTHONGS, **CONSONANT_UNITS}.items(), key=lambda kv: -len(kv[0]))
VOWELS = {"a": "a", "e": "e", "i": "i", "o": "o", "u": "u", "y": "y"}
CONSONANTS = {"c": "k", "g": "ɡ", "x": "ks"}
TWO_CONSONANTS = {"ks", "ŋɡ", "ŋk", "xt", "kw", "ŋkw"}
KOKORO_VOWELS = set("aeiouyAIOWYɑæəɛɪʊʌɔɜᵊᵻ")
# Unstressed little words inside names, written lowercase: "Gwaith-i-Mírdain", "Taur-im-Duinath" (but "Nan Elmoth").
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


def segment(word, sindarin):
    """(phonemes, kind) pieces: 'V' short vowel, 'L' long vowel or diphthong, 'C' consonant."""
    w = word.lower()
    segs, i = [], 0
    while i < len(w):
        unit = next(((src, dst) for src, dst in UNITS if w.startswith(src, i)), None)
        # w before a vowel is a consonant: "Tawar" is ta-war, not tau-ar.
        if unit and unit[0] == "aw" and i + 2 < len(w) and base(w[i + 2]) in VOWELS:
            unit = None
        # lh and rh are one sound only starting a word; inside one they are two ("Edhel-harn", "Dír-hael").
        if unit and unit[0] in ("lh", "rh") and i > 0:
            unit = None
        if unit:
            segs.append((unit[1], "L" if unit[0] in DIPHTHONGS else "C"))
            i += len(unit[0])
            continue
        ch, b = w[i], base(w[i])
        if b == "y" and i + 1 < len(w) and base(w[i + 1]) in "aeiou":
            segs.append(("j", "C"))
        elif sindarin and i == 0 and b == "i" and len(w) > 1 and base(w[1]) in "aeiou":
            segs.append(("j", "C"))  # Sindarin initial i before a vowel: "Ioreth", "Iarwain"
        elif sindarin and i == len(w) - 1 and b == "f" and w[i - 1:i] != "f":
            segs.append(("v", "C"))  # Sindarin final f is v: "Nindalf"
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
        following += (p != "j") + (p in TWO_CONSONANTS)  # ny, ly, ry are one consonant; x, ŋɡ, qu two
    return following >= 2


def stressed(part, mark, sindarin):
    segs = segment(part, sindarin)
    nuclei = [k for k, (_, kind) in enumerate(segs) if kind != "C"]
    if part in PARTICLES or not nuclei:
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


def pronounce(name, sindarin=False):
    words = []
    for word in name.split():
        parts = word.split("-")
        main = max((i for i, p in enumerate(parts) if p not in PARTICLES), default=len(parts) - 1)
        words.append("".join(stressed(p, "ˈ" if i == main else "ˌ", sindarin) for i, p in enumerate(parts)))
    return relocate_stress(" ".join(words))


def names(cache: Path, english):
    """Every capitalized word Eldamo has in these languages (names, and proper nouns such as Maia and Eä), and
    the capitalized forms Tolkien wrote of them ("Valar", "Teleri"), with their pronunciations."""
    words = [w for w in ET.parse(fetch(cache)).getroot().iter("word") if w.get("l") in LANGUAGES]

    def forms_of(w):
        return [(re.sub("[¹²³]", "", v or "").strip(), w.get("l")) for v in [w.get("v")] + [r.get("v") for r in w.iter("ref")]]

    forms = [f for w in words for f in forms_of(w)]
    # Every single word Tolkien wrote in these languages, names or not. Eldamo also has a few English phrases
    # ("Praises of Cormallen"): a phrase with an English word that isn't also Elvish is left out ("Amon Hen" stays).
    elvish = {f.lower() for v, _ in forms if " " not in v for f in v.split("-")}
    out = {}
    for name, language in (f for w in words if w.get("v", "")[:1].isupper() for f in forms_of(w)):
        # Leave out reconstructed forms ("Argol(do)"), all-caps inscriptions and letters the rules don't cover.
        if (not name[:1].isupper() or (len(name) > 1 and name.isupper())
                or not all(c.lower() in LETTERS or c in " -" for c in name)
                or any(w.lower() in english and w.lower() not in elvish for w in name.replace("-", " ").split())):
            continue
        out.setdefault(name, pronounce(name, sindarin=language in SINDARIN))
    return out
