"""GPU time by kernel for rows of forced length: one long request against the same frames packed
from several requests, compiled as the worker runs.

    uv run python bench/profile_rows.py MODELS ROWSxFRAMES [ROWSxFRAMES ...]
"""

import collections
import os
import sys
import time

import torch
from torch.profiler import ProfilerActivity, profile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from batch_scale import kind  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402
from kokoro_worker.server import warmup  # noqa: E402


def main():
    import logging

    logging.basicConfig(level=logging.WARNING, format="%(levelname)s %(message)s")
    models, specs = sys.argv[1], [tuple(int(v) for v in a.split("x")) for a in sys.argv[2:]]
    synth = Synth(models, max_chunk_samples=2**32 - 1, frame_budget=10**9)
    warmup(synth, 16)
    durations = synth._durations
    target = {"frames": 0}

    def forced(ids, mask, lengths, s, speed, max_dur):
        pred, d, t_en = durations(ids, mask, lengths, s, speed, max_dur)
        n = pred.shape[1]
        per = target["frames"] // n
        pred = torch.full_like(pred, per)
        pred[:, -1] += target["frames"] - per * n
        return pred.masked_fill(mask, 0), d, t_en

    synth._durations = forced
    ids = sorted(synth.vocab_ids)
    from kokoro_worker import kernels

    seen = collections.Counter()
    launch = kernels._conv1d_kernel

    class Recorder:
        def __getitem__(self, grid):
            def call(x, *args, **kw):
                seen[(tuple(x.shape[1:2]), x.stride(), x.dtype, x.is_contiguous())] += 1
                return launch[grid](x, *args, **kw)

            return call

    for rows, frames in specs:
        target["frames"] = frames
        seen.clear()
        kernels._conv1d_kernel = Recorder()
        list(synth.run([Request([ids[0]] * 100, synth.style(0, 100), 100)] * rows))
        kernels._conv1d_kernel = launch
        bad = {k: v for k, v in seen.items() if not k[3]}
        print(f"conv inputs for {rows} rows: {sum(seen.values())} calls, {sum(bad.values())} non-contiguous: "
              f"{list(bad)[:4]}", flush=True)
        n = 100
        reqs = [Request([ids[(i * 7 + r) % len(ids)] for i in range(n)], synth.style(0, n), 100) for r in range(rows)]
        for _ in range(2):
            list(synth.run(reqs))
        torch.cuda.synchronize()
        t = time.perf_counter()
        list(synth.run(reqs))
        torch.cuda.synchronize()
        wall = time.perf_counter() - t
        with profile(activities=[ProfilerActivity.CUDA]) as prof:
            list(synth.run(reqs))
            torch.cuda.synchronize()
        by_kind, by_name = collections.Counter(), collections.Counter()
        for e in prof.events():
            if e.device_type == torch.autograd.DeviceType.CUDA:
                by_kind[kind(e.name)] += e.device_time
                by_name[e.name] += e.device_time
        print(f"== {rows} x {frames} frames: {1000 * wall:.0f} ms, GPU {sum(by_kind.values()) / 1000:.0f} ms: "
              + ", ".join(f"{k} {v / 1000:.0f}" for k, v in by_kind.most_common()), flush=True)
        for name, us in by_name.most_common(12):
            print(f"  {us / 1000:7.1f} ms  {name[:110]}", flush=True)


if __name__ == "__main__":
    main()
