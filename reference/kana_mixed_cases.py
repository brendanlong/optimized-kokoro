"""Writes English corpus lines with kana runs next to Han runs, words and joiners, for dump.py: the
shapes where the port's "Japanese text" placeholders merge (runs of one language with only
spaces, apostrophes, hyphens, marks or format characters between them) and get a space before or
after them.

Usage (from reference/):
    python3 kana_mixed_cases.py SEED COUNT corpus/wiki-en.jsonl > /tmp/kana_mixed.jsonl
    .cache/kfa/.venv/bin/python -I dump.py /tmp/kana_mixed.jsonl out/kana-mixed-SEED.jsonl
"""

import json
import random
import re
import sys

seed, count, en_path = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
rng = random.Random(seed)
english = []
for line in open(en_path, encoding="utf-8"):
    if line.strip():
        english += [s.strip() for s in re.findall(r"[^.!?]+[.!?]", json.loads(line)["text"]) if 20 < len(s.strip()) < 160]
KANA = ["カナ", "ありがとう", "東京へ", "すし", "ー", "漢字かな", "トランス", "の"]
HAN = ["中文", "北京", "銀行", "兙", "漢字", "日本"]
GAPS = [" ", "'", "’", "-", " - ", "​", "‍", "́", "", "  ", " '", "' ", "、", "，", "。"]
AROUND = ["", "", " ", "(", ")", "“", "”", "'", "s", "is", "-like", ",", "."]

for i in range(count):
    words = rng.choice(english).split(" ")
    for _ in range(rng.randrange(1, 3)):
        piece = rng.choice(KANA)
        for _ in range(rng.randrange(0, 3)):
            piece += rng.choice(GAPS) + rng.choice(KANA + HAN)
        piece = rng.choice(AROUND) + piece + rng.choice(AROUND)
        j = rng.randrange(len(words) + 1)
        if rng.random() < 0.3 and j < len(words):
            words[j] = words[j] + piece
        else:
            words.insert(j, piece)
    print(json.dumps({"id": f"kana-mixed-{seed}-{i:05d}", "lang": rng.choice("ab"), "text": " ".join(words)}, ensure_ascii=False))
