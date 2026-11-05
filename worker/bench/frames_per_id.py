"""How many frames per phoneme id does Kokoro predict? The front sizes chunks for the worker's cap
from this (crates/server/src/stitch.rs, `FRAMES_PER_ID`).

Durations only (the text side), fp16 as the worker runs, in 8 voices at speeds 0.25, 0.5 and 1:
- every corpus chunk as the pipeline makes it;
- windows of N ids of each document's chunks joined with spaces, starting at a word, like the
  pieces of a stitched chunk.
Reports frames x speed / ids (boundary ids' frames included, not counted as ids) by length.

    python bench/frames_per_id.py MODELS [--device cuda] [--windows 300]
"""

import argparse
import json
import os
import sys
from collections import defaultdict

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import Synth  # noqa: E402
from kokoro_worker.protocol import max_duration  # noqa: E402

SPEEDS = (25, 50, 100)
VOICES = ["af_heart", "am_adam", "bf_emma", "bm_george", "af_bella", "am_michael", "af_nicole", "bm_lewis"]
WINDOWS = (16, 32, 64, 128, 256, 400, 510)


def documents(vocab):
    """[[ids of each chunk]] per English document of the corpus."""
    out = []
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") in ("a", "b") and "chunks" in row:
                chunks = [[vocab[c] for c in ch["phonemes"] if c in vocab] for ch in row["chunks"]]
                out.append([c for c in chunks if c])
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--windows", type=int, default=300, help="windows per length")
    args = ap.parse_args()
    synth = Synth(args.models, device=args.device, half=args.device != "cpu", compile=False)
    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    space = vocab[" "]
    docs = documents(vocab)

    seqs = [("chunk", c) for doc in docs for c in doc if len(c) <= 510]
    for n in WINDOWS:
        found = []
        for doc in docs:
            joined = []
            for c in doc:
                joined += ([space] if joined else []) + c
            starts = [0] + [i + 1 for i, t in enumerate(joined) if t == space]
            found += [joined[s : s + n] for s in starts[:: max(1, n // 8)] if s + n <= len(joined)]
        step = max(1, len(found) // args.windows)
        seqs += [(f"window", w) for w in found[::step][: args.windows]]
    print(f"{sum(k == 'chunk' for k, _ in seqs)} chunks, {sum(k == 'window' for k, _ in seqs)} windows, "
          f"{len(VOICES)} voices", flush=True)

    dev = synth.device
    speed = torch.tensor([s / 100 for s in SPEEDS], device=dev)
    max_dur = torch.tensor([[max_duration(s)] for s in SPEEDS], device=dev)
    # ratio[(kind, bucket)][speed] = list of frames * speed / ids
    ratios = defaultdict(lambda: defaultdict(list))
    worst = defaultdict(lambda: (0.0, None))
    with torch.inference_mode():
        for name in VOICES:
            v = synth.voice_names.index(name)
            for kind, ids in seqs:
                n = len(ids)
                width = n + 2
                t = torch.zeros(len(SPEEDS), width, dtype=torch.long)
                t[:, 1 : n + 1] = torch.tensor(ids)
                t = t.to(dev)
                mask = torch.zeros(len(SPEEDS), width, dtype=torch.bool, device=dev)
                ref = synth.style(v, n)[None].expand(len(SPEEDS), -1)
                with torch.autocast(dev.type, dtype=torch.float16, enabled=synth.half):
                    dur, _, _ = synth._durations(t, mask, torch.tensor([width] * len(SPEEDS)), ref[:, 128:],
                                                 speed, max_dur)
                frames = dur.sum(1).tolist()
                bucket = n if kind == "window" else min(b for b in (8, 16, 32, 64, 128, 256, 510) if n <= b)
                for s, f in zip(SPEEDS, frames):
                    r = f * s / 100 / n
                    ratios[(kind, bucket)][s].append(r)
                    if r > worst[(kind, bucket, s)][0]:
                        worst[(kind, bucket, s)] = (r, (name, n, f))
            print(f"done {name}", flush=True)

    print("frames x speed / ids: max (p99, mean); kind, ids (chunks: up to), speed")
    for kind, bucket in sorted(ratios):
        cells = []
        for s in SPEEDS:
            xs = sorted(ratios[(kind, bucket)][s])
            p99 = xs[min(len(xs) - 1, int(0.99 * len(xs)))]
            cells.append(f"{s / 100}: {xs[-1]:.2f} ({p99:.2f}, {sum(xs) / len(xs):.2f}) {worst[(kind, bucket, s)][1]}")
        print(f"{kind:6} {bucket:4} n={len(ratios[(kind, bucket)][100]):5}  " + "  ".join(cells))


if __name__ == "__main__":
    main()
