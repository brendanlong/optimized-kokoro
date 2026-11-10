"""How throughput scales with batch size, in process (no socket), compiled as the worker runs.

    uv run python bench/batch_scale.py MODELS [--steps equal mixed profile] [--json out.json]

equal    B copies of one real chunk of about L phonemes.
mixed    B chunks drawn from the corpus, packed into decoder rows of at most --frame-budget
         frames (the worker's is its chunk cap: 2,400 at the default 60 s).
profile  torch.profiler at a few batch sizes: GPU time by kind of kernel, and GPU busy time as a
         share of unprofiled wall time (what is left is the GPU waiting on the host).

Throughput is seconds of audio per wall second. Each point runs once to warm up, then REPS times.
"""

import argparse
import collections
import json
import os
import random
import statistics
import sys
import time

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import SAMPLES_PER_FRAME, Request, Synth  # noqa: E402
from kokoro_worker.server import warmup  # noqa: E402

SAMPLE_RATE = 24000


class Timed:
    """Wraps Synth._durations to record text-side time (synchronized) per run."""

    def __init__(self, synth):
        self.inner = synth._durations
        self.text_s = 0.0
        synth._durations = self

    def __call__(self, *args):
        torch.cuda.synchronize()
        t = time.perf_counter()
        out = self.inner(*args)
        torch.cuda.synchronize()
        self.text_s += time.perf_counter() - t
        return out


def measure(synth, timed, requests, reps):
    def once():
        timed.text_s = 0.0
        torch.cuda.synchronize()
        t = time.perf_counter()
        groups, audio, errors = 0, 0, collections.Counter()
        for results in synth.run(requests):
            groups += 1
            for r in results:
                if r.pcm is None:
                    errors[r.error] += 1
                else:
                    audio += len(r.pcm) // 2
        torch.cuda.synchronize()
        return time.perf_counter() - t, timed.text_s, groups, audio / SAMPLE_RATE, errors

    torch.cuda.empty_cache()
    torch.cuda.reset_peak_memory_stats()
    try:
        _, _, _, _, errors = once()
    except AssertionError as e:  # the Triton conv1d's int32 offsets
        return {"errors": {str(e): len(requests)}}
    if errors:
        return {"errors": dict(errors)}
    runs = [once() for _ in range(reps)]
    wall = statistics.median(r[0] for r in runs)
    text = statistics.median(r[1] for r in runs)
    audio = runs[0][3]
    return {
        "wall_ms": round(1000 * wall, 1),
        "text_ms": round(1000 * text, 1),
        "groups": runs[0][2],
        "audio_s": round(audio, 2),
        "x_realtime": round(audio / wall, 1),
        "peak_gib": round(torch.cuda.max_memory_allocated() / 2**30, 2),
    }


def requests_for(synth, chunks):
    return [Request(ids, synth.style(0, len(ids)), 100) for ids in chunks]


def batch_sizes(limit):
    b, out = 1, []
    while b <= limit:
        out.append(b)
        b *= 2
    return out


def step_equal(synth, timed, corpus, args, out):
    print("== equal: B copies of one chunk", flush=True)
    for length in args.lengths:
        ids = min(corpus, key=lambda c: abs(len(c) - length))
        frames_one = None
        for b in batch_sizes(args.max_batch):
            if frames_one and b * frames_one > args.max_frames:
                break
            r = measure(synth, timed, requests_for(synth, [ids] * b), args.reps)
            r.update(length=len(ids), batch=b)
            if "audio_s" in r:
                frames_one = round(r["audio_s"] * SAMPLE_RATE / SAMPLES_PER_FRAME / b)
            out.append(r)
            print(json.dumps(r), flush=True)
            if "errors" in r:
                break


def step_mixed(synth, timed, corpus, args, out):
    print("== mixed: corpus chunks", flush=True)
    rng = random.Random(0)
    batches = {b: [rng.choice(corpus) for _ in range(b)] for b in batch_sizes(args.max_batch)}
    for b, chunks in batches.items():
        r = measure(synth, timed, requests_for(synth, chunks), args.reps)
        r.update(batch=b)
        out.append(r)
        print(json.dumps(r), flush=True)
        if "errors" in r:
            break


def kind(name):
    n = name.lower()
    if n.startswith("_conv1d_kernel"):
        return "triton conv1d"
    if "softmax" in n or "attention" in n or "fmha" in n or "flash" in n:
        return "attention"
    if any(k in n for k in ("gemm", "cutlass", "nvjet", "sm90", "sm80", "cublas", "matmul")):
        return "matmul (cuBLAS)"
    if "lstm" in n or "rnn" in n:
        return "LSTM (cuDNN)"
    # Inductor names fused kernels after the ops they absorbed, "conv1d" included.
    if n.startswith("triton_"):
        return "inductor fused elementwise/reduction"
    if "elementwise" in n or "vectorized" in n or "reduce" in n or "unrolled" in n:
        return "aten elementwise/reduction"
    if "memcpy" in n or "memset" in n:
        return "memcpy/memset"
    return "other"


def step_profile(synth, timed, corpus, args, out):
    from torch.profiler import ProfilerActivity, profile

    print("== profile", flush=True)
    rng = random.Random(1)
    for b in args.profile_batches:
        reqs = requests_for(synth, [rng.choice(corpus) for _ in range(b)])
        for _ in range(2):
            for _ in synth.run(reqs):
                pass
        torch.cuda.synchronize()
        t = time.perf_counter()
        for _ in synth.run(reqs):
            pass
        torch.cuda.synchronize()
        plain = time.perf_counter() - t
        with profile(activities=[ProfilerActivity.CUDA]) as prof:
            t = time.perf_counter()
            for _ in synth.run(reqs):
                pass
            torch.cuda.synchronize()
            wall = time.perf_counter() - t
        by_kind, by_name, launches = collections.Counter(), collections.Counter(), 0
        for e in prof.events():
            if e.device_type == torch.autograd.DeviceType.CUDA:
                by_kind[kind(e.name)] += e.device_time
                by_name[e.name] += e.device_time
                launches += 1
        gpu_us = sum(by_kind.values())
        r = {
            "batch": b,
            "wall_ms": round(1000 * wall, 1),
            "gpu_busy_ms": round(gpu_us / 1000, 1),
            "unprofiled_wall_ms": round(1000 * plain, 1),
            # Against the unprofiled wall time: the profiler itself slows the host down.
            "gpu_busy_share": round(gpu_us / 1e6 / plain, 3),
            "gpu_events": launches,
            "by_kind_ms": {k: round(v / 1000, 1) for k, v in by_kind.most_common()},
            "top": [[k[:90], round(v / 1000, 2)] for k, v in by_name.most_common(15)],
        }
        out.append(r)
        print(json.dumps(r, indent=1), flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--steps", nargs="+", default=["equal", "mixed", "profile"])
    ap.add_argument("--lengths", type=int, nargs="+", default=[30, 100, 250])
    ap.add_argument("--max-batch", type=int, default=512)
    ap.add_argument("--max-frames", type=int, default=200_000, help="skip equal points beyond B x frames")
    ap.add_argument("--profile-batches", type=int, nargs="+", default=[1, 8, 64])
    # 75,000 frames is about 2**31 / 28,800 (the widest tensor's elements per frame), which the
    # Triton conv1d's int32 offsets can still address; the worker's own budget is its chunk cap.
    ap.add_argument("--frame-budget", type=int, default=75_000)
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--no-compile", action="store_true")
    ap.add_argument("--json")
    args = ap.parse_args()

    synth = Synth(args.models, compile=not args.no_compile, max_chunk_samples=2**32 - 1, frame_budget=args.frame_budget)
    t = time.perf_counter()
    warmup(synth, 8)
    print(f"{torch.cuda.get_device_name()}: warmup {time.perf_counter() - t:.0f} s", flush=True)
    vocab = json.load(open(os.path.join(args.models, "config.json"), encoding="utf-8"))["vocab"]
    corpus = [ids for _, ids in sentences.load(vocab)]
    print(f"{len(corpus)} corpus chunks, median {statistics.median(len(c) for c in corpus)} phonemes", flush=True)
    timed = Timed(synth)
    results = {"device": torch.cuda.get_device_name()}
    for step in args.steps:
        results[step] = []
        globals()[f"step_{step}"](synth, timed, corpus, args, results[step])
        if args.json:
            with open(args.json + ".tmp", "w") as f:
                json.dump(results, f, indent=1)
            os.replace(args.json + ".tmp", args.json)


if __name__ == "__main__":
    main()
