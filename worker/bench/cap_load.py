"""Closed-loop load of the longest requests the front sends for a chunk cap, so packed rows fill up
to the cap. Long chunks are consecutive corpus sentences joined up to 510 ids (as
bench/max_duration.py makes them). Each request is a window of as many ids as
crates/server/src/stitch.rs `max_render_ids` allows at its speed, starting at a word inside one of
those chunks, with the style row of the whole chunk, as the front renders a stitched piece.

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

# stitch.rs: a request of n ids may need up to FRAMES_PER_ID_TENTHS / 10 * n + FRAMES_OVERHEAD frames
# at speed 1.
FRAMES_PER_ID_TENTHS, FRAMES_OVERHEAD = 40, 120


def render_ids(cap_frames: int, speed: int) -> int:
    budget = max(0, cap_frames * speed // 100 - FRAMES_OVERHEAD)
    return min(510, budget * 10 // FRAMES_PER_ID_TENTHS)


def long_chunks(vocab) -> list[list[int]]:
    space = vocab[" "]
    out, cur = [], []
    for _, ids in sentences.load(vocab):
        if cur and len(cur) + 1 + len(ids) > 510:
            out.append(cur)
            cur = []
        cur = cur + [space] + ids if cur else list(ids)
    return out


def pieces(chunks, n: int, space: int) -> list[tuple[str, list[int], int]]:
    """Windows of n ids starting at a word, inside chunks longer than n, with the chunk's style row."""
    out = []
    for c in chunks:
        if len(c) <= n:
            continue
        starts = [0] + [i + 1 for i, t in enumerate(c) if t == space]
        out += [("", c[s : s + n], min(len(c), 510)) for s in starts[:: max(1, n // 16)] if s + n <= len(c)]
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
    chunks = long_chunks(vocab)
    client = wait_for(args.socket, 600)
    voice = client.hello.voices.index(args.voice)
    frames = round(args.cap_seconds * 40)
    results = []
    for speed in args.speeds:
        n = render_ids(frames, speed)
        requests = pieces(chunks, n, vocab[" "])
        if not requests:  # at 510 nothing is stitched: send the long chunks whole
            requests = [("", c) for c in chunks]
        r = run(client, requests, args.concurrency, args.requests, voice, speed=speed)
        r.update(speed=speed / 100, ids=n, distinct=len(requests))
        results.append(r)
        print(json.dumps(r), flush=True)
    client.close()
    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=1)


if __name__ == "__main__":
    main()
