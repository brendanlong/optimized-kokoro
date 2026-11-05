"""Word error rate of WAV files against their texts, with Whisper small.en.

    uv run python bench/asr_check.py PASSAGES_JSON DIR

PASSAGES_JSON is window_context.py's passages.json; prints the WER per variant.
"""

import json
import os
import re
import sys
import wave

import numpy as np
import torch
from transformers import pipeline


def words(text):
    return re.sub(r"[^a-z0-9' ]", " ", text.lower()).split()


def wer(ref, hyp):
    d = list(range(len(hyp) + 1))
    for i in range(1, len(ref) + 1):
        prev, d[0] = d[0], i
        for j in range(1, len(hyp) + 1):
            cur = d[j]
            d[j] = min(d[j] + 1, d[j - 1] + 1, prev + (ref[i - 1] != hyp[j - 1]))
            prev = cur
    return d[-1]


def main():
    meta = json.load(open(sys.argv[1], encoding="utf-8"))
    asr = pipeline("automatic-speech-recognition", "openai/whisper-small.en", device=0, torch_dtype=torch.float16)
    totals = {}
    for p in meta:
        ref = words(" ".join(p["text"]))
        for variant, fn in p["files"].items():
            w = wave.open(os.path.join(sys.argv[2], fn))
            x = np.frombuffer(w.readframes(w.getnframes()), dtype="<i2").astype(np.float32) / 32767
            at_16k = np.interp(np.arange(0, len(x), 1.5), np.arange(len(x)), x).astype(np.float32)
            hyp = words(asr({"raw": at_16k, "sampling_rate": 16000}, return_timestamps=True)["text"])
            errors = wer(ref, hyp)
            e, n = totals.get(variant, (0, 0))
            totals[variant] = (e + errors, n + len(ref))
            if errors:
                print(f"{fn}: {errors} errors: {' '.join(hyp)[:160]}", flush=True)
    for variant, (e, n) in totals.items():
        print(f"{variant}: WER {100 * e / n:.1f}% ({e} of {n} words)", flush=True)


if __name__ == "__main__":
    main()
