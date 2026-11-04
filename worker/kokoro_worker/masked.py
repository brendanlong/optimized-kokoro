"""The frame side of Kokoro (F0/N predictor and decoder) for several sentences packed into one row.

The sentences go end to end with GAP frames of zeros between them, so the convolutions run once
over one long row instead of over a padded batch. Every convolution here reaches less than GAP
frames to either side, and the gaps are zeroed again before each one, so a convolution at the edge
of a sentence sees the zeros its own zero padding would supply if the sentence ran alone. What
isn't local is done per sentence:
- instance norm statistics, and the style (AdaIN's gamma and beta), per segment;
- the harmonic source's phase sum restarts at each segment, and its upsampling works in each
  segment's own sample positions and clamps at its edges.
With the excitation noise off, each segment's audio is then its audio alone, up to float rounding,
except within about 10 samples (0.4 ms) of its edges: there the harmonic STFT and the inverse STFT
see the gap's zeros instead of the reflect padding and window normalization of a row's ends, and
the generator's one-sample reflection pad is a zero. Edges are near-silent anyway.
The generator magnifies rounding in the harmonic phase (a 1e-3 change in the source moves samples
by about 0.06), so "up to float rounding" is judged on the spectrogram, not sample by sample.

Rates, as samples per frame (and an extra sample per sentence); audio is 600 samples per frame:
    "f"   decoder frames             1
    "2f"  after the 2x upsample      2
    "20f" after the generator's first transposed conv          20
    "h"   after the second, plus the reflection pad (and the harmonic STFT frames)  120 (+1)
"""

import math

import torch
import torch.nn.functional as F

# Frames of zeros between sentences. The widest reach is the generator's kernel-11, dilation-5
# convolutions at 20 samples per frame: 25 samples, 1.25 frames.
GAP = 2
RATES = {"f": (1, 0), "2f": (2, 0), "20f": (20, 0), "h": (120, 1)}


class Packing:
    """Where each sentence sits in the packed row. `tensors` holds everything the packed forward
    needs, as tensors only, so a compiled forward isn't specialized on the lengths."""

    def __init__(self, frames: list[int], device, dtype):
        self.frames = frames
        n = len(frames)
        lengths = torch.tensor(frames)
        span = lengths + GAP
        starts = F.pad(span.cumsum(0)[:-1], (1, 0))
        self.starts = starts.tolist()
        total = int(span.sum())  # the row ends with a gap
        frame = torch.arange(total)
        seg = torch.repeat_interleave(torch.arange(n), span)
        offset = frame - starts[seg]
        real = offset < lengths[seg]
        first_gap = offset == lengths[seg]
        self.total = total
        self._seg, self._offset, self._real = seg, offset, real
        dev = dict(device=device, non_blocking=True)
        t = {
            "seg_start": starts.to(**dev),
            # Exclusive, in frames. The first gap frame after a sentence counts as the sentence's:
            # at rate "h" its first sample is the sentence's extra one.
            "seg_end": (starts + lengths + 1).to(**dev),
            "frame_seg": torch.where(real | first_gap, seg, n).to(**dev),
            "real": real.to(**dev),
            "first_gap": first_gap.to(**dev),
            "count_base": lengths.float().to(**dev),
        }
        for key, (r, extra) in RATES.items():
            m = t["real"].repeat_interleave(r)
            if extra:
                m = m | (t["first_gap"].repeat_interleave(r) & (torch.arange(r, device=device) == 0).repeat(total))
                m = F.pad(m, (0, extra))
            t["m_" + key] = m[None, None, :].to(dtype)
            t["count_" + key] = t["count_base"] * r + extra
        t["m_samples"] = t["real"].repeat_interleave(600).to(torch.float32)[None]
        for unused in ("real", "first_gap", "count_base"):  # only for building the others
            del t[unused]
        self.tensors = t

    def pack(self, x: torch.Tensor, width: int) -> torch.Tensor:
        """[n, C, width] padded rows (frame rate) -> [1, C, total], zero in the gaps."""
        n, c = x.shape[0], x.shape[1]
        flat = F.pad(x.transpose(0, 1).reshape(c, -1), (0, 1))  # column n * width is zero
        idx = torch.where(self._real, self._seg * width + self._offset, n * width)
        return flat[:, idx.to(x.device, non_blocking=True)][None]

    def unpack_audio(self, audio: torch.Tensor) -> list[torch.Tensor]:
        """[1, 1, 600 * total] -> each sentence's samples."""
        return [audio[0, 0, s * 600 : (s + f) * 600] for s, f in zip(self.starts, self.frames)]


def _segment_sum(pk, key, x):
    """x [1, C, W] (zero outside segments) -> [1, C, n] sums per segment."""
    r = RATES[key][0]
    p = pk["frame_seg"].shape[0]
    per_frame = x[..., : r * p].reshape(x.shape[0], x.shape[1], p, r).sum(-1)
    cs = F.pad(per_frame.cumsum(-1), (1, 0))
    return cs[..., pk["seg_end"]] - cs[..., pk["seg_start"]]


def _expand(pk, key, v):
    """[1, C, n] per segment -> [1, C, W] per position: per frame, then broadcast over the frame's
    samples (a gather per frame, not per sample). Gap values are whatever; callers mask them."""
    r, extra = RATES[key]
    per_frame = F.pad(v, (0, 1))[..., pk["frame_seg"]]  # [1, C, P]
    out = per_frame[..., None].expand(*per_frame.shape, r).reshape(v.shape[0], v.shape[1], -1)
    return F.pad(out, (0, extra)) if extra else out


def _adain(mod, x, s, pk, key):
    """AdaIN1d over each segment: instance norm statistics and style per segment."""
    h = mod.fc(s).float()  # [n, 2C]
    gamma, beta = torch.chunk(h.T[None], chunks=2, dim=1)  # [1, C, n]
    m = pk["m_" + key].float()
    count = pk["count_" + key]
    x = x.float() * m
    mean = _segment_sum(pk, key, x) / count
    centered = (x - _expand(pk, key, mean)) * m
    var = _segment_sum(pk, key, centered * centered) / count
    scale = torch.rsqrt(var + mod.norm.eps) * (1 + gamma)
    shift = beta
    if mod.norm.affine:
        shift = shift + (1 + gamma) * mod.norm.bias[None, :, None]
        scale = scale * mod.norm.weight[None, :, None]
    return (centered * _expand(pk, key, scale) + _expand(pk, key, shift)) * m


def adain_resblk(blk, x, s, pk, k_in, k_out):
    """AdainResBlk1d; x is zero in the gaps, and so is the result."""
    m_out = pk["m_" + k_out]
    r = blk.actv(_adain(blk.norm1, x, s, pk, k_in))
    if blk.upsample_type != "none":
        r = blk.pool(r) * m_out
    r = blk.conv1(r) * m_out
    r = blk.actv(_adain(blk.norm2, r, s, pk, k_out))
    r = blk.conv2(r) * m_out
    return (r + blk._shortcut(x)) * math.sqrt(0.5)


def _resblock1(rb, x, s, pk, key):
    """AdaINResBlock1 (the generator's resblocks)."""
    m = pk["m_" + key]
    for i in range(len(rb.convs1)):
        a1, a2 = rb.alpha1[i], rb.alpha2[i]
        xt = _adain(rb.adain1[i], x, s, pk, key)
        xt = xt + (1 / a1) * (torch.sin(a1 * xt) ** 2)
        xt = rb.convs1[i](xt) * m
        xt = _adain(rb.adain2[i], xt, s, pk, key)
        xt = xt + (1 / a2) * (torch.sin(a2 * xt) ** 2)
        xt = rb.convs2[i](xt) * m
        x = xt + x
    return x


def f0n_blocks(p, x, s, pk):
    """ProsodyPredictor.F0Ntrain after its LSTM; x [1, C, total] zero in the gaps."""
    outs = []
    for blocks, proj in ((p.F0, p.F0_proj), (p.N, p.N_proj)):
        y = x
        k_in = "f"
        for block in blocks:
            k_out = "2f" if block.upsample_type != "none" else k_in
            y = adain_resblk(block, y, s, pk, k_in, k_out)
            k_in = k_out
        outs.append((proj(y) * pk["m_2f"]).squeeze(1))
    return outs[0], outs[1]


def decoder(dec, asr, f0_curve, n, s, pk):
    """Decoder.forward; asr [1, 512, total], f0_curve and n [1, 2 * total], all zero in the gaps."""
    mf = pk["m_f"]
    f0 = dec.F0_conv(f0_curve.unsqueeze(1)) * mf
    n = dec.N_conv(n.unsqueeze(1)) * mf
    x = torch.cat([asr, f0, n], dim=1)
    x = adain_resblk(dec.encode, x, s, pk, "f", "f")
    asr_res = dec.asr_res(asr) * mf
    res = True
    k_in = "f"
    for block in dec.decode:
        if res:
            x = torch.cat([x, asr_res, f0, n], dim=1)
        k_out = "2f" if block.upsample_type != "none" else k_in
        x = adain_resblk(block, x, s, pk, k_in, k_out)
        k_in = k_out
        if block.upsample_type != "none":
            res = False
    return _generator(dec.generator, x, s, f0_curve, pk)


def _source(src, f0, pk):
    """SourceModuleHnNSF on f0 [1, 2 * total], as for each segment alone: the phase sum restarts at
    each segment, and the upsampling interpolates in each segment's own sample positions."""
    gen = src.l_sin_gen
    up = gen.upsample_scale
    fn = f0.float()[0, :, None] * torch.arange(1, gen.dim + 1, device=f0.device, dtype=torch.float32)
    rad = (fn / gen.sampling_rate) % 1  # [2T, dim]
    # In float64: the sum runs over the whole row, and the ×300 below magnifies its rounding.
    cs = F.pad(rad.double().cumsum(0), (0, 0, 1, 0))  # exclusive prefix at index i
    offset = cs[2 * pk["seg_start"]]  # [n, dim]: the sum before each segment
    phase = (cs[1:] - _expand(pk, "2f", offset.T[None])[0].T).float() * (2 * torch.pi * up)
    # F.interpolate(mode="linear", align_corners=False) by hand, with each sample's position counted
    # from its segment's start (and clamped to the segment), as it is when the segment runs alone.
    # The gaps are unvoiced, so what they get doesn't matter.
    n_samples = f0.shape[1] * up
    j = torch.arange(n_samples, device=f0.device)
    seg = pk["frame_seg"].clamp(max=pk["seg_start"].shape[0] - 1)[j // (2 * up)]
    start = 2 * pk["seg_start"][seg]
    last = 2 * (pk["seg_end"][seg] - 1) - 1  # the segment's last 2f position
    src_pos = ((2 * (j - start * up) + 1).float() / (2 * up) - 0.5).clamp(min=0)
    lo = src_pos.floor()
    w = (src_pos - lo)[:, None]
    lo = (start + lo.long()).minimum(last)
    hi = (lo + 1).minimum(last)
    sines = torch.sin(phase[lo] * (1 - w) + phase[hi] * w) * gen.sine_amp  # [S, dim]
    uv = (f0[0] > gen.voiced_threshold).float().repeat_interleave(up)[:, None]
    noise_amp = uv * gen.noise_std + (1 - uv) * gen.sine_amp / 3
    sines = sines * uv + noise_amp * torch.randn_like(sines)
    return torch.tanh(src.l_linear(sines.to(src.l_linear.weight.dtype)))[None, :, 0]  # [1, S]


def _generator(g, x, s, f0, pk):
    mh = pk["m_h"]
    with torch.no_grad():
        har_source = _source(g.m_source, f0, pk) * pk["m_samples"]
        har_spec, har_phase = g.stft.transform(har_source)
        har = torch.cat([har_spec, har_phase], dim=1) * mh
    keys = ("20f", "h")
    for i in range(g.num_upsamples):
        key = keys[i]
        m = pk["m_" + key]
        x = F.leaky_relu(x, negative_slope=0.1)
        x_source = g.noise_convs[i](har) * m
        x_source = _resblock1(g.noise_res[i], x_source, s, pk, key)
        x = g.ups[i](x)
        if i == g.num_upsamples - 1:
            x = g.reflection_pad(x)
        x = x * m + x_source
        xs = None
        for j in range(g.num_kernels):
            y = _resblock1(g.resblocks[i * g.num_kernels + j], x, s, pk, key)
            xs = y if xs is None else xs + y
        x = xs / g.num_kernels
    x = F.leaky_relu(x)
    x = g.conv_post(x)
    spec = torch.exp(x[:, : g.post_n_fft // 2 + 1, :]) * mh
    phase = torch.sin(x[:, g.post_n_fft // 2 + 1 :, :])
    return g.stft.inverse(spec, phase)
