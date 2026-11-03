"""Builds crates/espeak-ng/tests/c_calls.jsonl: C answers for the engine's regression test.

    python3 make_fixture.py OUT.jsonl

Inputs are in $ORACLE_OUT (default scratch/out): calls.jsonl from `oracle.py calls` on an
espeak_dump.py corpus dump, and gen_adversarial-2.jsonl from `adv.py gen_adversarial.py 2 1 3000`
with the twelve voices below.

Sections, each replayed with one engine per voice in file order (as compare.rs does):
  - the first 250 corpus calls per voice from calls.jsonl
  - the first 30 adversarial calls per voice from gen_adversarial-2.jsonl
  - minimized regression cases, run through the C oracle with fresh state
"""
import collections
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.environ.get("ORACLE_OUT", os.path.join(HERE, "..", "..", "scratch", "out"))
VOICES = ["gmw/en-US", "gmw/en", "roa/es", "roa/fr", "roa/it", "roa/pt-BR", "inc/hi", "zle/ru", "grk/el", "ko", "sem/ar", "sem/he"]


def head_per_voice(path, n):
    count = collections.Counter()
    out = []
    for line in open(path):
        r = json.loads(line)
        v = r["voice"]
        if v not in VOICES or count[v] >= n:
            continue
        if not r.get("reset"):
            count[v] += 1
        out.append(r)
    return out


def reset_all():
    return [{"voice": v, "text": "", "reset": True} for v in VOICES + ["inc/si"]]


regression = [
    ("gmw/en-US", "one"),
    ("gmw/en-US", "Třebíč"),
    ("gmw/en-US", "µ in it"),
    ("roa/it", "µ in it"),
    ("zle/ru", "o-eώ"),
    ("inc/si", "5-]"),
    ("gmw/en-US", "\\\x01B Hello, world. Again: (yes) \"no\"!"),
    ("gmw/en-US", "And after the switch, commas, stay; spoken?"),
    ("roa/fr", "»\\\x01B.,\n\n2006. if\n\nABC\n"),
    ("roa/fr", "1 000 000 et 21, 71 ou 91."),
    ("gmw/en", "99999999999.NET 1,234.56 3rd 1990-2000 12:30 £5 50%"),
    ("inc/hi", "नमस्ते 123 दुनिया।"),
    ("ko", "안녕하세요 123 세계"),
    ("sem/ar", "مرحبا 123 بالعالم"),
    ("sem/he", "שלום 123 עולם"),
    ("grk/el", "Καλημέρα 123 κόσμε"),
]
pairs = "".join(json.dumps({"voice": v, "text": t}, ensure_ascii=False) + "\n" for v, t in regression)
proc = subprocess.run([sys.executable, os.path.join(HERE, "oracle.py"), "pairs"], input=pairs.encode(), capture_output=True, check=True)
regression_calls = [json.loads(l) for l in proc.stdout.decode().splitlines() if l]
assert len(regression_calls) == len(regression), proc.stderr.decode()

records = (
    head_per_voice(os.path.join(OUT, "calls.jsonl"), 250)
    + reset_all()
    + head_per_voice(os.path.join(OUT, "gen_adversarial-2.jsonl"), 30)
    + reset_all()
    + regression_calls
)
with open(sys.argv[1], "w") as f:
    for r in records:
        f.write(json.dumps(r, ensure_ascii=False) + "\n")
print(len(records), "records")
