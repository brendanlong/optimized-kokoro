"""C vs Rust speed on the corpus calls: bench.py CALLS.jsonl [REPEATS] [VOICE...]"""
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
DATA = os.environ.get("ESPEAK_DATA", os.path.join(ROOT, "reference", ".cache", "espeak-ng-data"))
OUT = os.environ.get("ORACLE_OUT", os.path.join(ROOT, "scratch", "out"))
CBENCH = os.path.join(HERE, "bench")
RBENCH = os.environ.get("RBENCH", os.path.join(ROOT, "target", "release", "examples", "bench"))

path = sys.argv[1]
repeats = sys.argv[2] if len(sys.argv) > 2 else "3"
recs = [json.loads(l) for l in open(path).read().split("\n") if l]
voices = sys.argv[3:] or sorted({r["voice"] for r in recs})
print(f"{'voice':<12} {'calls':>6} {'words':>7} {'C us/call':>10} {'rs us/call':>10} {'C us/word':>10} {'rs us/word':>10} {'rs/C':>6}")
for v in voices:
    texts = [r["text"] for r in recs if r["voice"] == v]
    words = sum(max(1, len(t.split())) for t in texts)
    payload = b"".join(v.encode() + b"\0" + t.encode() + b"\0" for t in texts)
    c = subprocess.run([CBENCH, DATA, repeats], input=payload, capture_output=True).stdout.decode().split()
    c_s = float(c[3])
    r = subprocess.run([RBENCH, path, v, DATA, repeats], capture_output=True).stdout.decode().split()
    r_s = float(r[5])
    print(f"{v:<12} {len(texts):>6} {words:>7} {c_s * 1e6 / len(texts):>10.2f} {r_s * 1e6 / len(texts):>10.2f} {c_s * 1e6 / words:>10.2f} {r_s * 1e6 / words:>10.2f} {r_s / c_s:>6.2f}", flush=True)
