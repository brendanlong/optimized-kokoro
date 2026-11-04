"""Does padding to size buckets change the audio or the durations?

    uv run python bench/equiv_buckets.py MODELS [--bucket-ratio 0.05] [--fp32] [--compile] [--n 720]
        [--max-batch 1]

Every real chunk (or --n of them) at batch 1, through one model: exact sizes with seed 2, exact
sizes with seed 3 (the noise floor: the harmonic source draws random noise), and bucketed with
seed 2. Durations must be identical; audio is compared by log-spectrogram correlation against the
floor, and by sample length. With --max-batch 8, batches of 8 bucketed vs the same batches unbucketed.
"""

import argparse
import json
import os
import sys

import numpy as np
import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from check_equivalence import pcm, run, spec_corr  # noqa: E402
from kokoro_worker.model import Request, Synth, batch_buckets  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--bucket-ratio", type=float, default=0.05)
    ap.add_argument("--fp32", action="store_true")
    ap.add_argument("--compile", action="store_true")
    ap.add_argument("--n", type=int, default=0)
    ap.add_argument("--max-batch", type=int, default=1)
    args = ap.parse_args()

    synth = Synth(args.models, half=not args.fp32, compile=args.compile, bucket_ratio=args.bucket_ratio,
                  max_batch=args.max_batch)
    assert synth.buckets
    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    chunks = [ids for _, ids in sentences.load(vocab, sentences.CORPUS)]
    if args.n:
        chunks = chunks[: args.n]
    voice = synth.voice_names.index("af_heart")
    reqs = [Request(ids, synth.style(voice, len(ids)), 100) for ids in chunks]
    b = args.max_batch

    def go(bucketed, seed):
        synth.buckets = bucketed
        synth.batch_sizes = batch_buckets(b) if bucketed else [b]
        out = []
        for i in range(0, len(reqs), b):
            out += run(synth, reqs[i : i + b], seed)
        return out

    exact = go(False, 2)
    floor_run = go(False, 3)
    bucketed = go(True, 2)
    dur_eq = sum(x.durations == y.durations for x, y in zip(exact, bucketed))
    len_eq = sum(len(x.pcm) == len(y.pcm) for x, y in zip(exact, bucketed))
    corr = np.array([spec_corr(pcm(x), pcm(y)) for x, y in zip(exact, bucketed)])
    floor = np.array([spec_corr(pcm(x), pcm(y)) for x, y in zip(exact, floor_run)])
    frames = [sum(x.durations) for x in exact]
    pad = [synth._width(f) / f - 1 for f in frames]
    label = f"{'fp32' if args.fp32 else 'fp16'} {'compiled' if args.compile else 'eager'} batch {b}"
    print(f"{label}, bucket ratio {args.bucket_ratio}: {len(reqs)} chunks, frames {min(frames)}..{max(frames)}, "
          f"decoder padding mean {100 * np.mean(pad):.1f}% max {100 * np.max(pad):.1f}%")
    print(f"  durations identical {dur_eq}/{len(reqs)}, sample counts identical {len_eq}/{len(reqs)}")
    print(f"  spectrogram corr bucketed vs exact: mean {corr.mean():.4f} min {corr.min():.4f} p1 "
          f"{np.percentile(corr, 1):.4f}")
    print(f"  seed noise floor (exact vs exact):  mean {floor.mean():.4f} min {floor.min():.4f} p1 "
          f"{np.percentile(floor, 1):.4f}", flush=True)
    bad = [i for i, (x, y) in enumerate(zip(exact, bucketed)) if x.durations != y.durations]
    if bad:
        same = np.array([i not in bad for i in range(len(reqs))])
        print(f"  corr where durations are identical: mean {corr[same].mean():.4f} min {corr[same].min():.4f}; "
              f"where not: mean {corr[~same].mean():.4f} min {corr[~same].min():.4f}")
        n_diff = sum(sum(p != q for p, q in zip(exact[i].durations, bucketed[i].durations)) for i in bad)
        print(f"  {n_diff} of {sum(len(x.durations) for x in exact)} durations differ")
    for i in bad[:5]:
        diff = [(k, p, q) for k, (p, q) in enumerate(zip(exact[i].durations, bucketed[i].durations)) if p != q]
        print(f"  chunk {i} ({len(chunks[i])} ids): {diff[:5]}")


if __name__ == "__main__":
    main()
