"""Writes crates/g2p-zh/data/: the dictionaries misaki's legacy Chinese G2P (ZHG2P(version=None))
reads, from the reference venv's jieba 0.42.1 and pypinyin 0.55.0, in the shapes the Rust port uses.

    jieba.txt      line 1: jieba's `total` (the sum of every dict.txt frequency, duplicates
                   included); then `word freq` for each word of dict.txt with a non-zero frequency
                   (the last one when a word repeats) made only of the characters jieba's DAG ever
                   sees in misaki's input (U+4E00-U+9FD5). Prefixes are added at load, as
                   `gen_pfdict` does.
    hmm.txt        jieba's finalseg emission log-probabilities: `char B M E S`, `-` where absent.
    chars.txt      `char pinyin` for every character lazy_pinyin reads (RE_HANS and in
                   pinyin_dict): the first reading, which lazy_pinyin uses.
    phrases.txt    `phrase py1 py2 ...` for every phrase of phrases_dict: each character's first
                   reading. Its keys are also mmseg's prefix set.
    syllables.txt  `pinyin ipa` for every reading above: misaki's `ZHG2P.py2ipa` of pypinyin's
                   TONE3 (neutral tone as 5) form of it, or `!Error: message` where misaki raises.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I gen_zh_data.py ../crates/g2p-zh/data
"""

import os
import re
import sys

import jieba
import jieba.finalseg.prob_emit as prob_emit
import regex
from misaki.zh import ZHG2P
from pypinyin import Style, lazy_pinyin
from pypinyin.constants import PHRASES_DICT, PINYIN_DICT, RE_HANS
from pypinyin.converter import UltimateConverter

out = sys.argv[1]
os.makedirs(out, exist_ok=True)

JIEBA_HAN = re.compile("^[一-鿕]+$")
freq, total = {}, 0
with open(os.path.join(os.path.dirname(jieba.__file__), "dict.txt"), "rb") as f:
    for line in f:
        word, n = line.strip().decode("utf-8").split(" ")[:2]
        freq[word] = int(n)
        total += int(n)
with open(os.path.join(out, "jieba.txt"), "w", encoding="utf-8", newline="\n") as f:
    f.write(f"{total}\n")
    for word, n in freq.items():
        if n and JIEBA_HAN.match(word):
            f.write(f"{word} {n}\n")

emit = prob_emit.P
chars = sorted(set().union(*(set(v) for v in emit.values())))
with open(os.path.join(out, "hmm.txt"), "w", encoding="utf-8", newline="\n") as f:
    for c in chars:
        assert len(c) == 1 and not c.isspace()
        f.write(c + "".join(" " + (repr(emit[s][c]) if c in emit[s] else "-") for s in "BMES") + "\n")

readings = {}
for code, pys in sorted(PINYIN_DICT.items()):
    c = chr(code)
    if RE_HANS.match(c):
        readings[c] = pys.split(",")[0]
phrases = {}
for phrase, pys in PHRASES_DICT.items():
    assert len(pys) == len(phrase), phrase
    phrases[phrase] = [p[0] for p in pys]

syllables = sorted(set(readings.values()) | {p for pys in phrases.values() for p in pys})
converter = UltimateConverter(neutral_tone_with_five=True)
ipa = {}
for py in syllables:
    assert py and not any(c.isspace() for c in py), py
    tone3 = converter.convert_style("", py, Style.TONE3, True)
    try:
        ipa[py] = ZHG2P.py2ipa(tone3)
        assert ipa[py] and not ipa[py].startswith("!") and not any(c.isspace() for c in ipa[py])
    except Exception as e:
        ipa[py] = f"!{type(e).__name__}: {e}"

with open(os.path.join(out, "chars.txt"), "w", encoding="utf-8", newline="\n") as f:
    for c, py in readings.items():
        f.write(f"{c} {py}\n")
with open(os.path.join(out, "phrases.txt"), "w", encoding="utf-8", newline="\n") as f:
    for phrase, pys in phrases.items():
        f.write(phrase + "".join(" " + p for p in pys) + "\n")
with open(os.path.join(out, "syllables.txt"), "w", encoding="utf-8", newline="\n") as f:
    for py in syllables:
        f.write(f"{py} {ipa[py]}\n")

# Checks: the table gives what misaki does for every character, and `reads` (a reading in
# chars.txt) is exactly foreign.py's `lazy_pinyin(c)[0] != c` over every code point.
for c, py in readings.items():
    if "一" <= c <= "鿿":
        try:
            expected = ZHG2P.word2ipa(c)
        except Exception as e:
            expected = f"!{type(e).__name__}: {e}"
        assert expected == ipa[py], (c, py, expected, ipa[py])
for code in range(0x110000):
    c = chr(code)
    if 0xD800 <= code < 0xE000:
        continue
    reads = lazy_pinyin(c)[0] != c
    assert reads == (c in readings), (hex(code), reads)
han = regex.compile(r"\p{Han}")
print(
    f"jieba total {total}, {len(freq)} words; {len(chars)} HMM chars; {len(readings)} chars "
    f"({sum(1 for c in readings if han.match(c))} \\p{{Han}}); {len(phrases)} phrases; {len(syllables)} syllables, "
    f"{sum(1 for v in ipa.values() if v.startswith('!'))} that misaki can't read",
    file=sys.stderr,
)
