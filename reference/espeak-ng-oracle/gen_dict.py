"""Texts built from each language's own dictionary source: gen_dict.py SEED N VOICE...

Words come from espeak-ng's dictsource/<lang>_list (single and multi-word entries) and
<lang>_rules letter groups, mixed with numbers, capitals and punctuation.
"""
import glob
import json
import os
import random
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
DATA = os.environ.get("ESPEAK_DATA", os.path.join(ROOT, "reference", ".cache", "espeak-ng-data"))
OUT = os.environ.get("ORACLE_OUT", os.path.join(ROOT, "scratch", "out"))
DICTSOURCE = os.path.join(HERE, "espeak-ng-1.52.0", "dictsource")
LANG_DIR = os.path.join(DATA, "lang")

seed, n = int(sys.argv[1]), int(sys.argv[2])
voices = sys.argv[3:]
rng = random.Random(seed)


def voice_dict(voice):
    path = os.path.join(LANG_DIR, voice)
    lang = os.path.basename(voice).split("-")[0].lower()
    dictionary = lang
    if os.path.exists(path):
        for line in open(path, encoding="utf-8", errors="replace"):
            parts = line.split()
            if len(parts) >= 2 and parts[0] == "language" and dictionary == lang and parts[1] != "variant":
                dictionary = parts[1].split("-")[0]
            if len(parts) >= 2 and parts[0] == "dictionary":
                dictionary = parts[1]
    return dictionary


_cache = {}


def words_for(dictionary):
    if dictionary in _cache:
        return _cache[dictionary]
    words = []
    for path in glob.glob(os.path.join(DICTSOURCE, f"{dictionary}_list")) + glob.glob(os.path.join(DICTSOURCE, f"{dictionary}_extra")):
        for line in open(path, encoding="utf-8", errors="replace"):
            line = line.split("//")[0].strip()
            if not line or line.startswith("?") or line.startswith("$") or line.startswith("."):
                continue
            m = re.match(r"\(([^)]*)\)", line)
            if m:
                words.append(m.group(1))
                continue
            w = line.split()[0]
            if w.startswith("_"):
                continue
            words.append(w)
    rules = os.path.join(DICTSOURCE, f"{dictionary}_rules")
    letters = set()
    if os.path.exists(rules):
        for line in open(rules, encoding="utf-8", errors="replace"):
            m = re.match(r"\.group\s+(\S+)", line)
            if m:
                letters.add(m.group(1))
    _cache[dictionary] = (words or ["a"], sorted(letters) or list("abc"))
    return _cache[dictionary]


NUMBERS = ["0", "1", "7", "12", "21", "100", "101", "1000", "1984", "2006", "12345", "1.5", "3,14", "1 000", "1.000", "10.000", "1st", "2.", "50%", "07:30", "XIV", "III"]
PUNCT = [".", ",", ";", ":", "!", "?", "-", "—", "...", "\"", "'", "(", ")", "«", "»"]


def text(voice):
    words, letters = words_for(voice_dict(voice))
    parts = []
    for _ in range(rng.randint(1, 25)):
        r = rng.random()
        if r < 0.7:
            w = rng.choice(words)
            if rng.random() < 0.15:
                w = w.capitalize()
            elif rng.random() < 0.05:
                w = w.upper()
        elif r < 0.8:
            w = "".join(rng.choice(letters) for _ in range(rng.randint(2, 7)))
        elif r < 0.9:
            w = rng.choice(NUMBERS)
        else:
            w = rng.choice(PUNCT)
        parts.append(w)
        if rng.random() < 0.1:
            parts.append(rng.choice(PUNCT[:5]))
    return " ".join(parts)


for _ in range(n):
    v = rng.choice(voices)
    print(json.dumps({"voice": v, "text": text(v)}, ensure_ascii=False))
