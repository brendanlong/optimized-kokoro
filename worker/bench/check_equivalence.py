"""Checks the batched forward against the fork's own KModel.forward_with_tokens.

    uv run python bench/check_equivalence.py MODELS [--device cuda] [--half] [--compile]

1. Batch of one vs KModel, same RNG seed: should match to float precision.
2. Batched with padding vs one at a time: durations must be identical; audio is compared by
   log-spectrogram correlation, next to the seed-to-seed noise floor of unbatched audio.
3. NaN/inf count over many lengths (the m_source compile issue).
"""

import argparse
import json
import os
import sys

import numpy as np
import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402


def spec_corr(a: np.ndarray, b: np.ndarray) -> float:
    n = min(len(a), len(b))
    win = torch.hann_window(1024)
    sa = torch.stft(torch.tensor(a[:n], dtype=torch.float32), 1024, 256, window=win, return_complex=True).abs()
    sb = torch.stft(torch.tensor(b[:n], dtype=torch.float32), 1024, 256, window=win, return_complex=True).abs()
    la, lb = torch.log(sa + 1e-5).flatten(), torch.log(sb + 1e-5).flatten()
    return float(torch.corrcoef(torch.stack([la, lb]))[0, 1])


def pcm(result) -> np.ndarray:
    return np.frombuffer(result.pcm, dtype="<i2").astype(np.float32) / 32767


def run(synth, reqs, seed):
    torch.manual_seed(seed)
    out = {}
    for group in synth.run(reqs):
        for r in group:
            out[r.index] = r
    return [out[i] for i in range(len(reqs))]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--device", default="cpu")
    ap.add_argument("--half", action="store_true")
    ap.add_argument("--compile", action="store_true")
    ap.add_argument("--n", type=int, default=16)
    ap.add_argument("--inexact", action="store_true", help="plain padding, no masks in the decoder")
    ap.add_argument("--nan-lengths", type=int, default=0, help="check every length 1..N for NaN")
    args = ap.parse_args()

    synth = Synth(args.models, device=args.device, half=args.half, compile=args.compile, pad_slack=10.0,
                  frame_budget=10**9, exact=not args.inexact)
    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    chunks = sentences.load(vocab)
    rng = np.random.default_rng(0)
    picks = [chunks[i][1] for i in rng.choice(len(chunks), args.n, replace=False)]
    voice = synth.voice_names.index("af_heart")
    reqs = [Request(ids, synth.style([(voice, 1.0)], len(ids)), 1.0) for ids in picks]

    # 1. vs the fork's forward
    if not args.compile:
        ids = picks[0]
        torch.manual_seed(1)
        with torch.inference_mode(), torch.autocast(synth.device.type, dtype=torch.float16, enabled=synth.half):
            ref_audio, ref_dur = synth.model.forward_with_tokens(
                torch.tensor([[0, *ids, 0]], device=synth.device), reqs[0].ref_s[None], 1.0)
        ref_pcm = (ref_audio.float() * 32767).clamp(-32768, 32767).to(torch.int16).cpu().numpy()
        ours = run(synth, reqs[:1], 1)[0]
        ours_pcm = np.frombuffer(ours.pcm, dtype="<i2")
        same_dur = ours.durations == ref_dur.tolist()
        diff = np.abs(ours_pcm.astype(int) - ref_pcm.astype(int)).max() if len(ours_pcm) == len(ref_pcm) else None
        print(f"vs KModel: durations equal={same_dur}, len {len(ours_pcm)} vs {len(ref_pcm)}, max |diff| {diff} (int16 units)")

    # 2. batched vs alone
    alone = [run(synth, [r], 2)[0] for r in reqs]
    alone_seed = [run(synth, [r], 3)[0] for r in reqs]
    for b in (4, args.n):
        batched = []
        for i in range(0, len(reqs), b):
            batched += run(synth, reqs[i : i + b], 2)
        dur_eq = sum(x.durations == y.durations for x, y in zip(alone, batched))
        corr = [spec_corr(pcm(x), pcm(y)) for x, y in zip(alone, batched)]
        floor = [spec_corr(pcm(x), pcm(y)) for x, y in zip(alone, alone_seed)]
        print(f"B={b}: durations equal {dur_eq}/{len(reqs)}; spectrogram corr padded vs alone "
              f"mean {np.mean(corr):.4f} min {np.min(corr):.4f}; seed noise floor mean {np.mean(floor):.4f} "
              f"min {np.min(floor):.4f}")

    # 3. non-finite outputs
    if args.nan_lengths:
        ids_all = sorted(synth.vocab_ids)
        bad = []
        for n in range(1, args.nan_lengths + 1):
            ids = [ids_all[(i * 11 + n) % len(ids_all)] for i in range(n)]
            r = run(synth, [Request(ids, synth.style([(voice, 1.0)], n), 1.0)], n)[0]
            if r.pcm is None:
                bad.append(n)
        print(f"non-finite outputs at batch 1 over lengths 1..{args.nan_lengths}: {len(bad)} {bad[:20]}")


if __name__ == "__main__":
    main()
