"""Writes corpus lines for the espeak pipelines (lang e, f, h, i and p) mixing each language's
Wikipedia sentences with other scripts, for dump.py: English words (Latin runs, which the Hindi
pipeline reads with misaki's English G2P), Devanagari (Hindi's own script, which the others read
with espeak's hi), Han (with and without a reading), kana, the six other scripts foreign.py splits
out (short Greek included), the scripts it doesn't, CJK and ASCII punctuation, parentheses and
angle quotes (which EspeakG2P swaps), numbers, joiners and invisible characters, [word](/phonemes/)
links (dump.py takes no pause tags), in sentences long enough to be cut at clauses.

Usage (from reference/):
    python3 espeak_mixed_cases.py SEED COUNT corpus/wiki-en.jsonl corpus/wiki-{es,fr,hi,it,pt}.jsonl > /tmp/espeak_mixed.jsonl
    .cache/kfa/.venv/bin/python -I dump.py /tmp/espeak_mixed.jsonl out/mixed-espeak-SEED.jsonl

(Not out/espeak*.jsonl, which crates/espeak reads as espeak_dump.py output.) Seed 1, 100 lines is
crates/pipeline/tests/espeak_mixed.jsonl.
"""

import json
import random
import re
import sys

seed, count, en_path, paths = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4:]
rng = random.Random(seed)
LANGS = {"es": "e", "fr": "f", "hi": "h", "it": "i", "pt": "p"}


def sentences(path):
    out = []
    for line in open(path, encoding="utf-8"):
        if line.strip():
            out += [s.strip() for s in re.findall(r"[^.!?।]+[.!?।]?", json.loads(line)["text"]) if 10 < len(s.strip()) < 300]
    return out


english = sentences(en_path)
native = {}
for path in paths:
    code = re.search(r"wiki-(\w+)\.jsonl$", path).group(1)
    native[LANGS[code]] = sentences(path)

LATIN = ["iPhone", "COVID-19", "New York", "Mr. Smith", "AI", "OK", "e-mail", "don't", "Wi-Fi", "the United Kingdom", "Python 3.12", "rock 'n' roll"]
DEVANAGARI = ["नमस्ते", "भारत", "हिन्दी भाषा", "१२३", "संस्कृत", "क़िला", "ॐ"]
HAN = ["北京", "银行行长", "中文", "兙", "佐々木", "長城", "〇", "㐀"]
KANA = ["東京へ", "カタカナ", "ありがとう", "ー", "漢字かな", "すし"]
CYRILLIC = ["Москва", "Привет мир", "Лев Толстой"]
GREEK = ["Καλημέρα", "αβ", "Δ", "λόγος", "ΔΔCt", "Ελλάδα"]
ARABIC = ["مرحبا", "اللغة العربية", "ḥ"]
HEBREW = ["שָׁלוֹם", "עברית"]
HANGUL = ["안녕하세요", "서울"]
OTHER = ["สวัสดี", "გამარჯობა", "Բարեւ", "বাংলা", "தமிழ்", "ሰላም"]
NUMBERS = ["1.234,5", "3,14", "12%", "1990", "2024-03-05", "10 000", "3/4", "-7", "XIV", "1º", "n.º 5", "100.000.000"]
PUNCT = ["。", "，", "、", "：", "；", "！", "？", "「", "」", "（", "）", "　", "・", "(", ")", "«", "»", "“", "”", "—", "…", ";", ":", ",", " - "]
JOINERS = [" ", "'", "’", "-", "​", "́", "‍", "­", ""]
EXTRA = ["[hola](/ˈola/)", "[Kokoro](/kˈOkəɹO/)", "\n", "\n\n", "  "]


def foreign(lang):
    groups = [LATIN, HAN, KANA, CYRILLIC, GREEK, ARABIC, HEBREW, HANGUL, NUMBERS]
    groups += [LATIN, LATIN] if lang == "h" else [DEVANAGARI, DEVANAGARI]
    if rng.random() < 0.08:
        groups = [OTHER]
    piece = rng.choice(rng.choice(groups))
    if rng.random() < 0.3:
        piece += rng.choice(JOINERS) + rng.choice(rng.choice(groups))
    if rng.random() < 0.3:
        piece = rng.choice(["(", "«", "“", "'", "「"]) + piece + rng.choice([")", "»", "”", "'", "」"])
    return piece


for i in range(count):
    lang = "efhip"[i % 5]
    words = rng.choice(native[lang]).split(" ")
    for _ in range(rng.randrange(1, 5)):
        r = rng.random()
        piece = foreign(lang) if r < 0.6 else rng.choice(PUNCT) if r < 0.85 else rng.choice(EXTRA)
        j = rng.randrange(len(words) + 1)
        if rng.random() < 0.25 and j < len(words):
            words[j] += piece
        else:
            words.insert(j, piece)
    text = " ".join(words)
    if rng.random() < 0.2:
        text += " " + rng.choice(english)
    if rng.random() < 0.15:
        # Long enough to be cut at clauses.
        text = ", ".join(rng.choice(native[lang]).rstrip(".!?।") for _ in range(rng.randrange(4, 9))) + ". " + text
    print(json.dumps({"id": f"espeak-mixed-{seed}-{i:05d}", "lang": lang, "text": text}, ensure_ascii=False))
