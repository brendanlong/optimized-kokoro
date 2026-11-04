"""Why is the first request of each size slow? Times first-time and repeated sizes by stage.

    uv run python bench/first_shape.py MODELS [--n 120] [--no-compile] [--profile] [--bucket-ratio R]

After the server's own warmup, for each real sentence of a length not seen yet: one call (first
time), then the same call twice more (repeated). Each call is split into the text side and the
decoder, with CUDA syncs around each. Then the same sentences at speed 1.03, which keeps the text
side's shape and changes only the decoder's frame count. Counts dynamo recompiles throughout.
Environment variables set before this starts (TORCH_CUDNN_V8_API_DISABLED and so on) apply.
"""

import argparse
import json
import os
import statistics
import sys
import time

import numpy as np
import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402
from kokoro_worker.server import warmup  # noqa: E402

timing = {"text": 0.0, "decode": 0.0}


def timed(name, fn):
    def wrapper(*a, **k):
        torch.cuda.synchronize()
        t = time.perf_counter()
        out = fn(*a, **k)
        torch.cuda.synchronize()
        timing[name] += time.perf_counter() - t
        return out
    return wrapper


def call(synth, req):
    timing.update(text=0.0, decode=0.0)
    torch.cuda.synchronize()
    t = time.perf_counter()
    for _ in synth.run([req]):
        pass
    torch.cuda.synchronize()
    return 1000 * (time.perf_counter() - t), 1000 * timing["text"], 1000 * timing["decode"]


def summary(label, rows):
    tot, text, dec = zip(*rows)
    print(f"{label:34s} n={len(rows):3d}  total median {statistics.median(tot):6.1f} ms "
          f"(p90 {np.percentile(tot, 90):6.1f})  text {statistics.median(text):5.1f}  "
          f"decoder {statistics.median(dec):6.1f}", flush=True)


def graphs():
    from torch._dynamo.utils import counters
    return sum(counters["stats"].values()), dict(counters["stats"])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--n", type=int, default=120)
    ap.add_argument("--no-compile", action="store_true")
    ap.add_argument("--profile", action="store_true")
    ap.add_argument("--bucket-ratio", type=float, default=0.0)
    ap.add_argument("--json")
    args = ap.parse_args()

    print(f"torch {torch.__version__}, cudnn {torch.backends.cudnn.version()}, "
          f"cudnn.enabled={torch.backends.cudnn.enabled} benchmark={torch.backends.cudnn.benchmark} "
          f"deterministic={torch.backends.cudnn.deterministic}, "
          f"V8_API_DISABLED={os.environ.get('TORCH_CUDNN_V8_API_DISABLED')}", flush=True)
    t = time.monotonic()
    synth = Synth(args.models, compile=not args.no_compile, bucket_ratio=args.bucket_ratio)
    synth._durations = timed("text", synth._durations)
    synth._decode = timed("decode", synth._decode)
    warmup(synth, 1)
    print(f"load + warmup {time.monotonic() - t:.1f} s, buckets={synth.buckets}", flush=True)

    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    by_len = {}
    for _, ids in sentences.load(vocab):
        by_len.setdefault(len(ids), ids)
    rng = np.random.default_rng(0)
    lengths = [int(x) for x in rng.permutation(sorted(by_len))[: args.n]]
    voice = synth.voice_names.index("af_heart")
    reqs = [Request(by_len[n], synth.style(voice, n), 100) for n in lengths]
    print(f"{len(reqs)} sentences of distinct lengths {min(lengths)}..{max(lengths)} phonemes", flush=True)

    g0, _ = graphs()
    first, rep = [], []
    for r in reqs:
        first.append(call(synth, r))
        call(synth, r)
        rep.append(call(synth, r))
    summary("first time (new text + frames)", first)
    summary("repeated", rep)

    # Same text shape, new frame counts: speed 1.03 shortens durations.
    first_dec, rep_dec = [], []
    for r in reqs:
        r2 = Request(r.ids, r.ref_s, 103)
        first_dec.append(call(synth, r2))
        rep_dec.append(call(synth, r2))
    summary("same text shape, new frames", first_dec)
    summary("  ...repeated", rep_dec)
    g1, stats = graphs()
    print(f"dynamo counters during the timed calls: {g1 - g0} new ({stats})", flush=True)
    print(f"GPU memory: allocated {torch.cuda.memory_allocated() >> 20} MiB, reserved "
          f"{torch.cuda.memory_reserved() >> 20} MiB, peak reserved {torch.cuda.max_memory_reserved() >> 20} MiB")

    if args.profile:
        from torch.profiler import ProfilerActivity, profile

        fresh = [Request(r.ids, r.ref_s, 107) for r in reqs[:5]]
        for label, batch in (("FIRST-TIME", fresh), ("REPEATED", fresh)):
            with profile(activities=[ProfilerActivity.CPU, ProfilerActivity.CUDA]) as prof:
                for r in batch:
                    call(synth, r)
            print(f"== profile, 5 {label} calls, by self CPU time")
            print(prof.key_averages().table(sort_by="self_cpu_time_total", row_limit=25), flush=True)

    if args.json:
        with open(args.json, "w") as f:
            json.dump({"lengths": lengths, "first": first, "repeated": rep, "first_dec": first_dec,
                       "rep_dec": rep_dec}, f)


if __name__ == "__main__":
    main()
