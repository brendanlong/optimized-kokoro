"""Records Kokoro-FastAPI's per-chunk trimming and overlap stitching for crates/server's tests.

Run with the pinned Kokoro-FastAPI source importable as `api.src` (numpy and pydantic needed).
"""

import json
import random
import sys

import numpy as np

sys.path.insert(0, "/ref")

from api.src.inference import stitching
from api.src.services.audio import AudioNormalizer, AudioService
from api.src.inference.base import AudioChunk

rng = random.Random(7)
out = {"trim": [], "stitch": []}

# Trim: audio given as runs of (length, amplitude), samples alternate sign within a run.
def build(runs):
    parts = []
    for length, amp in runs:
        s = np.full(length, amp, dtype=np.int16)
        s[1::2] = -amp
        parts.append(s)
    return np.concatenate(parts) if parts else np.zeros(0, dtype=np.int16)

for case in range(60):
    runs = []
    for _ in range(rng.randint(0, 5)):
        runs.append((rng.randint(0, 6000), rng.choice([0, 3, 100, 184, 185, 186, 1000, 20000])))
    text = rng.choice(["Hello.", "Hi!", "What?", "One,", "end", "  x . ", "", "Wow!  "])
    speed = rng.choice([0.25, 0.5, 1.0, 1.3, 2.0, 4.0])
    audio = build(runs)
    chunk = AudioService.trim_audio(AudioChunk(audio.copy()), text, speed, False, AudioNormalizer())
    trimmed = chunk.audio
    # Locate the kept slice in the original.
    start = None
    for s in range(len(audio) - len(trimmed) + 1):
        if np.array_equal(audio[s : s + len(trimmed)], trimmed):
            start = s
            break
    out["trim"].append({"runs": runs, "text": text, "speed": speed, "start": int(start), "len": int(len(trimmed))})

# Stitching with a deterministic fake model.
VOCAB = "abcdefghijklmnopqrstuvwxyzæðŋɑɔəɛɪʊʌˈˌː"
SPACE_ID = 16
ids = {c: 43 + i for i, c in enumerate(VOCAB)}
ids[" "] = SPACE_ID


def fake_durations(context_ids, row):
    n = len(context_ids)
    return [2] + [1 + (i * 31 + j * 7 + row) % 4 for j, i in enumerate(context_ids)] + [2]


def fake_samples(context_ids, total):
    seed = sum(context_ids) % 997
    k = (np.arange(total, dtype=np.int64) * 13 + seed) % 4001 - 2000
    return k.astype(np.int16)


for case in range(40):
    words = []
    length = rng.randint(400, 1600)
    while sum(len(w) + 1 for w in words) < length:
        words.append("".join(rng.choice(VOCAB) for _ in range(rng.randint(1, rng.choice([8, 30, 120])))))
    ps = " ".join(words)
    calls = []

    def render(context, row):
        cids = [ids[c] for c in context]
        dur = fake_durations(cids, row)
        k = fake_samples(cids, 600 * sum(dur))
        calls.append(cids)
        # Floats that read back as k exactly once scaled by 32767 and truncated.
        audio = (k.astype(np.float64) + 0.5 * np.sign(k)) / 32767
        return audio.astype(np.float32), np.array(dur)

    pieces = []
    for audio, _ in stitching.render_stitched(ps, 450, render):
        pcm = np.clip(audio * 32767, -32768, 32767).astype(np.int16)
        rest = pcm[480:].astype(np.int64)
        pieces.append({
            "len": int(len(pcm)),
            "head": pcm[:480].tolist(),
            "rest_sum": int(rest.sum()),
            "rest_weighted": int((rest * (np.arange(len(rest), dtype=np.int64) % 1009)).sum()),
        })
    out["stitch"].append({"ids": [ids[c] for c in ps], "contexts": calls, "pieces": pieces})

json.dump(out, sys.stdout)
