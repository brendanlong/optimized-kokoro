"""Writes tests/data/subtokenize.jsonl: misaki's subtokenize over random strings of the characters its
regex distinguishes. Run with the reference venv: reference/.cache/kfa/.venv/bin/python -I tools/gen_subtokenize.py
"""

import json
import os
import random
import sys

from misaki.en import subtokenize

ALPHABET = "aZéÉǅ'‘’-_,.1٣x中\n !"
FIXED = ["iPhone", "HTMLParser", "McDonald's", "don't", "'tis", "-1,234.5", "1'2", "rock''n", "dogs'", "x2y",
         "a'b'c", "ABCdef", "aB'cD", "1.2.3", ".5", "-.5", "1,,2", "'''", "'", "'\n", "a'\n", "--", "__a"]


def main(dst):
    random.seed(7)
    cases = FIXED + ["".join(random.choice(ALPHABET) for _ in range(random.randint(1, 8))) for _ in range(8000)]
    with open(dst, "w") as f:
        for c in dict.fromkeys(cases):
            f.write(json.dumps([c, subtokenize(c)], ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "tests", "data", "subtokenize.jsonl"))
