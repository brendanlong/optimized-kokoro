"""Peak GPU memory of the longest chunks: one request (or ROWS requests in one decoder group) with
durations forced to a given total frame count, compiled as the worker runs.

    uv run python bench/chunk_memory.py MODELS ROWS FRAMES [FRAMES ...]

Frame counts run in the order given, each twice; an out-of-memory error is reported and the next
one tried. 40 frames are one second of audio; a chunk at --max-chunk-seconds S and speed 0.25 (the
slowest) may reach 160 * S frames. (Based on the PR #37 review's memory.py.)
"""

import os
import sys
import time

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from kokoro_worker.model import Request, Synth  # noqa: E402
from kokoro_worker.server import warmup  # noqa: E402


def main():
    models, rows, frame_counts = sys.argv[1], int(sys.argv[2]), [int(a) for a in sys.argv[3:]]
    synth = Synth(models, max_chunk_samples=2**32 - 1, frame_budget=10**9)
    warmup(synth, max(rows, 1))
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
    n = 508
    reqs = [Request([ids[(i * 7) % len(ids)] for i in range(n)], synth.style(0, n), 25) for _ in range(rows)]
    for frames in frame_counts:
        target["frames"] = frames
        for _ in range(2):
            torch.cuda.empty_cache()
            torch.cuda.reset_peak_memory_stats()
            torch.cuda.synchronize()
            t = time.perf_counter()
            try:
                res = [r for g in synth.run(reqs) for r in g]
                torch.cuda.synchronize()
                status = ",".join(sorted({r.error or "ok" for r in res}))
            except torch.OutOfMemoryError:
                status = "OOM (raised)"
            print(f"rows {rows} x {frames} frames ({frames / 40:.0f} s of audio each): {status}, "
                  f"{time.perf_counter() - t:.2f} s, peak allocated {torch.cuda.max_memory_allocated() >> 20} MiB, "
                  f"peak reserved {torch.cuda.max_memory_reserved() >> 20} MiB", flush=True)


if __name__ == "__main__":
    main()
