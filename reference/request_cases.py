"""Random English requests for crates/pipeline's request-level test: corpus words mixed with foreign
runs, phoneme links, punctuation, pause tags and odd spacing, long sentences of clauses, and
character-level fuzz. Written by the PR #9 review (Opus) and kept here.

Usage (from reference/, after fetch_corpus.py):
    python3 request_cases.py SEED N > cases.jsonl
"""

import json
import os
import random
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
seed, n = int(sys.argv[1]), int(sys.argv[2])
rng = random.Random(seed)
with open(os.path.join(HERE, "corpus", "wiki-en.jsonl")) as f:
    texts = [json.loads(line)["text"] for line in f]
words = [w for w in " ".join(texts).split() if not re.search(r"[぀-ヿ㐀-鿿豈-﫿\U00020000-\U0003ffff]", w)]
FOREIGN = ["Привет", "мир", "Москва-река", "д’Артаньян", "Καλημέρα", "κόσμε", "λόγος", "αβ", "ΔΔCt", "β-catenin", "π",
           "안녕하세요", "세계", "مرحبا", "بالعالم", "علي", "שָׁלוֹם", "שלום", "עולם", "नमस्ते", "दुनिया", "ट्रांसपोर्टरों",
           "Ελλ́ηνικά", "Б", "Ж-", "ё", "Ω", "ħ", "ʕ", "مُحَمَّد", "ﷺ", "٣", "۱۲۳", "१२३"]
LINKS = ["[Hello](/həlˈO/)", "[Привет](/pɹɪvjˈɛt/)", "[word](/ /)", "[x](/y/)", "[read](+1)", "[read](-1)", "[it](0.5)",
         "[x](", "[Καλημέρα](/kalimˈɛɾa/)", "[a](/ˈA/)", "[now](/ /)", "[q]( /a/)", "[](/a/)", "[Δ](/dˈɛltə/)"]
PUNCT = [",", ";", ":", "，", "、", "；", "：", "—", "...", "…", "!", "?", "¡", "¿", "«", "»", "\"", "'", "(", ")", "“", "”",
         "。", "-", "–", "/", "&", "%", "$5", "3.14", "1,234", "#1"]
SPACES = [" ", " ", " ", " ", "  ", " ", "\t", "\n", "\n\n", "　", " ", "・"]
CHARS = (list("abcxyzAEIOUthe ") + list("ПриветБжё") + list("αβγΔΩλόςΑΒ")
         + ["́", "҃", "ָ", "ּ", "ि", "्", "َ", "ّ", "̀"]
         + list(" '’-") * 3 + list("[]()/") * 2 + list(",.;:!?，。、；：！？（）「」") + list("0123456789")
         + list("안녕세") + list("مرحب") + list("שלוםעּ") + list("नमस्ते")
         + [" ", "\n", "\n\n", "\t", "　", "😀", "¡", "¿", "ー", "—", "…", "“", "”", "\"", "ħʕ", "ˈ", "[pause:1s]",
            "[x](/y/)", "[w](/ /)", "](/", "Mr.", "Dr.", "$", "%", "&", "@", "#", "*", "~", "ʃ", "ŋ", "ɪ", "İ"])


def eng(k):
    i = rng.randrange(len(words) - k)
    return " ".join(words[i:i + k])


def piece():
    r = rng.random()
    if r < 0.45:
        return eng(rng.randint(1, 6))
    if r < 0.7:
        return rng.choice(FOREIGN)
    if r < 0.8:
        return rng.choice(LINKS)
    if r < 0.95:
        return rng.choice(PUNCT)
    return f"[pause:{rng.choice(['0.5', '1', '0', '2.5', '30'])}s]"


def mixed():
    out = []
    for _ in range(rng.randint(1, 25)):
        out.append(piece())
        out.append(rng.choice(SPACES) if rng.random() < 0.85 else "")
    return "".join(out)


def long_sentence():
    parts = []
    for _ in range(rng.randint(8, 80)):
        c = eng(rng.randint(1, 12)).rstrip(".!?")
        if rng.random() < 0.3:
            c += " " + rng.choice(FOREIGN)
        if rng.random() < 0.1:
            c += " " + rng.choice(LINKS)
        parts.append(c)
        parts.append(rng.choice([", ", "; ", ": ", "，", "、", ", ", ","]))
    return "".join(parts[:-1]) + rng.choice([".", "!", "", "?"])


def short_sentences():
    return " ".join(eng(rng.randint(1, 4)).rstrip(".") + rng.choice([".", "!", "?", "。"]) for _ in range(rng.randint(2, 30)))


def alternating():
    a = rng.choice(["a", "the", "x", "I", "Hi", "1"])
    b = rng.choice(FOREIGN[:12])
    return rng.choice(["", " ", "-", ", "]).join([a, b] * rng.randint(2, 30))


def fuzz():
    return "".join(rng.choice(CHARS) for _ in range(rng.choice([3, 8, 20, 60, 200])))


gens = [mixed, mixed, long_sentence, short_sentences, alternating, lambda: eng(rng.randint(5, 120)), fuzz, fuzz, fuzz]
for i in range(n):
    print(json.dumps({"id": f"{seed}-{i}", "lang": rng.choice("ab"), "text": rng.choice(gens)()}, ensure_ascii=False))
