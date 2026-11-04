"""The Triton conv1d against F.conv1d (fp32 of the fp16 inputs) on the GPU: every layer configuration
of the model x every tile configuration, at edge lengths, with guard bands around the output.

    uv run python bench/kernels_correct.py MODELS

(From the PR #37 review.)
"""
import sys, os, random, torch, torch.nn.functional as F, triton
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from kokoro_worker import kernels as K
from kokoro_worker.model import Synth
models = sys.argv[1]
synth = Synth(models, compile=False)
layers = sorted({(m.in_channels, m.out_channels, m.kernel_size[0], m.dilation[0], m.stride[0], m.padding[0], m.bias is not None)
                 for m in synth.model.modules() if isinstance(m, K.TritonConv1d)})
print(len(layers), "distinct conv1d configs:", layers)
jit = K._conv1d_kernel.fn
torch.manual_seed(0); random.seed(0)
worst = {}
bad = 0
for (c, o, k, dil, s, p, hb) in layers:
    for cfg in K._CONFIGS:
        bo, bt, bc = cfg.kwargs["BLOCK_O"], cfg.kwargs["BLOCK_T"], cfg.kwargs["BLOCK_C"]
        for t in [1, 2, 3, bt - 1, bt, bt + 1, 2 * bt + 1, s * bt - 1, s * bt, s * bt + 1, 1001, 54321]:
            tout = K._t_out(t, k, s, p, dil)
            if tout < 1:
                continue
            b = random.choice([1, 2, 3])
            x = torch.randn(b, c, t, device="cuda", dtype=torch.float16)
            w = (torch.randn(o, c, k, device="cuda") / (c * k) ** .5).half()
            bias = torch.randn(o, device="cuda").half() if hb else None
            wk = w.permute(2, 0, 1).contiguous()
            n = b * o * tout
            buf = torch.full((n + 8192,), 777., device="cuda", dtype=torch.float16)
            y = buf[4096:4096 + n].view(b, o, tout)
            grid = (triton.cdiv(tout, bt), triton.cdiv(o, bo), b)
            jit[grid](x, wk, bias if hb else wk, y, t, tout, x.stride(0), x.stride(1), x.stride(2),
                      y.stride(0), y.stride(1), y.stride(2), C=c, O=o, K=k, DIL=dil, STRIDE=s, PAD=p, HAS_BIAS=hb,
                      BLOCK_O=bo, BLOCK_T=bt, BLOCK_C=bc, num_warps=cfg.num_warps, num_stages=cfg.num_stages)
            ref = F.conv1d(x.float(), w.float(), None if bias is None else bias.float(), s, p, dil)
            err = ((y.float() - ref).abs().max() / ref.abs().max().clamp_min(1e-6)).item()
            guard = bool((buf[:4096] == 777).all() and (buf[4096 + n:] == 777).all())
            key = (c, o, k, dil, s, p)
            worst[key] = max(worst.get(key, 0), err)
            if err > 3e-3 or not guard:
                bad += 1
                print("BAD", key, (bo, bt, bc), "t", t, "b", b, "err", err, "guard", guard)
torch.cuda.synchronize()
for k_, v in worst.items():
    print("worst rel err", k_, f"{v:.2e}")
# the public op too, incl. non-contiguous input
x = torch.randn(2, 300, 128, device="cuda", dtype=torch.float16).transpose(1, 2)
w = torch.randn(64, 128, 3, device="cuda").half()
y = K.conv1d(x, w.permute(2, 0, 1).contiguous(), None, 3, 1, 1, 1)
print("noncontig op err", (y.float() - F.conv1d(x.float(), w.float(), None, 1, 1, 1)).abs().max().item())
print("BAD total", bad)
