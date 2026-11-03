"""Writes corpus lines mixing Chinese with English, numbers and punctuation, for dump.py: English
records (lang a and b) with Han runs, which kokoro reads with misaki's Chinese G2P and makes
English (tones as stress, unaspirated stops voiced), and Chinese records (lang z) with Latin
words, digits, CJK and ASCII punctuation, kana and other scripts.

Usage (from reference/):
    python3 zh_mixed_cases.py SEED COUNT corpus/wiki-en.jsonl corpus/wiki-zh.jsonl > /tmp/zh_mixed.jsonl
    .cache/kfa/.venv/bin/python -I dump.py /tmp/zh_mixed.jsonl out/zh-mixed-SEED.jsonl
"""

import json
import random
import re
import sys

seed, count, en_path, zh_path = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3], sys.argv[4]
rng = random.Random(seed)


def sentences(path, pattern):
    out = []
    for line in open(path, encoding="utf-8"):
        if line.strip():
            out.extend(s.strip() for s in re.findall(pattern, json.loads(line)["text"]) if s.strip())
    return out


english = [s for s in sentences(en_path, r"[^.!?]+[.!?]") if 20 < len(s) < 200]
chinese = [s for s in sentences(zh_path, r"[^。！？]+[。！？]?") if len(s) < 120]
words = [w for s in chinese for w in re.findall(r"[一-鿿]{1,6}", s)]
POLYPHONES = ["银行行长", "长城", "行不行", "重庆", "音乐", "觉得", "了解", "差不多", "一个", "不对", "嗯", "呣", "兙", "〇", "㐀", "ー"]
LATIN = ["Beijing", "COVID-19", "iPhone", "Mr. Wang", "AI", "OK", "Python 3.12", "Xi Jinping", "NBA", "e-mail"]
OTHER = ["東京へ行く", "カタカナ", "Привет", "Καλημέρα", "안녕", "สวัสดี", "αβ"]
NUMBERS = ["2024年3月5日", "12.5%", "3/4", "30℃", "-7", "1,184", "１２３", "100000001", "12345678901234567", "3.14159", "第1名"]
PUNCT = ["，", "。", "、", "：", "；", "！", "？", "“", "”", "「", "」", "《", "》", "（", "）", "…", "——", " ", ", ", ". "]


def zh_piece():
    return rng.choice([rng.choice(words), rng.choice(POLYPHONES), rng.choice(chinese)[: rng.randrange(2, 30)]])


for i in range(count):
    if rng.random() < 0.5:
        lang = rng.choice("ab")
        s = rng.choice(english).split(" ")
        for _ in range(rng.randrange(1, 4)):
            j = rng.randrange(len(s) + 1)
            piece = zh_piece()
            if rng.random() < 0.3:
                piece += rng.choice(PUNCT) + zh_piece()
            if rng.random() < 0.2:
                piece = rng.choice(["(", "“", "'"]) + piece + rng.choice([")", "”", "'"])
            s.insert(j, piece if rng.random() < 0.8 else piece + rng.choice(["is", "s", "-like"]))
        text = " ".join(s)
    else:
        lang = "z"
        parts = []
        for _ in range(rng.randrange(1, 7)):
            parts.append(rng.choice([zh_piece(), zh_piece(), rng.choice(LATIN), rng.choice(NUMBERS), rng.choice(PUNCT), rng.choice(OTHER) if rng.random() < 0.3 else zh_piece()]))
        text = "".join(parts)
        if rng.random() < 0.3:
            text = rng.choice(chinese) + text
    print(json.dumps({"id": f"zh-mixed-{seed}-{i:05d}", "lang": lang, "text": text}, ensure_ascii=False))
