"""WAVs for a listening check: each sentence alone, and padded inside a batch with a long sentence,
with the masked (exact) decoder and with plain padding.

    python bench/make_pairs.py MODELS OUT_DIR [--no-compile]
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
from check_equivalence import spec_corr  # noqa: E402
from kokoro_worker.client import write_wav  # noqa: E402
from kokoro_worker.model import Request, Synth  # noqa: E402


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("out")
    ap.add_argument("--no-compile", action="store_true")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)

    vocab = json.load(open(os.path.join(args.models, "config.json")))["vocab"]
    chunks = sentences.load(vocab)
    texts = {}
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            for c in json.loads(line)["chunks"]:
                texts[c["phonemes"]] = c["text"]
    by_len = sorted(chunks, key=lambda c: len(c[1]))
    # Short, typical and long-ish sentences, padded up to the longest sentence in the corpus.
    picks = [by_len[int(q * (len(by_len) - 1))] for q in (0.15, 0.5, 0.8, 0.95)]
    longest = by_len[-1]
    batch = picks + [longest]

    variants = {}
    for exact in (True, False):
        synth = Synth(args.models, compile=not args.no_compile, pad_slack=100.0, frame_budget=10**9, exact=exact)
        voice = synth.voice_names.index("af_heart")
        reqs = [Request(ids, synth.style(voice, len(ids)), 100) for _, ids in batch]
        for r in reqs[:2]:  # compile and warm before the real runs
            list(synth.run([r]))
        list(synth.run(reqs))
        torch.manual_seed(0)
        alone = [next(iter(synth.run([r])))[0] for r in reqs[:-1]]
        torch.manual_seed(0)
        together = {r.index: r for g in synth.run(reqs) for r in g}
        variants["alone"] = alone
        variants["exact" if exact else "plain"] = [together[i] for i in range(len(picks))]
        frames_long = len(together[len(picks)].durations) and sum(together[len(picks)].durations)
        del synth
        torch.cuda.empty_cache()

    meta = []
    for i, (ps, ids) in enumerate(picks):
        pcm = {k: np.frombuffer(v[i].pcm, dtype="<i2") for k, v in variants.items()}
        row = {
            "index": i,
            "text": texts.get(ps, ""),
            "phonemes": ps,
            "n_ids": len(ids),
            "seconds": round(len(pcm["alone"]) / 24000, 2),
            "padding_fraction": round(1 - len(pcm["alone"]) / (frames_long * 600), 3),
            "files": {},
            "corr_vs_alone": {},
        }
        for k, samples in pcm.items():
            name = f"s{i}-{k}.wav"
            write_wav(os.path.join(args.out, name), samples.tobytes())
            row["files"][k] = name
            if k != "alone":
                row["corr_vs_alone"][k] = round(
                    spec_corr(pcm["alone"].astype(np.float32) / 32767, samples.astype(np.float32) / 32767), 4)
        meta.append(row)
        print(json.dumps(row, ensure_ascii=False))
    with open(os.path.join(args.out, "pairs.json"), "w", encoding="utf-8") as f:
        json.dump({"padded_to_frames": frames_long, "pairs": meta}, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
