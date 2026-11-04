"""Achieved TFLOP/s of the Triton conv1d against cuDNN on the decoder's layers at larger batches.

    uv run python bench/conv_peak.py [BATCH ...]
"""

import os
import sys

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from kernels_bench import LAYERS  # noqa: E402
from kokoro_worker import kernels  # noqa: E402


def best_ms(fn, reps=10):
    fn()
    start, end = torch.cuda.Event(enable_timing=True), torch.cuda.Event(enable_timing=True)
    times = []
    for _ in range(reps):
        start.record()
        fn()
        end.record()
        torch.cuda.synchronize()
        times.append(start.elapsed_time(end))
    return min(times)


def main():
    torch.backends.cudnn.benchmark = True
    batches = [int(a) for a in sys.argv[1:]] or [8, 64]
    for b in batches:
        tot = {"cudnn": 0.0, "triton": 0.0}
        for c, o, k, d, s, p, n in LAYERS:
            conv = torch.nn.Conv1d(c, o, k, stride=s, padding=p, dilation=d).cuda().half()
            tc = torch.nn.Conv1d(c, o, k, stride=s, padding=p, dilation=d).cuda().half()
            tc.load_state_dict(conv.state_dict())
            with torch.no_grad():
                tc._w_koc = tc.weight.detach().permute(2, 0, 1).contiguous()
                tc._b16 = tc.bias.detach()
            tc.__class__ = kernels.TritonConv1d
            # Long layers get shorter rows at large batches so one call stays under int32 offsets.
            t = min(n, 2**30 // (b * max(c, o)))
            x = torch.randn(b, c, t, device="cuda", dtype=torch.float16)
            t_out = (t + 2 * p - d * (k - 1) - 1) // s + 1
            flop = 2 * b * o * c * k * t_out
            row = []
            with torch.no_grad():
                for name, fn in (("cudnn", conv), ("triton", tc)):
                    ms = best_ms(lambda: fn(x))
                    tot[name] += ms
                    row.append(f"{name} {ms:7.3f} ms {flop / ms / 1e9:6.1f} TFLOP/s")
            print(f"B={b:3d} conv {c:4d}->{o:4d} k{k:2d} d{d} s{s} T={t:6d}: " + "  ".join(row), flush=True)
        print(f"B={b} sum: " + "  ".join(f"{k} {v:.2f} ms" for k, v in tot.items()), flush=True)


if __name__ == "__main__":
    main()
