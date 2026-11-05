"""How much Kokoro's instance-norm statistics vary between sentences, and how the audio changes when
they are frozen (each voice's average) instead of taken from each sentence.

    uv run python bench/norm_stats.py MODELS [--per-voice 40] [--test 16] [--clips DIR] [--json out.json]

Every InstanceNorm1d (inside the 70 AdaINs of the F0/N predictor, decoder and generator) normalizes
each channel by the sentence's own mean and standard deviation over time. For each norm this prints
how much the mean moves between sentences, relative to the standard deviation (a mean shift of 0.1
standard deviations is small), and how much the standard deviation itself moves (its coefficient of
variation), within one voice and over all of them. Then it renders held-out sentences with the
frozen statistics, in all norms or only the later ones, and compares their spectrograms with the
normal render (same seed), next to the seed-to-seed noise floor.
"""

import argparse
import json
import os
import random
import statistics
import sys

import numpy as np
import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from check_equivalence import spec_corr  # noqa: E402
from kokoro_worker.client import write_wav  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402

VOICES = ["af_heart", "af_bella", "am_adam", "am_michael", "bf_emma", "bm_george"]


class Norms:
    """Wraps every InstanceNorm1d: records per-sentence statistics, or applies frozen ones."""

    def __init__(self, model):
        self.mods = [m for m in model.modules() if isinstance(m, torch.nn.InstanceNorm1d)]
        self.mode, self.frozen, self.records = "normal", None, None
        for i, m in enumerate(self.mods):
            m.forward = self._forward(i, m)

    def _forward(self, i, mod):
        def forward(x):
            x32 = x.float()
            mean = x32.mean(-1, keepdim=True)
            var = x32.var(-1, unbiased=False, keepdim=True)
            if self.mode == "record":
                self.records[i].append((mean[0, :, 0].cpu(), var[0, :, 0].sqrt().cpu()))
            elif self.mode == "frozen" and self.frozen[i] is not None:
                fm, fs = self.frozen[i]
                mean, var = fm.to(x.device)[None, :, None], (fs.to(x.device) ** 2)[None, :, None]
            y = (x32 - mean) * torch.rsqrt(var + mod.eps)
            if mod.affine:
                y = y * mod.weight[None, :, None] + mod.bias[None, :, None]
            return y.to(x.dtype)

        return forward


def render(synth, req, seed):
    torch.manual_seed(seed)
    out = next(iter(synth.run([req])))[0]
    return np.frombuffer(out.pcm, dtype="<i2").astype(np.float32) / 32767


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--per-voice", type=int, default=40, help="sentences per voice to measure")
    ap.add_argument("--test", type=int, default=16, help="held-out sentences per voice to render")
    ap.add_argument("--clips")
    ap.add_argument("--json")
    args = ap.parse_args()

    synth = Synth(args.models, device="cuda", half=False, compile=False)
    norms = Norms(synth.model)
    vocab = json.load(open(os.path.join(args.models, "config.json"), encoding="utf-8"))["vocab"]
    corpus = [ids for _, ids in sentences.load(vocab)]
    rng = random.Random(0)
    rng.shuffle(corpus)
    measure, test = corpus[: args.per_voice], corpus[args.per_voice : args.per_voice + args.test]
    voices = [synth.voice_names.index(v) for v in VOICES]
    print(f"{len(norms.mods)} instance norms; {len(measure)} sentences x {len(voices)} voices measured, "
          f"{len(test)} held out", flush=True)

    # 1. Statistics per sentence.
    per_voice = {}
    norms.mode = "record"
    for v, name in zip(voices, VOICES):
        norms.records = [[] for _ in norms.mods]
        for ids in measure:
            render(synth, Request(ids, synth.style(v, len(ids)), 100), 0)
        per_voice[name] = [(torch.stack([m for m, _ in r]), torch.stack([s for _, s in r])) for r in norms.records]
    norms.mode = "normal"

    def spread(means, stds):
        """Median over channels of std_k(mean)/mean_k(std), and of std_k(std)/mean_k(std)."""
        scale = stds.mean(0).clamp_min(1e-6)
        return float((means.std(0) / scale).median()), float((stds.std(0) / scale).median())

    rows = []
    for i in range(len(norms.mods)):
        within = [spread(*per_voice[n][i]) for n in VOICES]
        pooled = spread(torch.cat([per_voice[n][i][0] for n in VOICES]), torch.cat([per_voice[n][i][1] for n in VOICES]))
        rows.append({"norm": i, "channels": norms.mods[i].num_features,
                     "mean_shift_within_voice": round(statistics.median(w[0] for w in within), 4),
                     "std_cv_within_voice": round(statistics.median(w[1] for w in within), 4),
                     "mean_shift_all": round(pooled[0], 4), "std_cv_all": round(pooled[1], 4)})
    print("norm  ch  mean shift (in std) within voice / all   std variation within voice / all")
    for r in rows:
        print(f"{r['norm']:3d} {r['channels']:5d}   {r['mean_shift_within_voice']:.3f} / {r['mean_shift_all']:.3f}"
              f"   {r['std_cv_within_voice']:.3f} / {r['std_cv_all']:.3f}")

    # 2. Frozen statistics (each voice's average) on held-out sentences, for all norms and for the
    # later ones only. Norms 0-11 are the F0/N predictor's, 12-21 the decoder's, 22-69 the generator's.
    frozen_voice = {n: [(m.mean(0), s.mean(0)) for m, s in per_voice[n]] for n in VOICES}
    subsets = {"all": 0, "decoder_and_generator": 12, "generator": 22}
    results = {"floor": [], **{k: [] for k in subsets}}
    for v, name in zip(voices, VOICES):
        for k, ids in enumerate(test):
            req = Request(ids, synth.style(v, len(ids)), 100)
            norms.mode = "normal"
            normal, other_seed = render(synth, req, 1), render(synth, req, 2)
            results["floor"].append(spec_corr(normal, other_seed))
            clips = {"normal": normal}
            norms.mode = "frozen"
            for label, first in subsets.items():
                norms.frozen = [f if i >= first else None for i, f in enumerate(frozen_voice[name])]
                clips[label] = render(synth, req, 1)
                results[label].append(spec_corr(normal, clips[label]))
            norms.mode = "normal"
            if args.clips and k < 3:
                os.makedirs(args.clips, exist_ok=True)
                for label, audio in clips.items():
                    write_wav(os.path.join(args.clips, f"{name}-{k}-{label}.wav"),
                              (np.clip(audio, -1, 1) * 32767).astype("<i2").tobytes())
    for label, corrs in results.items():
        print(f"{label}: spectrogram corr vs normal mean {np.mean(corrs):.4f} min {np.min(corrs):.4f}", flush=True)
    if args.json:
        with open(args.json, "w") as f:
            json.dump({"norms": rows, "corr": results}, f, indent=1)


if __name__ == "__main__":
    main()
