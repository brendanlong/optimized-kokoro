"""The frame side, batch-first: the F0/N predictor and a Vocos-style decoder that use only local
operations, so a batch of sentences is one padded tensor and each sentence's output is what it
would be alone (up to the last frame or two, see Decoder).

Distilled from Kokoro (brendanlong/kokoro, experiments/batch_first and experiments/vocos):

- F0/N predictor: Kokoro's AdainResBlk1d blocks and weights, but each AdaIN normalizes each frame
  over its channels (a layer norm) instead of each channel over the sentence. The style comes from
  each sentence's own vector. Padded frames are zeroed before every convolution, as a sentence
  alone would see its convolutions' zero padding there.
- Decoder: ConvNeXt blocks at 80 frames per second (300 samples per frame, the rate of F0 and N)
  and an inverse STFT (n_fft 1200, hop 300), in place of Kokoro's 300-fold upsampling generator.
  Its inputs are Kokoro's decoder inputs and the STFT of Kokoro's harmonic source; each norm is
  conditioned on the style and the frame's F0 and N.

Convolutions with weights shared across channels go through kernels.py's Triton conv1d; the
depthwise ones are sums of shifted copies (convseq.DepthwiseConv1d); the STFTs are matmuls.
"""

import math

import torch
import torch.nn as nn
import torch.nn.functional as F

from .convseq import DepthwiseConv1d
from .kernels import MatmulSTFT

HOP = 300  # samples per decoder frame: 80 per second, two per Kokoro frame
N_FFT = 4 * HOP
BINS = N_FFT // 2 + 1
FRAME_FEATURES = 3  # log F0 (0 when unvoiced), voiced, N
DIM, LAYERS, MULT, STYLE = 512, 8, 3, 128


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


class Decoder(nn.Module):
    """The Vocos-style decoder. `source` is Kokoro's SourceModuleHnNSF (sines at F0 and 8
    overtones plus noise, mixed by one linear layer); keep it and `stft` in fp32.

    Batched by padding, with each sentence's output what it would be alone: past its end, its
    harmonic source is reflected (as the STFT's padding of a lone sentence is), its last frame's
    features repeated (as a lone sentence's extra STFT frame gets them), its frames masked, and its
    inverse STFT normalized over its own frames. The network reaches about 31 frames (0.4 s) to
    either side, so without these a padded sentence's last 0.2 s or so came out audibly different."""

    def __init__(self, source: nn.Module, asr_dim: int = 512):
        super().__init__()
        cond_dim = STYLE + FRAME_FEATURES
        self.source = source
        self.embed = nn.Conv1d(asr_dim + FRAME_FEATURES, DIM, 7, padding=3)
        self.embed_source = nn.Linear(3 * BINS, DIM)
        self.norm = AdaLN(DIM, cond_dim)
        self.blocks = nn.ModuleList(ConvNeXtBlock(DIM, cond_dim) for _ in range(LAYERS))
        self.final = AdaLN(DIM, cond_dim)
        self.head = nn.Linear(DIM, 2 * BINS)
        self.stft = MatmulSTFT(N_FFT, HOP)

    @torch.autocast("cuda", enabled=False)
    def harmonic_source(self, f0, lengths):
        """f0 [B, 2T] (Hz) -> [B, 600 T] samples, reflected past each sentence's end."""
        source = self.source(f0.float()[..., None])[..., 0]
        t = torch.arange(source.shape[1], device=f0.device)[None, :]
        end = (lengths * 2 * HOP)[:, None]
        return torch.gather(source, 1, torch.where(t < end, t, 2 * (end - 1) - t).clamp(min=0))

    def forward(self, asr, f0, n, s, lengths):
        """asr [B, 512, T] (40 frames per second, zero at padding), F0 and N [B, 2T] (zero at
        padding), s [B, 128], lengths [B] in 40-per-second frames -> audio [B, 1, 600 T]."""
        steps = torch.arange(f0.shape[1] + 1, device=f0.device)[None, :]  # the STFT's 2T + 1 frames
        kept = steps <= 2 * lengths[:, None]  # [B, 2T + 1]
        keep = kept[..., None]
        mag, phase = self.stft.transform(self.harmonic_source(f0, lengths))  # [B, BINS, 2T + 1]
        src = torch.cat([torch.log(mag + 1e-5), torch.cos(phase), torch.sin(phase)], dim=1)

        voiced = (f0 > 10).to(asr.dtype)
        feats = torch.stack([torch.log(f0.float().clamp(min=10)).to(asr.dtype) * voiced, voiced, n.to(asr.dtype)], 1)
        # Frame 2 * length (the STFT's extra one) gets the sentence's last frame's features.
        last = torch.minimum(steps, 2 * lengths[:, None] - 1)[:, None, :]
        feats = torch.gather(feats, 2, last.expand(-1, feats.shape[1], -1))  # [B, 3, 2T + 1]
        asr2 = asr.repeat_interleave(2, dim=2)
        asr2 = torch.gather(asr2, 2, last.expand(-1, asr2.shape[1], -1))
        k = keep.transpose(1, 2).to(asr.dtype)
        x = self.embed(torch.cat([asr2, feats], dim=1) * k).transpose(1, 2)
        x = x + self.embed_source(src.transpose(1, 2).to(x.dtype))
        feats = feats.transpose(1, 2)
        cond = torch.cat([s[:, None, :].expand(-1, feats.shape[1], -1).to(feats.dtype), feats], dim=-1)
        keep = keep.to(x.dtype)
        x = self.norm(x, cond)
        for block in self.blocks:
            x = block(x, cond, keep)
        x = self.head(self.final(x, cond)).transpose(1, 2).float()  # [B, 2 * BINS, 2T + 1]
        mag = torch.exp(x[:, :BINS].clamp(max=math.log(100.0))) * kept[:, None, :]
        return self.stft.inverse(mag, x[:, BINS:], kept)
