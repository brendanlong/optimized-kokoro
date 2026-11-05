"""How many frames does Kokoro predict for a request of n phoneme ids? The front sizes requests for
the worker's chunk cap from this (crates/server/src/stitch.rs, `max_render_ids`).

Durations only (the text side), fp16 as the worker runs, at speeds 0.25 and 1:
- every corpus chunk (English) as the pipeline makes it, with its own style row, in 8 voices;
- windows of n ids of each corpus document's chunks joined with spaces, starting at a word, like
  the pieces of a stitched chunk. The server renders a piece with the style row of the whole chunk
  (`Plan::style_len`, longer than the piece, up to 510), so each window is rendered with its own
  length and 6 longer ones up to 510;
- the non-English chunks of the reference fixtures (Chinese, Japanese, Hindi, ...; mixed-script
  English), in the voices of their language, with their own style row and 6 longer ones.
Prints the most frames x speed for each kind and length, and the line A x n + B above all of them
(the fit the server uses), and writes every point to --json.

    python bench/frames_per_id.py MODELS [--device cuda] [--windows 200] [--json out.json]
"""

import argparse
import json
import os
import sys
from collections import defaultdict

import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker.model import Synth  # noqa: E402
from kokoro_worker.protocol import max_duration  # noqa: E402

SPEEDS = (25, 100)
VOICES = ["af_heart", "am_adam", "bf_emma", "bm_george", "af_bella", "am_michael", "af_nicole", "bm_lewis"]
WINDOWS = (8, 16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 510)
FIXTURES = os.path.join(sentences.HERE, "..", "..", "reference", "fixtures")


def ids_of(vocab, phonemes):
    return [vocab[c] for c in phonemes if c in vocab]


def documents(vocab):
    """[[ids of each chunk]] per English document of the corpus."""
    out = []
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") in ("a", "b") and "chunks" in row:
                out.append([c for c in (ids_of(vocab, ch["phonemes"]) for ch in row["chunks"]) if c])
    return out


def fixture_chunks(vocab):
    """(lang, ids) for the fixtures' chunks that aren't plain English: other languages, and the
    mixed-script records (kana_mixed, zh_mixed)."""
    out = []
    for name in ("hard_cases.out.jsonl", "kana_mixed.out.jsonl", "zh_mixed.out.jsonl"):
        path = os.path.join(FIXTURES, name)
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as f:
            for line in f:
                row = json.loads(line)
                lang = row.get("lang")
                if lang in ("a", "b") and name == "hard_cases.out.jsonl":
                    continue
                for ch in row.get("chunks") or []:
                    ids = ids_of(vocab, ch.get("phonemes") or "")
                    if 1 <= len(ids) <= 510:
                        out.append((lang, ids))
    return out


def styles(n: int) -> list[int]:
    return sorted({n} | {n + k * (510 - n) // 6 for k in range(1, 7)})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--windows", type=int, default=200, help="windows per length")
    ap.add_argument("--json")
    args = ap.parse_args()
    synth = Synth(args.models, device=args.device, half=args.device != "cpu", compile=False)
    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    space = vocab[" "]
    docs = documents(vocab)
    english = [v for v in VOICES if v in synth.voice_names]

    # (kind, ids, voices, style lengths)
    jobs = [("chunk", c, english, [len(c)]) for doc in docs for c in doc if len(c) <= 510]
    for n in WINDOWS:
        found = []
        for doc in docs:
            joined = []
            for c in doc:
                joined += ([space] if joined else []) + c
            starts = [0] + [i + 1 for i, t in enumerate(joined) if t == space]
            found += [joined[s : s + n] for s in starts[:: max(1, n // 8)] if s + n <= len(joined)]
        step = max(1, len(found) // args.windows)
        jobs += [("window", w, english, styles(n)) for w in found[::step][: args.windows]]
    for lang, ids in fixture_chunks(vocab):
        voices = [v for v in synth.voice_names if v[0] == lang] if lang not in ("a", "b") else english
        jobs.append((f"other-{lang}", ids, voices or english, styles(len(ids))))
    print(f"{sum(len(v) * len(s) for _, _, v, s in jobs)} (sequence, voice, style) cases, "
          f"{len(jobs)} sequences", flush=True)

    dev = synth.device
    points = []  # (kind, n, style_len, voice, speed, frames)
    with torch.inference_mode():
        for done, (kind, ids, voices, style_lens) in enumerate(jobs):
            n = len(ids)
            rows = [(st, sp) for st in style_lens for sp in SPEEDS]
            b = len(rows)
            t = torch.zeros(b, n + 2, dtype=torch.long)
            t[:, 1 : n + 1] = torch.tensor(ids)
            t = t.to(dev)
            mask = torch.zeros(b, n + 2, dtype=torch.bool, device=dev)
            speed = torch.tensor([sp / 100 for _, sp in rows], device=dev)
            max_dur = torch.tensor([[max_duration(sp)] for _, sp in rows], device=dev)
            for name in voices:
                v = synth.voice_names.index(name)
                ref = torch.stack([synth.style(v, st) for st, _ in rows])
                with torch.autocast(dev.type, dtype=torch.float16, enabled=synth.half):
                    dur, _, _ = synth._durations(t, mask, torch.tensor([n + 2] * b), ref[:, 128:], speed, max_dur)
                for (st, sp), f in zip(rows, dur.sum(1).tolist()):
                    points.append((kind, n, st, name, sp, f))
            if done % 500 == 0:
                print(f"{done} of {len(jobs)}", flush=True)

    # The most frames x speed per kind and length bucket.
    top = defaultdict(lambda: (0.0, None))
    for kind, n, st, name, sp, f in points:
        key = (kind.split("-")[0], n if kind == "window" else min(b for b in WINDOWS if n <= b))
        x = f * sp / 100
        if x > top[key][0]:
            top[key] = (x, (n, st, name, sp / 100, f))
    print("most frames x speed: kind, ids (up to), value (n, style_len, voice, speed, frames), per id")
    for key in sorted(top):
        x, why = top[key]
        print(f"{key[0]:6} {key[1]:4} {x:8.1f} {why} {x / why[0]:.2f}/id")

    # Lines A n + B over every point: for each A, the smallest B.
    xs = [(n, f * sp / 100) for _, n, _, _, sp, f in points]
    print("A, smallest B with every point at or under A n + B, ids for 1,200 and 300 frames")
    for a10 in range(30, 51):
        a = a10 / 10
        b = max(x - a * n for n, x in xs)
        print(f"{a:.1f} {b:7.1f}  {int((1200 - b) // a)} {int((300 - b) // a)}")
    if args.json:
        with open(args.json, "w") as f:
            json.dump(points, f)


if __name__ == "__main__":
    main()
