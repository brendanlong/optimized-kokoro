"""How close real.py's readings of a language's names come to Wiktionary's IPA for them: a measurement only
(Wiktionary is CC BY-SA, so nothing of it goes into names.json).

    python3 tools/names/evaluate.py LANGUAGE [--cache DIR] [--show N]

Wiktionary's proper nouns come from kaikki.org's extract (downloaded into the cache once). Both readings are
cut into sounds (ties joined, marks dropped), compared without length, syllable breaks, secondary stress or
nasality, a long consonant as one, and with the sounds COARSE merges taken as one, since dictionaries write some
sounds differently (Polish sz as ʂ or ʃ, Czech e as ɛ or e). It finds where espeak is badly wrong, not fine detail. Reported: names whose sounds all match, the sound error rate
(edits over Wiktionary's sounds), and names whose main stress falls on the same vowel.
"""

import argparse
import collections
import json
import re
import unicodedata
import urllib.parse
import urllib.request
from pathlib import Path

import real
import sounds

KAIKKI = {"nb": "Norwegian Bokmål", "sl": "Slovene", "hr": "Serbo-Croatian", "bs": "Serbo-Croatian"}
VOWELS = set("aeiouyɑɐɒæɛɜəɘɪɨʉʊʌɔøœɤɯɵʏ")
COARSE = {"ʂ": "ʃ", "ʐ": "ʒ", "ʑ": "ʒ", "ɫ": "l", "ɭ": "l", "g": "ɡ", "ɾ": "r", "ʁ": "r", "ɹ": "r", "ʋ": "v",
          "ɦ": "h", "ɘ": "ɨ", "ʲ": "j", "ʏ": "y", "ʉ": "y", "ɵ": "ø",
          # Vowel qualities dictionaries write differently for one sound (Czech e as ɛ, Hungarian a as ɒ).
          "ɛ": "e", "ε": "e", "ɪ": "i", "ɔ": "o", "ɒ": "a", "ɑ": "a", "ɐ": "a", "ʊ": "u"}
# Nasal vowels are compared without their nasality (Wiktionary writes Polish ę as ɛw̃, espeak as ɛ̃).
NASAL = ("w\u0303", "j\u0303")
IGNORED = set("ː.ˌ‿ ˥˦˧˨˩ˀ")


def kaikki(cache, language):
    name = KAIKKI.get(language, real.LANGUAGE_NAMES[language])
    path = cache / "kaikki" / f"{name}-name.jsonl"
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        q = urllib.parse.quote(name)
        url = f"https://kaikki.org/dictionary/{q}/pos-name/kaikki.org-dictionary-{q.replace('%20', '')}-by-pos-name.jsonl"
        path.write_bytes(urllib.request.urlopen(url).read())
    out = collections.defaultdict(set)
    for line in open(path, encoding="utf-8"):
        d = json.loads(line)
        for s in d.get("sounds", []):
            if s.get("ipa"):
                for ipa in re.findall(r"/([^/]+)/|\[([^\]]+)\]", s["ipa"]):
                    out[unicodedata.normalize("NFC", d["word"])].add(ipa[0] or ipa[1])
    return out


def segments(ipa):
    """(sounds, index of the stressed sound or None)."""
    s = unicodedata.normalize("NFD", ipa.replace("ˈˈ", "ˈ"))
    s = s.replace("\u035c", "").replace("\u0361", "").replace("r\u031d\u030a", "rʃ").replace("r\u031d", "rʒ")
    for n in NASAL:
        s = s.replace(unicodedata.normalize("NFD", n), "")
    out, stress, pending = [], None, False
    for c in s:
        if c in IGNORED or unicodedata.category(c) == "Mn" or c == "ʰ":
            continue
        if c == "ˈ":
            pending = True
        else:
            out.append(c)
            if pending and c in VOWELS:
                stress, pending = len(out) - 1, False
    out = [(COARSE.get(x, x), k == stress) for k, x in enumerate(out)]
    # Long consonants written twice or with ː, and the glide between i and a vowel, as one.
    merged = []
    for k, (x, stressed) in enumerate(out):
        if merged and x == merged[-1][0] and x[0] not in VOWELS:
            continue
        if x == "j" and merged and merged[-1][0] == "i" and k + 1 < len(out) and out[k + 1][0][0] in VOWELS:
            continue
        merged.append((x, stressed))
    return [x for x, _ in merged], next((k for k, (_, st) in enumerate(merged) if st), None)


def vowel_index(segs, i):
    return None if i is None else sum(1 for s in segs[:i] if s[0] in VOWELS)


def distance(a, b):
    row = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        prev, row[0] = row[0], i
        for j, y in enumerate(b, 1):
            prev, row[j] = row[j], min(row[j] + 1, row[j - 1] + 1, prev + (x != y))
    return row[-1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("language")
    ap.add_argument("--cache", type=Path, default=Path.home() / ".cache/kokoro-names")
    ap.add_argument("--show", type=int, default=20)
    args = ap.parse_args()
    found = real.readings({args.language})
    wikt = kaikki(args.cache, args.language)
    rows = []
    for spelling, (_, voice, raw) in found.items():
        if spelling not in wikt:
            continue
        ours, our_stress = segments(sounds.kokoro(raw))
        best = None
        for ref_ipa in wikt[spelling]:
            ref, ref_stress = segments(ref_ipa)
            d = distance(ours, ref)
            stress_ok = vowel_index(ours, our_stress) == vowel_index(ref, ref_stress) if ref_stress is not None else None
            if best is None or (d, not stress_ok) < (best[0], not best[1]):
                best = (d, stress_ok, len(ref), ref_ipa)
        rows.append((spelling, sounds.kokoro(raw), *best))
    if not rows:
        raise SystemExit(f"no {args.language} names in Wiktionary")
    exact = sum(r[2] == 0 for r in rows)
    stressed = [r for r in rows if r[3] is not None]
    print(f"{args.language}: {len(rows)} of {len(found)} names are in Wiktionary; "
          f"{exact / len(rows):.0%} all sounds match, sound error rate {sum(r[2] for r in rows) / sum(r[4] for r in rows):.1%}, "
          f"stress on the same vowel {sum(r[3] for r in stressed) / max(len(stressed), 1):.0%} (of {len(stressed)} with stress)")
    for spelling, ours, d, stress_ok, _, ref in sorted(rows, key=lambda r: -r[2])[:args.show]:
        print(f"  {spelling}: ours {ours}  Wiktionary {ref}  ({d} edits{', stress differs' if stress_ok is False else ''})")


if __name__ == "__main__":
    main()
