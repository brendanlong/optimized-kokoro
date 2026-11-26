"""What a batch-first frame side would cost, before training one: Kokoro's F0/N predictor and
decoder with per-frame normalization in place of per-sentence instance norm, against the worker's
packed versions (masked.py). Same layers and weights, so the same compute apart from the norms;
the batch-first audio is garbage (the weights were trained with instance norm), only time counts.

Batch-first means only local operations: each AdaIN becomes a layer norm over the channels of
each frame, scaled and shifted by the frame's sentence's style, and the harmonic source is one
phase sum over the whole row (a sentence's starting phase doesn't matter). Packing then needs only
the gap masks, with no per-sentence sums.

    cd WORKER_DIR && python bench/batch_first.py MODELS VARIANT MODE OUT_JSON
    VARIANT: packed | batchfirst    MODE: compiled | eager

Times the first calls (compiling, if compiled, and kernels.py's Triton autotuning) and then
throughput over rows of 1, 4 and 8 sentences with lengths from bench/corpus.jsonl.
"""

import json
import math
import os
import random
import sys
import time

sys.path.insert(0, os.getcwd())

import torch  # noqa: E402
import torch._dynamo.utils as du  # noqa: E402
import torch.nn.functional as F  # noqa: E402

from kokoro_worker import masked  # noqa: E402
from kokoro_worker.model import Synth  # noqa: E402

MODELS, VARIANT, MODE, OUT = sys.argv[1:5]
FRAMES_PER_PHONEME = 2.7  # about what the duration predictor gives at speed 1
dev = torch.device(os.environ.get("DEVICE", "cuda"))  # DEVICE=cpu: a quick check that it runs
sync = torch.cuda.synchronize if dev.type == "cuda" else (lambda: None)


def positions(pk, s):
    """Sentence index of each position at each rate (the sentence count in gaps)."""
    seg, n = pk["frame_seg"], s.shape[0]
    return {key: F.pad(seg.repeat_interleave(r), (0, extra), value=n) for key, (r, extra) in masked.RATES.items()}


def adaln(mod, x, s, ix, m):
    """AdaIN1d with the statistics of each frame (over channels) instead of each sentence (over time)."""
    h = F.pad(mod.fc(s), (0, 0, 0, 1))[ix]  # [W, 2C], the sentence's style at each position
    gamma, beta = h.T[None].chunk(2, dim=1)
    y = F.layer_norm(x.transpose(1, 2).float(), (x.shape[1],)).transpose(1, 2)
    if mod.norm.affine:
        y = y * mod.norm.weight[None, :, None] + mod.norm.bias[None, :, None]
    return ((1 + gamma) * y + beta).to(x.dtype) * m


def resblk(blk, x, s, pk, ix, k_in, k_out):
    m_in, m_out = pk["m_" + k_in], pk["m_" + k_out]
    r = blk.actv(adaln(blk.norm1, x, s, ix[k_in], m_in))
    if blk.upsample_type != "none":
        r = blk.pool(r) * m_out
    r = blk.conv1(r) * m_out
    r = blk.actv(adaln(blk.norm2, r, s, ix[k_out], m_out))
    r = blk.conv2(r) * m_out
    return (r + blk._shortcut(x)) * math.sqrt(0.5)


def resblock1(rb, x, s, pk, ix, key):
    m = pk["m_" + key]
    for i in range(len(rb.convs1)):
        a1, a2 = rb.alpha1[i], rb.alpha2[i]
        xt = adaln(rb.adain1[i], x, s, ix[key], m)
        xt = xt + (1 / a1) * (torch.sin(a1 * xt) ** 2)
        xt = rb.convs1[i](xt) * m
        xt = adaln(rb.adain2[i], xt, s, ix[key], m)
        xt = xt + (1 / a2) * (torch.sin(a2 * xt) ** 2)
        xt = rb.convs2[i](xt) * m
        x = xt + x
    return x


def f0n_blocks(p, x, s, pk):
    ix = positions(pk, s)
    outs = []
    for blocks, proj in ((p.F0, p.F0_proj), (p.N, p.N_proj)):
        y, k_in = x, "f"
        for block in blocks:
            k_out = "2f" if block.upsample_type != "none" else k_in
            y = resblk(block, y, s, pk, ix, k_in, k_out)
            k_in = k_out
        outs.append((proj(y) * pk["m_2f"]).squeeze(1))
    return outs[0], outs[1]


def source(src, f0, pk):
    """SourceModuleHnNSF over the whole row: one phase sum, one linear interpolation."""
    gen = src.l_sin_gen
    up = gen.upsample_scale
    fn = f0.float()[0, :, None] * torch.arange(1, gen.dim + 1, device=f0.device, dtype=torch.float32)
    rad = (fn / gen.sampling_rate) % 1  # [2T, dim]
    phase = (rad.double().cumsum(0) % 1).float() * (2 * torch.pi * up)
    phase = F.interpolate(phase.T[None], scale_factor=up, mode="linear")[0].T  # [S, dim]
    sines = torch.sin(phase) * gen.sine_amp
    uv = (f0[0] > gen.voiced_threshold).float().repeat_interleave(up)[:, None]
    noise_amp = uv * gen.noise_std + (1 - uv) * gen.sine_amp / 3
    sines = sines * uv + noise_amp * torch.randn_like(sines)
    return torch.tanh(src.l_linear(sines.to(src.l_linear.weight.dtype)))[None, :, 0]


def decoder(dec, asr, f0_curve, n, s, pk):
    ix = positions(pk, s)
    mf = pk["m_f"]
    f0 = dec.F0_conv(f0_curve.unsqueeze(1)) * mf
    n = dec.N_conv(n.unsqueeze(1)) * mf
    x = resblk(dec.encode, torch.cat([asr, f0, n], dim=1), s, pk, ix, "f", "f")
    asr_res = dec.asr_res(asr) * mf
    res, k_in = True, "f"
    for block in dec.decode:
        if res:
            x = torch.cat([x, asr_res, f0, n], dim=1)
        k_out = "2f" if block.upsample_type != "none" else k_in
        x = resblk(block, x, s, pk, ix, k_in, k_out)
        k_in = k_out
        if block.upsample_type != "none":
            res = False
    g = dec.generator
    with torch.no_grad():
        har_spec, har_phase = g.stft.transform(source(g.m_source, f0_curve, pk) * pk["m_samples"])
        har = torch.cat([har_spec, har_phase], dim=1) * pk["m_h"]
    for i, key in enumerate(("20f", "h")):
        m = pk["m_" + key]
        x = F.leaky_relu(x, negative_slope=0.1)
        x_source = resblock1(g.noise_res[i], g.noise_convs[i](har) * m, s, pk, ix, key)
        x = g.ups[i](x)
        if i == g.num_upsamples - 1:
            x = g.reflection_pad(x)
        x = x * m + x_source
        xs = None
        for j in range(g.num_kernels):
            y = resblock1(g.resblocks[i * g.num_kernels + j], x, s, pk, ix, key)
            xs = y if xs is None else xs + y
        x = xs / g.num_kernels
    x = g.conv_post(F.leaky_relu(x))
    spec = torch.exp(x[:, : g.post_n_fft // 2 + 1, :]) * pk["m_h"]
    return g.stft.inverse(spec, torch.sin(x[:, g.post_n_fft // 2 + 1 :, :]))


synth = Synth(MODELS, device=str(dev), half=dev.type == "cuda", compile=False)
model = synth.model
if VARIANT == "packed":
    f0n_fn = lambda x, s, pk: masked.f0n_blocks(model.predictor, x, s, pk)  # noqa: E731
    dec_fn = lambda asr, f0, n, s, pk: masked.decoder(model.decoder, asr, f0, n, s, pk)  # noqa: E731
else:
    f0n_fn = lambda x, s, pk: f0n_blocks(model.predictor, x, s, pk)  # noqa: E731
    dec_fn = lambda asr, f0, n, s, pk: decoder(model.decoder, asr, f0, n, s, pk)  # noqa: E731
if MODE == "compiled":
    f0n_fn, dec_fn = torch.compile(f0n_fn, dynamic=True), torch.compile(dec_fn, dynamic=True)

lengths = []
with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus.jsonl"), encoding="utf-8") as f:
    for line in f:
        for chunk in json.loads(line)["chunks"]:
            lengths.append(min(2400, max(8, round(len(chunk["phonemes"]) * FRAMES_PER_PHONEME))))
rng = random.Random(0)
x_dtype = torch.float16 if dev.type == "cuda" else torch.float32
gen = torch.Generator(device=dev).manual_seed(0)


@torch.inference_mode()
def run(frames):
    """One packed row: the F0/N predictor, then the decoder on a made-up pitch curve."""
    n = len(frames)
    pk = masked.Packing(frames, dev, x_dtype)
    t = pk.tensors
    s = torch.randn(n, 256, device=dev, generator=gen) * 0.3
    with torch.autocast("cuda", dtype=torch.float16, enabled=dev.type == "cuda"):
        x = torch.randn(1, 512, pk.total, device=dev, generator=gen, dtype=x_dtype) * t["m_f"]
        f0n_fn(x, s[:, 128:], t)
        m2 = t["m_2f"][0]
        f0 = (150 + 60 * torch.rand(m2.shape, device=dev, generator=gen)) * (torch.rand(m2.shape, device=dev, generator=gen) < 0.8) * m2
        nn = torch.randn(m2.shape, device=dev, generator=gen) * m2
        asr = torch.randn(1, 512, pk.total, device=dev, generator=gen, dtype=x_dtype) * t["m_f"]
        audio = dec_fn(asr, f0, nn, s[:, :128], t)
    return pk.unpack_audio(audio)


report = {"variant": VARIANT, "mode": MODE, "first_calls": []}
for frames in ([300, 280, 260, 240], [200]):  # several sentences, then one (size 1 compiles apart)
    before = len(du.get_compilation_metrics())
    t0 = time.monotonic()
    rows = run(frames)
    sync()
    wall = time.monotonic() - t0
    graphs = [m for m in du.get_compilation_metrics()[before:] if m.co_name]
    report["first_calls"].append({
        "frames": frames, "wall_s": round(wall, 1),
        "compile_s": round(sum(m.entire_frame_compile_time_s or 0 for m in graphs), 1),
        "graphs": [{"name": m.co_name, "nodes": m.graph_node_count,
                    "compile_s": round(m.entire_frame_compile_time_s or 0, 1)} for m in graphs],
        "finite": all(bool(r.isfinite().all()) for r in rows),
    })
    print(json.dumps(report["first_calls"][-1]), flush=True)

report["throughput"] = []
for b in (1, 4, 8):
    batches = [[rng.choice(lengths) for _ in range(b)] for _ in range(30)]
    for frames in batches[:3]:
        run(frames)
    sync()
    times, audio_s = [], 0.0
    for frames in batches:
        t0 = time.perf_counter()
        run(frames)
        sync()
        times.append(time.perf_counter() - t0)
        audio_s += sum(frames) * masked.SAMPLES_PER_FRAME / 24000
    times.sort()
    r = {"sentences": b, "x_realtime": round(audio_s / sum(times), 1),
         "median_ms": round(times[len(times) // 2] * 1000, 1), "max_ms": round(times[-1] * 1000, 1)}
    report["throughput"].append(r)
    print(json.dumps(r), flush=True)
report["peak_reserved_mib"] = torch.cuda.max_memory_reserved() >> 20 if dev.type == "cuda" else None
print("peak reserved MiB", report["peak_reserved_mib"], flush=True)
with open(OUT, "w") as f:
    json.dump(report, f, indent=1)
