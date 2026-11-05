"""Packed decoder rows against each sentence alone, with the excitation noise off, in fp32 eager.
Durations must match, and the spectrograms should match closely (correlation near 1). Samples can
still differ: the generator magnifies rounding in the harmonic phase. The alone path's phase sum is
switched to float64 like the packed one's.

    uv run python bench/check_packing.py MODELS [--device cuda] [--n 12]
"""

import argparse
import json
import os
import sys

import numpy as np
import torch
import torch.nn.functional as F

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from check_equivalence import spec_corr  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402


def results(synth, reqs):
    out = {r.index: r for group in synth.run(reqs) for r in group}
    return [out[i] for i in range(len(reqs))]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--n", type=int, default=12)
    args = ap.parse_args()
    torch.randn_like = torch.zeros_like  # the SineGen excitation noise, alone and packed
    synth = Synth(args.models, device=args.device, half=False, compile=False)
    from kokoro.istftnet import SineGen  # importable once Synth has set up the kokoro package

    def f02sine(self, f0_values):
        rad = (f0_values / self.sampling_rate) % 1
        phase = (torch.cumsum(rad.double(), dim=1).float() * 2 * torch.pi).transpose(1, 2)
        phase = F.interpolate(phase * self.upsample_scale, scale_factor=self.upsample_scale, mode="linear")
        return torch.sin(phase.transpose(1, 2))

    SineGen._f02sine = f02sine
    vocab = json.load(open(os.path.join(args.models, "config.json"), encoding="utf-8"))["vocab"]
    chunks = sentences.load(vocab)
    rng = np.random.default_rng(0)
    picks = [chunks[i][1] for i in rng.choice(len(chunks), args.n, replace=False)]
    voices = [synth.voice_names.index(v) for v in ("af_heart", "am_adam", "bf_emma")]
    reqs = [Request(ids, synth.style(voices[k % 3], len(ids)), (100, 80, 130)[k % 3]) for k, ids in enumerate(picks)]

    alone = [results(synth, [r])[0] for r in reqs]
    packed = results(synth, reqs)
    corrs = []
    for a, p in zip(alone, packed):
        assert a.durations == p.durations, "durations differ"
        x, y = (np.frombuffer(r.pcm, dtype="<i2").astype(np.float32) / 32767 for r in (a, p))
        corrs.append(spec_corr(x, y))
        print(f"{len(a.durations) - 2:3d} ids, {len(x) / 24000:5.2f} s: spectrogram corr {corrs[-1]:.5f}, "
              f"max |diff| {np.abs(x - y).max():.3f} (peak {np.abs(x).max():.3f})")
    print(f"spectrogram corr over {len(reqs)} sentences: mean {np.mean(corrs):.5f} min {np.min(corrs):.5f}")


if __name__ == "__main__":
    main()
