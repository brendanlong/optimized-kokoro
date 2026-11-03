"""Padding-aware forward passes for the frame side of Kokoro (F0/N predictor and decoder).

They call the fork's own modules (weights, convs, linears) but replace two things that let
padding leak into real frames:
- instance norm statistics are taken over each row's real frames only;
- padded positions are zeroed before every convolution, so a convolution near the end of a row
  sees the same zeros its own zero padding would supply when the row runs alone.

Masks are given per time resolution as (mask [B, 1, W] in the activation dtype, count [B, 1, 1]):
    "f"   decoder frames             F
    "2f"  after the 2x upsample      2F
    "20f" after the generator's first transposed conv          20F
    "h"   after the second, plus the reflection pad (and the harmonic STFT frames)  120F + 1
    "s"   audio samples              600F
"""

import math

import torch
import torch.nn.functional as F


def masks_for(frames: list[int], width: int, device, dtype) -> dict:
    lengths = torch.tensor(frames, device=device)
    out = {}
    for key, scale, extra in (("f", 1, 0), ("2f", 2, 0), ("20f", 20, 0), ("h", 120, 1), ("s", 600, 0)):
        pos = torch.arange(width * scale + extra, device=device)
        m = (pos[None, :] < (lengths * scale + extra)[:, None])[:, None, :]
        out[key] = (m.to(dtype), m.sum(-1, keepdim=True).float())
    return out


def _inorm(norm: torch.nn.InstanceNorm1d, x, mask):
    m, n = mask
    m = m.float()
    x = x.float()
    mean = (x * m).sum(-1, keepdim=True) / n
    centered = (x - mean) * m
    var = (centered * centered).sum(-1, keepdim=True) / n
    y = centered * torch.rsqrt(var + norm.eps)
    if norm.affine:
        y = y * norm.weight[None, :, None] + norm.bias[None, :, None]
    return y


def _adain(mod, x, s, mask):
    h = mod.fc(s)[:, :, None]
    gamma, beta = torch.chunk(h, chunks=2, dim=1)
    return ((1 + gamma) * _inorm(mod.norm, x, mask) + beta) * mask[0]


def adain_resblk(blk, x, s, m_in, m_out):
    """AdainResBlk1d; x is zero at padding, and so is the result."""
    r = blk.actv(_adain(blk.norm1, x, s, m_in))
    if blk.upsample_type != "none":
        r = blk.pool(r) * m_out[0]
    r = blk.conv1(r) * m_out[0]
    r = blk.actv(_adain(blk.norm2, r, s, m_out))
    r = blk.conv2(r) * m_out[0]
    return (r + blk._shortcut(x)) * math.sqrt(0.5)


def _resblock1(rb, x, s, mask):
    """AdaINResBlock1 (the generator's resblocks)."""
    m = mask[0]
    for i in range(len(rb.convs1)):
        a1, a2 = rb.alpha1[i], rb.alpha2[i]
        xt = _adain(rb.adain1[i], x, s, mask)
        xt = xt + (1 / a1) * (torch.sin(a1 * xt) ** 2)
        xt = rb.convs1[i](xt) * m
        xt = _adain(rb.adain2[i], xt, s, mask)
        xt = xt + (1 / a2) * (torch.sin(a2 * xt) ** 2)
        xt = rb.convs2[i](xt) * m
        x = xt + x
    return x


def f0n_blocks(p, x, s, masks):
    """ProsodyPredictor.F0Ntrain after its LSTM; x [B, C, W] zero at padding."""
    f0 = x
    n = x
    for blocks, out in ((p.F0, "f0"), (p.N, "n")):
        y = x
        m_in = masks["f"]
        for block in blocks:
            m_out = masks["2f"] if block.upsample_type != "none" else m_in
            y = adain_resblk(block, y, s, m_in, m_out)
            m_in = m_out
        if out == "f0":
            f0 = y
        else:
            n = y
    f0 = p.F0_proj(f0) * masks["2f"][0]
    n = p.N_proj(n) * masks["2f"][0]
    return f0.squeeze(1), n.squeeze(1)


def decoder(dec, asr, f0_curve, n, s, masks):
    """Decoder.forward; asr [B, 512, W], f0_curve and n [B, 2W], all zero at padding."""
    mf = masks["f"]
    f0 = dec.F0_conv(f0_curve.unsqueeze(1)) * mf[0]
    n = dec.N_conv(n.unsqueeze(1)) * mf[0]
    x = torch.cat([asr, f0, n], dim=1)
    x = adain_resblk(dec.encode, x, s, mf, mf)
    asr_res = dec.asr_res(asr) * mf[0]
    res = True
    m_in = mf
    for block in dec.decode:
        if res:
            x = torch.cat([x, asr_res, f0, n], dim=1)
        m_out = masks["2f"] if block.upsample_type != "none" else m_in
        x = adain_resblk(block, x, s, m_in, m_out)
        m_in = m_out
        if block.upsample_type != "none":
            res = False
    return _generator(dec.generator, x, s, f0_curve, masks)


def _generator(g, x, s, f0, masks):
    with torch.no_grad():
        har_source = g.m_source(f0[:, :, None]).transpose(1, 2).squeeze(1)
        har_source = har_source * masks["s"][0][:, 0, :]
        har_spec, har_phase = g.stft.transform(har_source)
        har = torch.cat([har_spec, har_phase], dim=1) * masks["h"][0]
    res_masks = (masks["20f"], masks["h"])
    for i in range(g.num_upsamples):
        mask = res_masks[i]
        x = F.leaky_relu(x, negative_slope=0.1)
        x_source = g.noise_convs[i](har) * mask[0]
        x_source = _resblock1(g.noise_res[i], x_source, s, mask)
        x = g.ups[i](x)
        if i == g.num_upsamples - 1:
            x = g.reflection_pad(x)
        x = x * mask[0] + x_source
        xs = None
        for j in range(g.num_kernels):
            y = _resblock1(g.resblocks[i * g.num_kernels + j], x, s, mask)
            xs = y if xs is None else xs + y
        x = xs / g.num_kernels
    x = F.leaky_relu(x)
    x = g.conv_post(x)
    spec = torch.exp(x[:, : g.post_n_fft // 2 + 1, :]) * masks["h"][0]
    phase = torch.sin(x[:, g.post_n_fft // 2 + 1 :, :])
    return g.stft.inverse(spec, phase)
