"""Writes `repr(float)` for random doubles: "<bits as hex> <repr>" per line.

Half are random bit patterns; half are values with a few fractional bits, where a shortest
round-trip string can tie between two digits and Python picks the even one.

Usage (from reference/): python3 gen_float_repr.py N fixtures/float_repr.txt
"""

import math
import random
import struct
import sys


def bits(x):
    return struct.unpack("<Q", struct.pack("<d", x))[0]


def main(n, dst):
    rng = random.Random(20261102)
    with open(dst, "w") as f:
        for i in range(n):
            if i % 2:
                x = struct.unpack("<d", struct.pack("<Q", rng.getrandbits(64)))[0]
                if not math.isfinite(x):
                    continue
            else:
                x = rng.getrandbits(rng.randint(1, 53)) / 2 ** rng.randint(0, 4) * 10 ** rng.randint(-8, 8)
                x = -x if rng.random() < 0.5 else x
            f.write(f"{bits(x):016x} {x!r}\n")


if __name__ == "__main__":
    main(int(sys.argv[1]), sys.argv[2])
