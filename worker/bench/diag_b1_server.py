"""Batch-of-one speed after the server's own warmup, at --max-batch 1 vs 8, in one process each.

    TORCH_LOGS=recompiles python bench/diag_b1_server.py MODELS MAX_BATCH
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
from kokoro_worker.server import warmup  # noqa: E402

models, max_batch = sys.argv[1], int(sys.argv[2])
synth = Synth(models)
warmup(synth, max_batch)
vocab = json.load(open(os.path.join(models, "config.json")))["vocab"]
chunks = [ids for _, ids in sentences.load(vocab)][:150]
reqs = [Request(ids, synth.style(0, len(ids)), 100) for ids in chunks]
for p in range(2):
    torch.cuda.synchronize()
    t = time.perf_counter()
    for r in reqs:
        list(synth.run([r]))
    torch.cuda.synchronize()
    print(f"max_batch {max_batch}, pass {p}: {1000 * (time.perf_counter() - t) / len(reqs):.1f} ms per batch of one",
          flush=True)
if max_batch > 1:
    for i in range(0, 64, 4):
        list(synth.run(reqs[i : i + 4]))
    torch.cuda.synchronize()
    t = time.perf_counter()
    for r in reqs:
        list(synth.run([r]))
    torch.cuda.synchronize()
    print(f"max_batch {max_batch}, after batches of 4: {1000 * (time.perf_counter() - t) / len(reqs):.1f} ms per batch of one",
          flush=True)
