"""How long can a real chunk get? Predicted audio length (speed 1) for the longest chunks the front
can send: consecutive corpus sentences joined up to 510 phonemes, in several voices.

    python bench/max_duration.py MODELS [--device cpu]
"""

import argparse
import json
import os
import sys

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import SAMPLES_PER_FRAME, Synth  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("models")
ap.add_argument("--device", default="cpu")
args = ap.parse_args()

synth = Synth(args.models, device=args.device, half=args.device != "cpu", compile=False)
vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
space = vocab[" "]
chunks = [ids for _, ids in sentences.load(vocab)]

long = []
cur: list[int] = []
for ids in chunks:
    if cur and len(cur) + 1 + len(ids) > 510:
        long.append(cur)
        cur = []
    cur = cur + [space] + ids if cur else list(ids)
long = [c for c in long if len(c) >= 450]
voices = ["af_heart", "am_adam", "bf_emma", "bm_george", "af_bella", "am_michael", "af_nicole", "bm_lewis"]
rows = []
with torch.inference_mode():
    for name in voices:
        if name not in synth.voice_names:
            continue
        v = synth.voice_names.index(name)
        for ids in long:
            n = len(ids)
            width = n + 2
            t = torch.zeros(1, width, dtype=torch.long)
            t[0, 1 : n + 1] = torch.tensor(ids)
            t = t.to(synth.device)
            mask = torch.zeros(1, width, dtype=torch.bool, device=synth.device)
            ref = synth.style(v, n)[None]
            with torch.autocast(synth.device.type, dtype=torch.float16, enabled=synth.half):
                dur, _, _ = synth._durations(t, mask, torch.tensor([width]), ref[:, 128:],
                                             torch.ones(1, device=synth.device), torch.full((1, 1), 50, device=synth.device))
            frames = int(dur.sum())
            rows.append((frames * SAMPLES_PER_FRAME / 24000, n, name))
rows.sort(reverse=True)
print(f"{len(long)} chunks of 450-510 phonemes x {len(voices)} voices at speed 1")
print("longest:", [(round(s, 1), n, v) for s, n, v in rows[:5]])
secs = [s for s, _, _ in rows]
print(f"seconds: max {max(secs):.1f}, median {sorted(secs)[len(secs) // 2]:.1f}, "
      f"max per phoneme {max(s / n for s, n, _ in rows) * 510:.1f} s per 510")
