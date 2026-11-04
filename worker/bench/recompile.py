"""After the server's warmup, random requests (batch 1..MAX_BATCH, lengths 1..510, speeds 25..400):
reports any new Triton conv1d variant, autotune entry or dynamo recompile, and the slowest calls.

    uv run python bench/recompile.py MODELS MAX_BATCH N [SEED]

(From the PR #37 review.)
"""
import sys, os, random, time, torch
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from kokoro_worker import kernels as K
from kokoro_worker.model import Request, Synth
from kokoro_worker.server import warmup
import torch._dynamo.utils as du
models, max_batch, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
synth = Synth(models)
t = time.monotonic(); warmup(synth, max_batch); print(f"warmup {time.monotonic()-t:.1f}s", flush=True)
jit = K._conv1d_kernel.fn
def variants():
    return sum(len(v[0]) for v in jit.device_caches.values())
v0, a0 = variants(), len(K._conv1d_kernel.cache)
print("after warmup: triton variants", v0, "autotune entries", a0, "dynamo", dict(du.counters["stats"]), flush=True)
ids = sorted(synth.vocab_ids)
random.seed(int(sys.argv[4]) if len(sys.argv) > 4 else 0)
times = []
for i in range(n):
    b = random.randint(1, max_batch)
    reqs = []
    for _ in range(b):
        L = random.choice([random.randint(1, 30), random.randint(1, 510)])
        reqs.append(Request([random.choice(ids) for _ in range(L)], synth.style(random.randrange(len(synth.voice_names)), L),
                            random.choice([25, 50, 77, 100, 100, 100, 133, 400])))
    before = variants()
    torch.cuda.synchronize(); t = time.perf_counter()
    res = [r for g in synth.run(reqs) for r in g]
    torch.cuda.synchronize(); dt = time.perf_counter() - t
    times.append(dt)
    after = variants()
    if after != before or dt > 1.0:
        print(f"req {i}: b={b} lens={[len(r.ids) for r in reqs]} {dt*1000:.0f} ms new variants {after-before}", flush=True)
    errs = [r.error for r in res if r.error]
    if errs: print("errors", errs)
times.sort()
print(f"n={n} median {times[n//2]*1000:.1f} ms p99 {times[int(n*.99)]*1000:.1f} max {times[-1]*1000:.1f}")
print("final: triton variants", variants(), "autotune entries", len(K._conv1d_kernel.cache), "dynamo", dict(du.counters["stats"]))
print("peak allocated MiB", torch.cuda.max_memory_allocated() >> 20, "reserved", torch.cuda.max_memory_reserved() >> 20)
# Latent: alignment specialization on pointers
x = torch.randn(1, 129, 1000, device="cuda", dtype=torch.float16)
w = torch.randn(3, 64, 128, device="cuda", dtype=torch.float16)
v = variants(); K.conv1d(x[:, 1:], w, None, 3, 1, 1, 1); torch.cuda.synchronize()
print("aligned slice (offset 1000 elems) new variants:", variants() - v)
v = variants(); K.conv1d(x[:, :128, 1:], w, None, 3, 1, 1, 1); torch.cuda.synchronize()
print("misaligned slice (offset 1 elem) new variants:", variants() - v)
