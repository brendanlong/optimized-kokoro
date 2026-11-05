"""Closed-loop load of the longest requests the front sends for a chunk cap: windows of the corpus
(each document's chunks joined with spaces, starting at a word) of as many ids as
crates/server/src/stitch.rs `max_render_ids` allows at each speed, so packed rows fill up to the cap.

    python bench/cap_load.py SOCKET CONFIG_JSON --cap-seconds 30 [--concurrency 16] [--requests 200]

Prints one bench/load.py result per speed, with failures (too long) and the longest audio.
"""

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from load import run, wait_for  # noqa: E402

FRAMES_PER_ID_TENTHS = 47  # stitch.rs


def windows(vocab, n: int) -> list[tuple[str, list[int]]]:
    space = vocab[" "]
    out = []
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") not in ("a", "b") or "chunks" not in row:
                continue
            joined = []
            for chunk in row["chunks"]:
                ids = [vocab[c] for c in chunk["phonemes"] if c in vocab]
                joined += ([space] if joined and ids else []) + ids
            starts = [0] + [i + 1 for i, t in enumerate(joined) if t == space]
            out += [("", joined[s : s + n]) for s in starts[:: max(1, n // 16)] if s + n <= len(joined)]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("socket")
    ap.add_argument("config")
    ap.add_argument("--cap-seconds", type=float, required=True)
    ap.add_argument("--speeds", type=int, nargs="+", default=[25, 50, 100], help="hundredths")
    ap.add_argument("--concurrency", type=int, default=16)
    ap.add_argument("--requests", type=int, default=200, help="per speed")
    ap.add_argument("--voice", default="af_nicole", help="the voice with the longest durations")
    ap.add_argument("--json")
    args = ap.parse_args()

    vocab = json.load(open(args.config, encoding="utf-8"))["vocab"]
    client = wait_for(args.socket, 600)
    voice = client.hello.voices.index(args.voice)
    frames = round(args.cap_seconds * 40)
    results = []
    for speed in args.speeds:
        n = min(510, frames * speed // (10 * FRAMES_PER_ID_TENTHS))
        chunks = windows(vocab, n)
        r = run(client, chunks, args.concurrency, args.requests, voice, speed=speed)
        r.update(speed=speed / 100, ids=n, windows=len(chunks))
        results.append(r)
        print(json.dumps(r), flush=True)
    client.close()
    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=1)


if __name__ == "__main__":
    main()
