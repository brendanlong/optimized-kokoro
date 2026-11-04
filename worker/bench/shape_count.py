"""How many distinct shapes run over 1,000 real requests, with and without size buckets?

    uv run python bench/shape_count.py MODELS [--bucket-ratio 0.05] [--compile]

Requests: every English chunk of bench/corpus.jsonl at speed 1 (650), then the first
of them again at speed 1.2 up to 1,000. Counts distinct text-side shapes (batch, ids), decoder
shapes (batch, frames), convolution calls by input/weight shape and parameters (each is a cuDNN
execution plan), cuFFT plans, and, compiled, dynamo graphs. Convolutions are seen through module
hooks, so they are counted in eager mode only.
"""

import argparse
import json
import os
import sys

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402

class ConvShapes:
    """Forward pre-hooks on every Conv1d/ConvTranspose1d (eager only): each distinct (input shape,
    weight shape, dtype, stride, padding, dilation, groups, transposed) is one cuDNN plan."""

    def __init__(self, model):
        self.convs, self.calls = set(), 0
        for m in model.modules():
            if isinstance(m, (torch.nn.Conv1d, torch.nn.ConvTranspose1d)):
                m.register_forward_pre_hook(self.hook)

    def hook(self, m, inputs):
        x = inputs[0]
        self.convs.add((tuple(x.shape), tuple(m.weight.shape), m.stride, m.padding, m.dilation, m.groups,
                        isinstance(m, torch.nn.ConvTranspose1d)))
        self.calls += 1


def requests_1000(synth, models):
    vocab = json.load(open(os.path.join(models, "config.json")))["vocab"]
    chunks = [ids for _, ids in sentences.load(vocab, sentences.CORPUS)]
    voice = synth.voice_names.index("af_heart")
    reqs = [Request(ids, synth.style(voice, len(ids)), 100) for ids in chunks]
    reqs += [Request(ids, synth.style(voice, len(ids)), 120) for ids in chunks[: 1000 - len(reqs)]]
    return reqs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--bucket-ratio", type=float, default=0.0)
    ap.add_argument("--compile", action="store_true")
    args = ap.parse_args()

    synth = Synth(args.models, compile=args.compile, bucket_ratio=args.bucket_ratio)
    text_shapes, dec_shapes, frames_seen = set(), set(), set()
    durations, decode = synth._durations, synth._decode

    def durations_hook(ids, *a):
        text_shapes.add(tuple(ids.shape))
        return durations(ids, *a)

    def decode_hook(idx, frames, *a):
        dec_shapes.add((len(idx), synth._width(max(frames))))
        frames_seen.update(frames)
        return decode(idx, frames, *a)

    synth._durations, synth._decode = durations_hook, decode_hook
    reqs = requests_1000(synth, args.models)
    from torch._dynamo.utils import counters

    mode = None if args.compile else ConvShapes(synth.model)
    cufft = torch.backends.cuda.cufft_plan_cache[0]
    cufft.clear()
    for r in reqs:
        list(synth.run([r]))
    label = f"buckets {args.bucket_ratio}" if synth.buckets else "exact sizes"
    print(f"{label}, {'compiled' if args.compile else 'eager'}: {len(reqs)} requests, "
          f"{len(frames_seen)} distinct frame counts", flush=True)
    print(f"  text-side shapes (batch, ids): {len(text_shapes)}")
    print(f"  decoder shapes (batch, frames): {len(dec_shapes)}")
    if not args.compile:
        print(f"  distinct convolutions (cuDNN plans): {len(mode.convs)} over {mode.calls} calls "
              f"({mode.calls // len(reqs)} per request)")
    print(f"  cuFFT plan cache entries: {cufft.size} (max {cufft.max_size})")
    print(f"  dynamo: {dict(counters['stats'])}", flush=True)


if __name__ == "__main__":
    main()
