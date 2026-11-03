"""Differential runs on generated texts: adv.py GENERATOR FIRST_SEED COUNT N VOICE...

For each seed: generate N texts, get C's answers with per-voice history (oracle.py pairs), and
compare with the Rust engine. Prints the compare summary lines that aren't 100%.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
DATA = os.environ.get("ESPEAK_DATA", os.path.join(ROOT, "reference", ".cache", "espeak-ng-data"))
OUT = os.environ.get("ORACLE_OUT", os.path.join(ROOT, "scratch", "out"))
COMPARE = os.path.join(ROOT, "target", "release", "examples", "compare")

gen, first, count, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
voices = sys.argv[5:]
for seed in range(first, first + count):
    name = f"{os.path.splitext(os.path.basename(gen))[0]}-{seed}"
    pairs = os.path.join(OUT, name + ".pairs.jsonl")
    calls = os.path.join(OUT, name + ".jsonl")
    with open(pairs, "w") as f:
        subprocess.run([sys.executable, os.path.join(HERE, gen), str(seed), n, *voices], stdout=f, check=True)
    with open(pairs) as fin, open(calls, "w") as fout, open(calls + ".err", "w") as ferr:
        subprocess.run([sys.executable, os.path.join(HERE, "oracle.py"), "pairs"], stdin=fin, stdout=fout, stderr=ferr, check=True)
    r = subprocess.run([COMPARE, calls, DATA, "-", "0"], capture_output=True, text=True)
    lines = [l for l in r.stdout.splitlines()[1:] if "100.00%" not in l.split()[2:3]]
    crashes = sum(1 for l in open(calls + ".err") if l.startswith("C crashed"))
    print(f"seed {seed}: C crashes {crashes}; " + ("all match" if not lines else "\n  " + "\n  ".join(lines)), flush=True)
