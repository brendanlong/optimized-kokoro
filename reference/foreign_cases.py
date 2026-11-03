"""Records kokoro's ForeignScripts.split (English pipeline) on random mixed-script strings, for
crates/pipeline's segmentation test. Each line: {"text", "segments": [[segment, language]],
"unreadable": [Han characters pypinyin has no reading for]}.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I foreign_cases.py 3000 ../crates/pipeline/tests/foreign_cases.jsonl
"""

import json
import os
import random
import sys

from phonemizer.backend.espeak.wrapper import EspeakWrapper

import misaki.espeak  # noqa: F401  (points espeak at a path too long for it; see dump.py)

EspeakWrapper.set_data_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".cache", "espeak-ng-data"))

from kokoro.foreign import ForeignScripts  # noqa: E402
from pypinyin import lazy_pinyin  # noqa: E402

PIECES = (
    list("abcXYZ  '’-.,()[]/#1")
    + ["the ", "Ab", "[x](/y/)", "[東京](/tˈOkjO/)", "](", "[", "]("]
    + list("αβγΔλόγος") + ["Καλημέρα", "ΔΔ"]
    + list("Привет") + ["Москва"]
    + list("مرحبا") + ["علي"]
    + list("שלום") + ["ָ", "ּ", "שָׁלוֹם"]
    + list("नमस्ते") + ["्", "ं"]
    + list("안녕하세요")
    + list("我你东京北兙⺀") + ["明天见"]
    + list("ひらカタ") + ["ー", "、", "・"]
    + ["́", "҃", "͂", "‍", "　"]
)


def main(n, dst):
    rng = random.Random(1)
    foreign = ForeignScripts("a")
    with open(dst, "w") as out:
        for _ in range(n):
            text = "".join(rng.choice(PIECES) for _ in range(rng.randint(1, 14)))
            segments = foreign.split(text)
            unreadable = sorted({c for c in text if lazy_pinyin(c)[0] == c})
            out.write(json.dumps({"text": text, "segments": segments, "unreadable": unreadable}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(int(sys.argv[1]), sys.argv[2])
