"""Why are batch-of-one calls slower in a worker that has also compiled batched graphs?

Times B=1 calls on real sentences: after compiling only B=1, then after compiling B>1 too,
and splits the time into the text side and the decoder.
"""

import json
import os
import sys
import time

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402

models = sys.argv[1]
synth = Synth(models)
vocab = json.load(open(os.path.join(models, "config.json")))["vocab"]
chunks = [ids for _, ids in sentences.load(vocab)][:120]
reqs = [Request(ids, synth.style(0, len(ids)), 100) for ids in chunks]

timing = {"text": 0.0, "decode": 0.0}
orig_durations, orig_decode = synth._durations, synth._decode


def timed(name, fn):
    def wrapper(*a, **k):
        torch.cuda.synchronize()
        t = time.perf_counter()
        out = fn(*a, **k)
        torch.cuda.synchronize()
        timing[name] += time.perf_counter() - t
        return out
    return wrapper


synth._durations = timed("text", orig_durations)
synth._decode = timed("decode", orig_decode)


def bench(label):
    for r in reqs[:20]:
        list(synth.run([r]))
    timing.update(text=0.0, decode=0.0)
    torch.cuda.synchronize()
    t = time.perf_counter()
    for r in reqs:
        list(synth.run([r]))
    torch.cuda.synchronize()
    n = len(reqs)
    print(f"{label}: {1000 * (time.perf_counter() - t) / n:.1f} ms/call "
          f"(text {1000 * timing['text'] / n:.1f}, decoder {1000 * timing['decode'] / n:.1f})", flush=True)


bench("B=1 only compiled")
for lengths in ([50, 50], [40, 44], [100, 104, 110]):
    list(synth.run([Request(chunks[0][:n] if len(chunks[0]) >= n else chunks[0], reqs[0].ref_s, 100)
                    for n in lengths]))
    list(synth.run([reqs[i] for i in range(len(lengths))]))
bench("after B>1 compiled")
torch._dynamo.reset()
bench("after dynamo reset (recompiles B=1 only)")
