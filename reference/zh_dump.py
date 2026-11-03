"""Calls misaki's legacy Chinese G2P (ZHG2P(version=None)) directly on generated and corpus text,
for crates/g2p-zh's parity test.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I zh_dump.py SEED COUNT corpus/wiki-zh.jsonl > out/zh-g2p/seed-SEED.jsonl
    .cache/kfa/.venv/bin/python -I zh_dump.py --fixture > ../crates/g2p-zh/tests/cases.jsonl

Output lines: {"text", "phonemes"} or {"text", "error"} where misaki raises. Cases: corpus
paragraphs and their sentences; numbers (dates, fractions, percentages, degrees, decimals, signs,
full-width and other decimal digits, more than 16 digits); Chinese mixed with English, digits and
punctuation; polyphones; random strings of readable and unreadable characters (U+9FD6-U+9FFF
included, which jieba leaves out of its Han blocks); and runs of dictionary words and phrases,
which push jieba's DAG and HMM and pypinyin's mmseg into their corner cases.
"""

import json
import random
import re
import sys
import warnings

warnings.simplefilter("ignore")
import jieba  # noqa: E402
from loguru import logger  # noqa: E402

logger.remove()
jieba.setLogLevel(60)
from misaki.zh import ZHG2P  # noqa: E402
from pypinyin.constants import PHRASES_DICT, PINYIN_DICT  # noqa: E402

G2P = ZHG2P()

POLYPHONES = [
    "银行行长", "行长很长", "长城很长", "他在银行工作", "行不行", "重新重要", "音乐快乐", "还还钱", "得到得了",
    "了解了", "觉得睡觉", "为了为人", "调查调整", "朝阳朝代", "传记传说", "便宜方便", "差不多出差", "地方的地",
    "好好学习", "都是都城", "着急看着", "藏族宝藏", "数学数一数", "长大成长", "教书教育", "处理到处",
    "一个", "一样", "不对", "不是", "一一", "了不起", "我们走吧", "曾经曾子", "单于单位", "仇人仇姓",
    "解放解元", "会计开会", "种子种树", "降落投降", "空气空闲", "参加人参", "好奇爱好", "什么", "这么样",
]
PUNCTUATION = list("、，。．！：；？«»《》「」【】（）“”‘’…—·,.!?;:()[]- ")
DIGITS = "0123456789"
FULLWIDTH = "０１２３４５６７８９"
OTHER_DIGITS = "٠١٢٣٤٥٦٧٨٩०१२३४५६७८९"


def number(rng):
    n = rng.choice([1, 1, 2, 3, 4, 4, 5, 8, 12, 16, 17, 20])
    digits = "".join(rng.choice(DIGITS) for _ in range(n))
    if rng.random() < 0.1:
        digits = "".join(rng.choice(FULLWIDTH) for _ in range(n))
    if rng.random() < 0.05:
        digits = "".join(rng.choice(OTHER_DIGITS) for _ in range(n))
    form = rng.randrange(10)
    if form == 0:
        return digits + "年" + (str(rng.randrange(0, 15)) + "月" if rng.random() < 0.6 else "") + (
            str(rng.randrange(0, 40)) + "日" if rng.random() < 0.5 else "")
    if form == 1:
        return f"{digits}/{rng.randrange(1, 1000)}"
    if form == 2:
        return ("-" if rng.random() < 0.3 else "") + digits + ("." + str(rng.randrange(100)) if rng.random() < 0.5 else "") + "%"
    if form == 3:
        return digits + "℃"
    if form == 4:
        return ("-" if rng.random() < 0.3 else "") + digits + "." + "".join(rng.choice(DIGITS) for _ in range(rng.choice([1, 2, 5, 17, 20])))
    if form == 5:
        return str(rng.randrange(0, 13)) + "月" + str(rng.randrange(0, 32)) + "日"
    if form == 6:
        return "0" * rng.randrange(1, 4) + digits
    if form == 7:
        return digits + rng.choice(["..", ".", "-", "--", "/", "%%"]) + digits
    return digits


def han(rng, readable):
    while True:
        c = chr(rng.randrange(0x4E00, 0xA000) if rng.random() < 0.9 else rng.randrange(0x9FD0, 0xA000))
        if (ord(c) in PINYIN_DICT) == readable:
            return c


def generated(rng, words, phrases, sentences):
    kind = rng.randrange(9)
    if kind == 0:
        return number(rng)
    if kind == 1:
        parts = []
        for _ in range(rng.randrange(1, 6)):
            r = rng.randrange(5)
            parts.append([rng.choice(sentences)[: rng.randrange(1, 30)], number(rng), rng.choice(["hello", "AI", "COVID-19", "iPhone 15", "Mr. Li", "abc"]),
                          rng.choice(PUNCTUATION), rng.choice(POLYPHONES)][r])
        return "".join(parts)
    if kind == 2:
        return "".join(rng.choice(POLYPHONES) for _ in range(rng.randrange(1, 4)))
    if kind == 3:
        return "".join(han(rng, rng.random() < 0.97) for _ in range(rng.randrange(1, 12)))
    if kind == 4:
        return "".join(rng.choice(words) for _ in range(rng.randrange(1, 8)))
    if kind == 5:
        return "".join(rng.choice(phrases) for _ in range(rng.randrange(1, 6)))
    if kind == 6:
        # Prefixes and pieces of phrases: mmseg's prefix-only remainders.
        out = []
        for _ in range(rng.randrange(1, 5)):
            p = rng.choice(phrases)
            out.append(p[: rng.randrange(1, len(p) + 1)] if rng.random() < 0.5 else p[rng.randrange(len(p)):])
        return "".join(out)
    if kind == 7:
        s = rng.choice(sentences)
        i = rng.randrange(len(s))
        return s[i : i + rng.randrange(1, 60)]
    # Unusual characters among Chinese ones.
    pool = ["〇", "㐀", "𠀀", "々", "ー", "　", "̯", " ", "\t", "Ａ", "ａ", "Ω", "й", "ä", "😀"]
    return "".join(rng.choice(pool) if rng.random() < 0.3 else han(rng, True) for _ in range(rng.randrange(1, 10)))


def call(text):
    try:
        return {"text": text, "phonemes": G2P(text)[0]}
    except Exception as e:
        return {"text": text, "error": f"{type(e).__name__}: {e}"}


def corpus(path):
    paragraphs = [json.loads(line)["text"] for line in open(path, encoding="utf-8") if line.strip()]
    sentences = [s for p in paragraphs for s in re.findall(r"[^。！？]+[。！？]?", p) if s.strip()]
    return paragraphs, sentences


def main():
    words = [line.split(" ")[0] for line in open(jieba.get_dict_file().name, encoding="utf-8")]
    phrases = list(PHRASES_DICT)
    if sys.argv[1] == "--fixture":
        rng = random.Random(0)
        paragraphs, sentences = corpus("corpus/wiki-zh.jsonl")
        cases = POLYPHONES + rng.sample(sentences, 400) + [generated(rng, words, phrases, sentences) for _ in range(2600)]
    else:
        seed, count, path = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
        rng = random.Random(seed)
        paragraphs, sentences = corpus(path)
        cases = (paragraphs + sentences if seed == 1 else []) + [generated(rng, words, phrases, sentences) for _ in range(count)]
    seen = set()
    for text in cases:
        if text not in seen:
            seen.add(text)
            print(json.dumps(call(text), ensure_ascii=False))


if __name__ == "__main__":
    main()
