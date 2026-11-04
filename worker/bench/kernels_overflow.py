"""The conv1d's int32 offsets: an input just past 2^31 elements must be refused, not wrap.

    uv run python bench/kernels_overflow.py

(From the PR #37 review.)
"""
import sys, os, torch
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from kokoro_worker import kernels as K
T = 2**30 + 2**22
x = torch.empty(2, 1, T, device="cuda", dtype=torch.float16)
x[0].fill_(1.0); x[1].fill_(2.0)
w = torch.ones(1, 1, 1, device="cuda", dtype=torch.float16)  # [K, O, C]
# stride 64 keeps y small; the custom op allocates y itself
try:
    y = K.conv1d(x, w, None, 1, 64, 0, 1)
    torch.cuda.synchronize()
    r1 = y[1, 0].float()
    print("row1 expected all 2.0: min", r1.min().item(), "max", r1.max().item(), "frac wrong", (r1 != 2).float().mean().item())
except Exception as e:
    print("EXC", type(e).__name__, str(e)[:300])
