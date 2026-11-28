"""The frame side, batch-first: the F0/N predictor and a Vocos-style decoder that use only local
operations, so a batch of sentences is one padded tensor and each sentence's output is what it
would be alone (up to the last frame or two, see Decoder).

Distilled from Kokoro (brendanlong/kokoro, experiments/batch_first and experiments/vocos):

- F0/N predictor: Kokoro's AdainResBlk1d blocks and weights, but each AdaIN normalizes each frame
  over its channels (a layer norm) instead of each channel over the sentence. The style comes from
  each sentence's own vector. Padded frames are zeroed before every convolution, as a sentence
  alone would see its convolutions' zero padding there.
- Decoder: ConvNeXt blocks at 80 frames per second (300 samples per frame, the rate of F0 and N)
  and an inverse STFT, in place of Kokoro's 300-fold upsampling generator. Each frame's output is
  three short spectra (n_fft 400, hop 100), each the sum of a spectrum of its own and two source
  spectra it filters (a gain and a phase shift per bin): Kokoro's harmonic source, and every
  harmonic of F0 up to 11.5 kHz. Its inputs are Kokoro's decoder inputs and the STFTs of both
  sources; each norm is conditioned on the style and the frame's F0 and N.

Convolutions with weights shared across channels go through kernels.py's Triton conv1d; the
depthwise ones are sums of shifted copies (convseq.DepthwiseConv1d); the STFTs are matmuls.
"""

import math

import torch
import torch.nn as nn
import torch.nn.functional as F

from .convseq import DepthwiseConv1d
from .kernels import MatmulSTFT

SAMPLE_RATE = 24000
HOP = 300  # samples per decoder frame: 80 per second, two per Kokoro frame
SUB = 3  # spectra per decoder frame
STFT_HOP = HOP // SUB
N_FFT = 4 * STFT_HOP
BINS = N_FFT // 2 + 1
FRAME_FEATURES = 3  # log F0 (0 when unvoiced), voiced, N
DIM, LAYERS, MULT, STYLE = 512, 8, 3, 128
SOURCES = 2  # Kokoro's harmonic source and the full-band one
FULL_BAND_HZ = 11500  # the full-band source's harmonics stop here
MAX_GAIN, MAX_MAGNITUDE = 1e4, 400.0
# The distilled checkpoint's VocosDecoder configuration (brendanlong/kokoro experiments/vocos), which
# this decoder is; convert.py checks it.
CONFIG = {"dim": DIM, "layers": LAYERS, "mult": MULT, "head": "both", "full_band": True, "sub": SUB, "noise": False}


def _adaln(mod, x, s):
    """Kokoro's AdaIN1d weights (mod.norm's affine, mod.fc) applied with per-frame statistics.
    x [B, C, T], s [B, style]."""
    gamma, beta = mod.fc(s)[:, :, None].chunk(2, dim=1)
    y = F.layer_norm(x.transpose(1, 2).float(), (x.shape[1],)).transpose(1, 2)
    if mod.norm.affine:
        y = y * mod.norm.weight[:, None].float() + mod.norm.bias[:, None].float()
    return ((1 + gamma) * y + beta).to(x.dtype)


def _resblk(blk, x, s, m_in, m_out):
    """AdainResBlk1d; x is zero at padding, and so is the result."""
    r = blk.actv(_adaln(blk.norm1, x, s)) * m_in
    if blk.upsample_type != "none":
        r = blk.pool(r) * m_out
    r = blk.conv1(r) * m_out
    r = blk.actv(_adaln(blk.norm2, r, s)) * m_out
    r = blk.conv2(r) * m_out
    return (r + blk._shortcut(x)) * math.sqrt(0.5)


def f0n(predictor, x, s, m_f, m_2f):
    """ProsodyPredictor's F0 and N branches after its shared sequence model.
    x [B, 512, T] zero at padding, s [B, 128], m_f [B, 1, T] and m_2f [B, 1, 2T] the real frames
    -> F0 and N [B, 2T], zero at padding."""
    outs = []
    for blocks, proj in ((predictor.F0, predictor.F0_proj), (predictor.N, predictor.N_proj)):
        y, m = x, m_f
        for block in blocks:
            m_out = m_2f if block.upsample_type != "none" else m
            y = _resblk(block, y, s, m, m_out)
            m = m_out
        outs.append((proj(y) * m_2f).squeeze(1))
    return outs[0], outs[1]


class AdaLN(nn.Module):
    """Layer norm over a frame's channels, scaled and shifted by that frame's conditioning."""

    def __init__(self, dim: int, cond_dim: int):
        super().__init__()
        self.fc = nn.Linear(cond_dim, 2 * dim)

    def forward(self, x, cond):  # x [B, T, C], cond [B, T, cond_dim]
        gamma, beta = self.fc(cond).chunk(2, dim=-1)
        return (F.layer_norm(x.float(), (x.shape[-1],)) * (1 + gamma) + beta).to(x.dtype)


class ConvNeXtBlock(nn.Module):
    def __init__(self, dim: int, cond_dim: int):
        super().__init__()
        self.dw = DepthwiseConv1d(dim, 7, 1)
        self.norm = AdaLN(dim, cond_dim)
        self.pw1 = nn.Linear(dim, MULT * dim)
        self.pw2 = nn.Linear(MULT * dim, dim)
        self.scale = nn.Parameter(torch.full((dim,), 1 / LAYERS))

    def forward(self, x, cond, keep):  # x [B, T, C], keep [B, T, 1]
        h = self.dw((x * keep).transpose(1, 2)).transpose(1, 2)
        return x + self.scale * self.pw2(F.gelu(self.pw1(self.norm(h, cond))))


def full_band_source(f0):
    """Every harmonic of F0 up to FULL_BAND_HZ, in phase with the fundamental of Kokoro's source.

    f0 [B, F] (Hz, 80 per second) -> [B, 300 F] samples: the sum of sin(k phase) over every k with
    k F0 under FULL_BAND_HZ, where phase is the fundamental's as Kokoro's SineGen computes it (a
    running sum of F0 per frame, interpolated to samples). The sum has a closed form, so the cost
    doesn't grow with the number of harmonics. Zero where unvoiced, and past a padded sentence's
    end (where F0 is zero and the running sum stops)."""
    f0 = f0.double()
    cycles = ((f0 / SAMPLE_RATE) % 1).cumsum(1)
    cycles = F.interpolate(cycles[:, None] * HOP, scale_factor=HOP, mode="linear")[:, 0]
    half = math.pi * (cycles % 1)  # half the fundamental's phase
    f0 = f0.repeat_interleave(HOP, dim=1)
    voiced = f0 > 10
    k = torch.floor(FULL_BAND_HZ / f0.clamp(min=10)) * voiced
    denom = torch.sin(half)
    safe = denom.abs() > 1e-6
    total = torch.sin(k * half) * torch.sin((k + 1) * half) / torch.where(safe, denom, torch.ones_like(denom))
    return (0.1 * total * safe / k.clamp(min=1).sqrt()).float()


def _per_frame(z):
    """[B, C, bins, frames * SUB] -> [B, C * SUB * bins, frames]: a frame's spectra side by side."""
    b, c, bins, n = z.shape
    return z.reshape(b, c, bins, n // SUB, SUB).permute(0, 1, 4, 2, 3).reshape(b, -1, n // SUB)


def _per_spectrum(x):
    """_per_frame's inverse for one channel: [B, SUB * bins, frames] -> [B, bins, frames * SUB]."""
    b, _, n = x.shape
    return x.reshape(b, SUB, BINS, n).permute(0, 2, 3, 1).reshape(b, BINS, n * SUB)


class Decoder(nn.Module):
    """The Vocos-style decoder. `source` is Kokoro's SourceModuleHnNSF (sines at F0 and 8
    overtones plus noise, mixed by one linear layer); keep it and `stft` in fp32.

    Batched by padding, with each sentence's output what it would be alone: past its end, both
    harmonic sources are reflected (as the STFT's padding of a lone sentence is), its frames are
    masked, the spectrum at its very end (which the network doesn't make) is zero, and its inverse
    STFT is normalized over its own spectra. The network reaches about 30 frames (0.4 s) to either
    side, so without these a padded sentence's last 0.2 s or so came out audibly different."""

    def __init__(self, source: nn.Module, asr_dim: int = 512):
        super().__init__()
        cond_dim = STYLE + FRAME_FEATURES
        per_frame = SUB * BINS
        self.source = source
        self.embed = nn.Conv1d(asr_dim + FRAME_FEATURES, DIM, 7, padding=3)
        self.embed_source = nn.Linear(3 * per_frame * SOURCES, DIM)
        self.norm = AdaLN(DIM, cond_dim)
        self.blocks = nn.ModuleList(ConvNeXtBlock(DIM, cond_dim) for _ in range(LAYERS))
        self.final = AdaLN(DIM, cond_dim)
        self.head = nn.Linear(DIM, 2 * per_frame * (SOURCES + 1))
        self.stft = MatmulSTFT(N_FFT, STFT_HOP)

    @torch.autocast("cuda", enabled=False)
    def harmonic_sources(self, f0, lengths):
        """f0 [B, 2T] (Hz) -> Kokoro's source and the full-band one, each [B, 600 T] samples,
        reflected past each sentence's end."""
        f0 = f0.float()
        t = torch.arange(f0.shape[1] * HOP, device=f0.device)[None, :]
        end = (lengths * 2 * HOP)[:, None]
        reflect = torch.where(t < end, t, 2 * (end - 1) - t).clamp(min=0)
        return [torch.gather(x, 1, reflect) for x in (self.source(f0[..., None])[..., 0], full_band_source(f0))]

    def forward(self, asr, f0, n, s, lengths):
        """asr [B, 512, T] (40 frames per second, zero at padding), F0 and N [B, 2T] (zero at
        padding), s [B, 128], lengths [B] in 40-per-second frames -> audio [B, 1, 600 T]."""
        frames = f0.shape[1]  # 2T; the STFTs have SUB * 2T + 1 spectra, the last left zero
        keep = (torch.arange(frames, device=f0.device)[None, :] < 2 * lengths[:, None])[..., None]  # [B, 2T, 1]
        specs = [[z[..., :-1] for z in self.stft.spectrum(x)] for x in self.harmonic_sources(f0, lengths)]
        feats = []
        for re, im in specs:
            angle = torch.atan2(im, re)
            feats += [torch.log(torch.sqrt(re * re + im * im) + 1e-5), torch.cos(angle), torch.sin(angle)]
        src = _per_frame(torch.stack(feats, dim=1))  # [B, 3 * SOURCES * SUB * BINS, 2T]

        voiced = (f0 > 10).to(asr.dtype)
        frame = torch.stack([torch.log(f0.float().clamp(min=10)).to(asr.dtype) * voiced, voiced, n.to(asr.dtype)], 1)
        k = keep.transpose(1, 2).to(asr.dtype)
        x = self.embed(torch.cat([asr.repeat_interleave(2, dim=2), frame], dim=1) * k).transpose(1, 2)
        x = x + self.embed_source(src.transpose(1, 2).to(x.dtype))
        frame = frame.transpose(1, 2)
        cond = torch.cat([s[:, None, :].expand(-1, frame.shape[1], -1).to(frame.dtype), frame], dim=-1)
        keep = keep.to(x.dtype)
        x = self.norm(x, cond)
        for block in self.blocks:
            x = block(x, cond, keep)
        x = self.head(self.final(x, cond)).transpose(1, 2).float()  # [B, 2 * (SOURCES + 1) * SUB * BINS, 2T]
        # Per output, [B, BINS, SUB * 2T]: (gain, shift) for each source, then (log magnitude, phase).
        outs = [_per_spectrum(o) for o in x.chunk(2 * (SOURCES + 1), dim=1)]
        re = im = 0
        for i, (sr, si) in enumerate(specs):
            gain = torch.exp(outs[2 * i].clamp(max=math.log(MAX_GAIN)))
            c, sn = gain * torch.cos(outs[2 * i + 1]), gain * torch.sin(outs[2 * i + 1])
            re, im = re + c * sr - sn * si, im + c * si + sn * sr
        mag = torch.exp(outs[-2].clamp(max=math.log(MAX_MAGNITUDE)))
        re, im = re + mag * torch.cos(outs[-1]), im + mag * torch.sin(outs[-1])
        total = torch.sqrt(re * re + im * im)
        scale = total.clamp(max=MAX_MAGNITUDE) / (total + 1e-9)
        # Each sentence's spectra, then a zero one at its end; the inverse counts those and no more.
        step = torch.arange(frames * SUB + 1, device=f0.device)[None, :]
        made = (step < SUB * 2 * lengths[:, None]).float()[:, None, :]
        scale = F.pad(scale, (0, 1)) * made
        re, im = F.pad(re, (0, 1)) * scale, F.pad(im, (0, 1)) * scale
        return self.stft.synthesize(re, im, step <= SUB * 2 * lengths[:, None])
