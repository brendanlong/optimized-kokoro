"""Fit one voice's short rows (lengths 1..30, rows 0..29) by self-distillation; nothing else changes.

    uv run python train/fit_short_rows.py MODELS PHRASES_JSON OUT_DIR [--voice af_heart] [--steps N]

Teacher: the frozen model rendering "<phrase> <filler sentence>" with the row for that whole
length, cut after the phrase (kdiff.teacher). Student: the same model rendering the phrase alone
with its own row, which is the only thing trained. Loss per phrase:
  stft   multi-resolution L1 of log STFT magnitudes of the student's audio, decoded with the
         teacher's durations so frames line up, against the teacher's cut
  level  L1 between their RMS envelopes in dB (/10), over the target's frames above -50 dB
  dur    L1 between the student's durations before rounding and the teacher's, per id
  f0, n  L1 against the teacher's F0 (/50 Hz) and energy curves over the same frames
  reg    squared distance of each row from its original, summed over its 256 values
Writes OUT_DIR/rows.safetensors ({voice: [30, 256]}), OUT_DIR/log.jsonl (training losses, and
held-out losses every --eval-every steps, per length) and OUT_DIR/summary.json.
"""

import argparse
import json
import math
import os
import random
import time

import torch
from safetensors.torch import save_file

import kdiff

N_ROWS = 30


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("phrases")
    ap.add_argument("out")
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--steps", type=int, default=1000)
    ap.add_argument("--batch", type=int, default=16)
    ap.add_argument("--lr", type=float, default=2e-3)
    ap.add_argument("--fillers", type=int, default=4, help="training fillers per phrase")
    ap.add_argument("--w-dur", type=float, default=0.2)
    ap.add_argument("--w-f0", type=float, default=0.2)
    ap.add_argument("--w-n", type=float, default=0.2)
    ap.add_argument("--w-level", type=float, default=1.0)
    ap.add_argument("--teacher-cache", help="file to keep the teacher's targets in, and reuse")
    ap.add_argument("--w-reg", type=float, default=0.05)
    ap.add_argument("--eval-every", type=int, default=250)
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--max-test", type=int, default=0, help="cap on held-out phrases (0: all)")
    ap.add_argument("--max-train", type=int, default=0, help="cap on training phrases (0: all)")
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    dev = args.device
    kdiff.noise_off()
    torch.backends.cuda.matmul.allow_tf32 = False
    torch.backends.cudnn.allow_tf32 = False
    m, vocab, voices = kdiff.load(args.models, dev)
    voice = voices[args.voice]
    data = json.load(open(args.phrases, encoding="utf-8"))
    train = data["train"][: args.max_train or None]
    test = data["test"][: args.max_test or None]
    fillers = [f["ids"] for f in data["fillers"]["train"][: args.fillers]]
    test_fillers = [f["ids"] for f in data["fillers"]["test"]]

    t0 = time.time()
    key = [args.voice, [p["ids"] for p in train], [p["ids"] for p in test], fillers, test_fillers]
    cached = None
    if args.teacher_cache and os.path.exists(args.teacher_cache):
        cached = torch.load(args.teacher_cache, weights_only=True)
        if cached["key"] != key:
            cached = None
    if cached:
        targets, val_t = cached["targets"], cached["val"]
    else:
        targets = [[kdiff.teacher(m, voice, p["ids"], f) for f in fillers] for p in train]
        val_t = [kdiff.teacher(m, voice, p["ids"], f) for p in test for f in test_fillers]
        if args.teacher_cache:
            torch.save({"key": key, "targets": targets, "val": val_t}, args.teacher_cache + ".tmp")
            os.replace(args.teacher_cache + ".tmp", args.teacher_cache)
    val = list(zip([p for p in test for _ in test_fillers], val_t))
    teacher_s = time.time() - t0
    print(f"teacher: {len(train)} x {len(fillers)} + {len(val)} renders in {teacher_s:.0f} s", flush=True)

    rows0 = voice[:N_ROWS].clone()
    rows = torch.nn.Parameter(rows0.clone())
    opt = torch.optim.Adam([rows], lr=args.lr)
    sched = torch.optim.lr_scheduler.LambdaLR(
        opt, lambda s: min(1.0, (s + 1) / 50) * 0.5 * (1 + math.cos(math.pi * min(s, args.steps) / args.steps)))
    weights = {"stft": 1.0, "level": args.w_level, "dur": args.w_dur, "f0": args.w_f0, "n": args.w_n}
    log = open(os.path.join(args.out, "log.jsonl"), "w")

    def evaluate(step):
        sums, per_len = {}, {}
        with torch.no_grad():
            for p, t in val:
                n = len(p["ids"])
                ls = {k: float(v) for k, v in kdiff.losses(m, rows[n - 1], p["ids"], t).items()}
                ls["total"] = sum(weights[k] * v for k, v in ls.items())
                for k, v in ls.items():
                    sums[k] = sums.get(k, 0.0) + v / len(val)
                per_len.setdefault(n, []).append(ls["total"])
        rec = {"step": step, "split": "test", **{k: round(v, 4) for k, v in sums.items()},
               "per_len": {n: round(sum(v) / len(v), 4) for n, v in sorted(per_len.items())}}
        log.write(json.dumps(rec) + "\n")
        log.flush()
        print(f"step {step} TEST " + " ".join(f"{k} {v:.4f}" for k, v in sums.items()), flush=True)
        return rec

    first = evaluate(0)
    rng = random.Random(0)
    t1 = time.time()
    for step in range(1, args.steps + 1):
        opt.zero_grad(set_to_none=True)
        sums = {}
        for _ in range(args.batch):
            i = rng.randrange(len(train))
            p, t = train[i], rng.choice(targets[i])
            ls = kdiff.losses(m, rows[len(p["ids"]) - 1], p["ids"], t)
            loss = sum(weights[k] * v for k, v in ls.items()) / args.batch
            loss.backward()
            for k, v in ls.items():
                sums[k] = sums.get(k, 0.0) + v.item() / args.batch
        reg = args.w_reg * ((rows - rows0) ** 2).sum(-1).mean()
        reg.backward()
        opt.step()
        sched.step()
        if step % 10 == 0:
            rec = {"step": step, "split": "train", **{k: round(v, 4) for k, v in sums.items()},
                   "reg": round(float(reg), 5), "lr": sched.get_last_lr()[0]}
            log.write(json.dumps(rec) + "\n")
        if step % 50 == 0:
            print(f"step {step} " + " ".join(f"{k} {v:.4f}" for k, v in sums.items())
                  + f" reg {float(reg):.5f} ({time.time() - t1:.0f} s)", flush=True)
        if step % args.eval_every == 0 or step == args.steps:
            last = evaluate(step)
            # a row with no training phrase gets no gradient, so it stays as it was
            save_file({args.voice: rows.detach().cpu().contiguous()}, os.path.join(args.out, "rows.tmp"))
            os.replace(os.path.join(args.out, "rows.tmp"), os.path.join(args.out, "rows.safetensors"))
    train_s = time.time() - t1
    drift = (rows.detach() - rows0).norm(dim=1)
    cos_t = torch.nn.functional.cosine_similarity(rows.detach()[:, :128], voice[199:200, :128])
    cos_t0 = torch.nn.functional.cosine_similarity(rows0[:, :128], voice[199:200, :128])
    summary = {"args": vars(args), "teacher_s": round(teacher_s), "train_s": round(train_s),
               "test_before": first, "test_after": last,
               "row_drift_l2": [round(float(x), 3) for x in drift],
               "timbre_cos_to_row199_before": [round(float(x), 3) for x in cos_t0],
               "timbre_cos_to_row199_after": [round(float(x), 3) for x in cos_t]}
    json.dump(summary, open(os.path.join(args.out, "summary.json"), "w"), indent=1)
    print(json.dumps({k: summary[k] for k in ("teacher_s", "train_s", "row_drift_l2")}), flush=True)


if __name__ == "__main__":
    main()
