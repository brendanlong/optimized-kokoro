"""Do the kernels (kokoro_worker/kernels.py) change the audio against cuDNN and cuFFT?

    uv run python bench/equiv_kernels.py MODELS [--compile] [--n 0] [--whisper 200] [--clips DIR]

Every real chunk of the corpus (or --n of them) at batch 1: the cuDNN reference (bench/reference.py)
with seed 2 and seed 3 (the noise floor: the harmonic source draws random noise), and the worker's
model with seed 2. Reports durations that differ, log-spectrogram correlation and loudness against
the reference, and with --whisper N, Whisper (small.en) word error rates on the first N chunks.
--clips DIR writes a few reference/kernels pairs as WAV files, for listening.
"""

import argparse
import json
import os
import re
import sys
import wave

import numpy as np
import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import reference  # noqa: E402
import sentences  # noqa: E402
from check_equivalence import pcm, run, spec_corr  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402

CLIPS = (3, 41, 117, 236, 402, 588)  # chunk indices: a spread of lengths


def load(vocab):
    """[(text, ids)] for every English chunk of the corpus."""
    out = []
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") not in ("a", "b"):
                continue
            for c in row.get("chunks", []):
                ids = [vocab[p] for p in c["phonemes"] if p in vocab]
                if 1 <= len(ids) <= 510:
                    out.append((c["text"], ids))
    return out


def words(text):
    return re.sub(r"[^a-z0-9' ]", " ", text.lower().replace("-", " ")).split()


def wer(ref, hyp):
    """(word edits, reference words)."""
    r, h = words(ref), words(hyp)
    d = list(range(len(h) + 1))
    for i in range(1, len(r) + 1):
        prev, d[0] = d[0], i
        for j in range(1, len(h) + 1):
            prev, d[j] = d[j], min(d[j] + 1, d[j - 1] + 1, prev + (r[i - 1] != h[j - 1]))
    return d[len(h)], len(r)


def write_wav(path, result):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(24000)
        w.writeframes(result.pcm)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--compile", action="store_true")
    ap.add_argument("--n", type=int, default=0)
    ap.add_argument("--whisper", type=int, default=0)
    ap.add_argument("--clips")
    args = ap.parse_args()

    ref_synth = reference.cudnn(Synth(args.models, compile=args.compile))
    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    chunks = load(vocab)[: args.n or None]
    voice = ref_synth.voice_names.index("af_heart")

    def go(synth, seed):
        return [run(synth, [Request(ids, synth.style(voice, len(ids)), 100)], seed)[0] for _, ids in chunks]

    exact = go(ref_synth, 2)
    runs = {"cuDNN, seed 3 (noise floor)": go(ref_synth, 3)}
    del ref_synth
    torch._dynamo.reset()
    runs["kernels, seed 2"] = go(Synth(args.models, compile=args.compile), 2)
    print(f"{'compiled' if args.compile else 'eager'} fp16 batch 1: {len(chunks)} chunks, against cuDNN seed 2")
    rms = lambda r: float(np.sqrt(np.mean(pcm(r) ** 2)))  # noqa: E731
    for name, other in runs.items():
        n_dur = sum(sum(p != q for p, q in zip(x.durations, y.durations)) for x, y in zip(exact, other))
        corr = np.array([spec_corr(pcm(x), pcm(y)) for x, y in zip(exact, other)])
        loud = np.array([rms(y) / rms(x) for x, y in zip(exact, other)])
        print(f"  {name:28s} durations differing {n_dur} of {sum(len(x.durations) for x in exact)}; spectrogram "
              f"corr mean {corr.mean():.4f} min {corr.min():.4f} p1 {np.percentile(corr, 1):.4f}; RMS ratio "
              f"{loud.mean():.3f} ({loud.min():.3f}..{loud.max():.3f})", flush=True)

    if args.clips:
        os.makedirs(args.clips, exist_ok=True)
        with open(os.path.join(args.clips, "clips.json"), "w") as f:
            json.dump([{"i": i, "text": chunks[i][0]} for i in CLIPS if i < len(chunks)], f, indent=1)
        for i in CLIPS:
            if i < len(chunks):
                write_wav(os.path.join(args.clips, f"{i}-cudnn.wav"), exact[i])
                write_wav(os.path.join(args.clips, f"{i}-kernels.wav"), runs["kernels, seed 2"][i])

    if args.whisper:
        from transformers import pipeline

        asr = pipeline("automatic-speech-recognition", model="openai/whisper-small.en", device=0,
                       dtype=torch.float16)

        def transcribe(results):
            out = []
            for r in results[: args.whisper]:
                x = pcm(r)
                at_16k = np.interp(np.arange(0, len(x), 1.5), np.arange(len(x)), x).astype(np.float32)
                out.append(asr({"raw": at_16k, "sampling_rate": 16000})["text"])
            return out

        texts = [t for t, _ in chunks[: args.whisper]]
        ref = transcribe(exact)
        e, n = map(sum, zip(*(wer(t, h) for t, h in zip(texts, ref))))
        print(f"Whisper small.en, {len(texts)} chunks ({n} words): WER vs text, cuDNN seed 2: {100 * e / n:.2f}%")
        for name, other in runs.items():
            hyp = transcribe(other)
            e, n = map(sum, zip(*(wer(t, h) for t, h in zip(texts, hyp))))
            e2, n2 = map(sum, zip(*(wer(x, h) for x, h in zip(ref, hyp))))
            diff = sum(words(x) != words(h) for x, h in zip(ref, hyp))
            print(f"  {name:28s} WER vs text {100 * e / n:.2f}%; vs the cuDNN transcript {100 * e2 / n2:.2f}% "
                  f"({diff} of {len(texts)} transcripts differ)", flush=True)


if __name__ == "__main__":
    main()
