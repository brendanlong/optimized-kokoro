"""Writes tests/data/num2words.tsv: num2words 0.5.14's English output for the calls misaki makes.

Run with the reference venv: reference/.cache/kfa/.venv/bin/python -I tools/gen_num2words.py
Lines are `kind<TAB>input<TAB>output`, with output `ERR` where num2words raises.
"""

import os
import random
import sys

from num2words import num2words

random.seed(20261102)


def digits(n, zero_heavy=False):
    alphabet = "0000000001" if zero_heavy else "0123456789"
    s = random.choice("123456789") + "".join(random.choice(alphabet) for _ in range(n - 1))
    return s


def ints():
    out = [str(i) for i in range(0, 2101)]
    for k in range(1, 40):
        out += [str(10**k - 1), str(10**k), str(10**k + 1), str(10**k + 99), str(10**k + 100)]
    for n in range(1, 41):
        out += [digits(n) for _ in range(10)] + [digits(n, True) for _ in range(10)]
    return out + ["0" + digits(5), "000"]


def big_ints():
    out = []
    for k in [*range(40, 300, 13), *range(300, 312)]:
        out += [str(10**k - 1), str(10**k), str(10**k + 1)]
    return out + [digits(n, True) for n in range(41, 312, 9)]


def floats():
    out = ["0.5", "1.5", "1.10", "100.001", "0.07", "1.0", "5.", "3.14159265358979", "0.30000000000000004",
           "9999999999999999.9", "123456789012345678.5", "1" + "0" * 305 + ".5", "1" + "0" * 306 + ".5",
           "0." + "0" * 307 + "1", "0." + "0" * 308 + "1", "0." + "0" * 400 + "1", "9" * 400 + ".5"]
    for a in range(0, 26):
        for b in range(0, 26):
            for _ in range(4):
                left = "".join(random.choice("0123456789") for _ in range(a))
                right = "".join(random.choice("0123456789") for _ in range(b))
                if left or right:
                    out.append(left + "." + right)
    for _ in range(150):
        out.append(digits(random.randint(17, 40)) + "." + digits(random.randint(1, 3)))
        out.append("0." + "0" * random.randint(5, 40) + digits(random.randint(1, 5)))
    return out


def run(kind, value, fn):
    try:
        return f"{kind}\t{value}\t{fn()}"
    except (OverflowError, ValueError):
        return f"{kind}\t{value}\tERR"


def main(dst):
    lines = []
    for s in ints():
        lines.append(run("cardinal", s, lambda: num2words(int(s))))
        lines.append(run("ordinal", s, lambda: num2words(int(s), to="ordinal")))
    for s in big_ints():
        lines.append(run("cardinal", s, lambda: num2words(int(s))))
    for y in range(10000):
        lines.append(run("year", y, lambda: num2words(y, to="year")))
    for s in floats():
        lines.append(run("float", s, lambda: num2words(float(s))))
    with open(dst, "w") as f:
        f.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "tests", "data", "num2words.tsv"))
