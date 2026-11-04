"""The Triton conv1d (kokoro_worker/kernels.py) against cuDNN, layer by layer: error, and time for
fresh lengths (first call) and repeats, eager and compiled with dynamic=True.

    uv run python bench/kernels_bench.py
"""

import os
import sys
import time

import torch
import torch.nn.functional as F

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from kokoro_worker import kernels  # noqa: E402

# (in, out, kernel, dilation, stride, padding, typical length at 400 frames): the decoder's layers
LAYERS = [
    (1090, 1024, 3, 1, 1, 1, 400), (1024, 1024, 3, 1, 1, 1, 400), (512, 64, 1, 1, 1, 0, 400),
    (256, 256, 3, 1, 1, 1, 8000), (256, 256, 7, 3, 1, 9, 8000), (256, 256, 11, 5, 1, 25, 8000),
    (128, 128, 3, 1, 1, 1, 48001), (128, 128, 7, 5, 1, 15, 48001), (128, 128, 11, 1, 1, 5, 48001),
    (22, 256, 12, 1, 6, 4, 48001), (128, 22, 7, 1, 1, 3, 48001), (1, 1, 3, 1, 2, 1, 800),
]


def timed(fn):
    torch.cuda.synchronize()
    t = time.perf_counter()
    fn()
    torch.cuda.synchronize()
    return 1000 * (time.perf_counter() - t)


def main():
    torch.manual_seed(0)
    tot = {"cudnn": [0.0, 0.0], "triton": [0.0, 0.0], "triton compiled": [0.0, 0.0]}
    for c, o, k, d, s, p, n in LAYERS:
        conv = torch.nn.Conv1d(c, o, k, stride=s, padding=p, dilation=d).cuda().half()
        tc = torch.nn.Conv1d(c, o, k, stride=s, padding=p, dilation=d).cuda().half()
        tc.load_state_dict(conv.state_dict())
        with torch.no_grad():
            tc._w_koc = tc.weight.detach().permute(2, 0, 1).contiguous()
            tc._b16 = tc.bias.detach()
        tc.__class__ = kernels.TritonConv1d
        compiled = torch.compile(tc, dynamic=True)
        x = torch.randn(1, c, n, device="cuda", dtype=torch.float16)
        with torch.no_grad():
            ref, got = conv(x), tc(x)
            err = float((ref.float() - got.float()).abs().max() / ref.float().abs().max())
            compiled(torch.randn(1, c, n + 7, device="cuda", dtype=torch.float16))  # compile once
            row = {}
            for name, fn in (("cudnn", conv), ("triton", tc), ("triton compiled", compiled)):
                first, rep = [], []
                for m in range(1, 6):
                    xi = torch.randn(1, c, n + 13 * m + (1 if name == "triton" else 0) + (2 if name == "cudnn" else 0),
                                     device="cuda", dtype=torch.float16)
                    first.append(timed(lambda: fn(xi)))
                    rep.append(min(timed(lambda: fn(xi)) for _ in range(3)))
                f, r = sum(first) / 5, sum(rep) / 5
                tot[name][0] += f
                tot[name][1] += r
                row[name] = f"{f:6.2f}/{r:6.2f}"
        print(f"conv {c:4d}->{o:4d} k{k:2d} d{d} s{s} T={n:6d}: rel err {err:.1e}; first/repeat ms  "
              + "  ".join(f"{k}: {v}" for k, v in row.items()), flush=True)
    print("sum over the layers above, first/repeat ms: "
          + "  ".join(f"{k}: {v[0]:.2f}/{v[1]:.2f}" for k, v in tot.items()))


if __name__ == "__main__":
    main()
