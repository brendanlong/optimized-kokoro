"""Held-out phrases through the deployed forward pass (Synth: fp16, excitation noise on, seed 0)
three ways, with loudness, length and Whisper's transcript:

  orig     alone, with the voice pack's own row (what the server does today)
  fitted   alone, with the fitted row (fit_short_rows.py)
  context  before a held-out filler sentence, with the row for the whole length, cut after the
           phrase's final punctuation (the target)

    uv run python train/score_short_rows.py MODELS PHRASES_JSON ROWS OUT_DIR [--voice af_heart]

Also renders, for listening, a few short phrases followed by a sentence as separate chunks
(today's server: each alone, trimmed, joined with the sentence gap) with original and fitted
rows; and checks that sentences of 31+ phonemes render identically with the patched pack.
Writes OUT_DIR/metrics.json and the clips in OUT_DIR/wav/.
"""

import argparse
import collections
import json
import os
import re
import sys

import numpy as np
import torch
from safetensors.torch import load_file

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "bench"))

from kokoro_worker.client import write_wav  # noqa: E402
from kokoro_worker.model import SAMPLES_PER_FRAME, Request, Synth  # noqa: E402
from make_phrases import number_words  # noqa: E402

SPACE = 16
BUCKETS = [(1, 5), (6, 10), (11, 20), (21, 30)]
CHUNKED = [("No.", "nˈO."), ("Yes.", "jˈɛs."), ("Chapter one.", "ʧˈæptəɹ wˈʌn."), ("Wait.", "wˈAt."),
           ("Oh!", "ˈO!")]
LISTEN_PER_LENGTH = 2  # clips kept for the page per phrase length, besides short_words.py's phrases
MEASURED = ["One.", "Chapter one.", "Twelve.", "Yes.", "No.", "Wait.", "he said.", "Silent?"]


def level(pcm: np.ndarray) -> dict:
    """short_words.py's measure: RMS over 5 ms windows above -45 dBFS, and their total length."""
    x = pcm.astype(np.float64) / 32767
    win = 120
    n = len(x) // win
    frames = x[: n * win].reshape(n, win)
    rms = np.sqrt((frames**2).mean(1))
    loud = rms > 10 ** (-45 / 20)
    if not loud.any():
        return {"rms_db": None, "speech_ms": 0}
    speech = frames[loud]
    return {"rms_db": float(20 * np.log10(np.sqrt((speech**2).mean()))), "speech_ms": int(loud.sum() * win / 24)}


def trimmed(pcm, lead=1200, tail=9840):
    """Speech plus 50 ms before and 410 ms after (the server's trim, roughly; short_words.py's)."""
    pcm = np.concatenate([pcm, np.zeros(tail, dtype=pcm.dtype)])
    x = np.abs(pcm.astype(np.float64)) > 32767 * 10 ** (-45 / 20)
    if not x.any():
        return pcm
    a, b = int(np.argmax(x)), len(x) - int(np.argmax(x[::-1]))
    return pcm[max(0, a - lead): b + tail]


def words(text):
    """Lowercase words, with whole numbers under 1000 spelled out (Whisper writes "383" for
    "Three hundred eighty-three")."""
    out = []
    for w in re.sub(r"[^a-z0-9' ]", " ", text.lower().replace("-", " ")).split():
        out += number_words(int(w)).replace("-", " ").split() if w.isdigit() and int(w) < 1000 else [w]
    return out


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
    ap = argparse.ArgumentParser()
    ap.add_argument("models")
    ap.add_argument("phrases")
    ap.add_argument("rows")
    ap.add_argument("out")
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--no-asr", action="store_true")
    ap.add_argument("--device", default="cuda")
    ap.add_argument("--max-test", type=int, default=0)
    args = ap.parse_args()
    os.makedirs(os.path.join(args.out, "wav"), exist_ok=True)
    synth = Synth(args.models, device=args.device, half=True, compile=False)
    v = synth.voice_names.index(args.voice)
    pack = synth.voices[v]
    fitted = load_file(args.rows)[args.voice].to(pack.device)
    patched = pack.clone()
    patched[: len(fitted)] = fitted
    data = json.load(open(args.phrases, encoding="utf-8"))
    vocab = json.load(open(os.path.join(args.models, "config.json"), encoding="utf-8"))["vocab"]
    fillers = data["fillers"]["test"]

    def render(ids, ref):
        torch.manual_seed(0)
        r = next(iter(synth.run([Request(ids, ref, 100)])))[0]
        return np.frombuffer(r.pcm, dtype="<i2").copy(), r.durations

    rows = []
    listen = collections.Counter()
    for k, p in enumerate(data["test"][: args.max_test or None]):
        ids, n = p["ids"], len(p["ids"])
        keep = p["text"] in MEASURED or listen[n] < LISTEN_PER_LENGTH
        if keep and p["text"] not in MEASURED:
            listen[n] += 1
        row = {"text": p["text"], "phonemes": p["phonemes"], "kind": p["kind"], "n": n, "clips": {}}
        clips = {}
        clips["orig"], d_o = render(ids, pack[n - 1])
        clips["fitted"], d_f = render(ids, patched[n - 1])
        row["frames"] = {"orig": sum(d_o[1:-1]), "fitted": sum(d_f[1:-1])}
        ctx = []
        for j, f in enumerate(fillers):
            full = ids + [SPACE] + f["ids"]
            pcm, d = render(full, pack[len(full) - 1])
            clip = pcm[: sum(d[: n + 1]) * SAMPLES_PER_FRAME]
            ctx.append((clip, sum(d[1: n + 1])))
        clips["context"] = ctx[0][0]
        row["frames"]["context"] = float(np.mean([c[1] for c in ctx]))
        row["level"] = {name: level(c) for name, c in clips.items()}
        row["level"]["context_all"] = [level(c[0]) for c in ctx]
        for name, c in clips.items():
            fn = f"t{k:03d}-{name}.wav"
            row["clips"][name] = fn
            if keep:
                write_wav(os.path.join(args.out, "wav", fn), c.astype("<i2").tobytes())
        row["listen"] = keep
        row["_pcm"] = clips
        rows.append(row)
        if k % 50 == 0:
            print(k, json.dumps({x: row[x] for x in ("text", "frames")}, ensure_ascii=False), flush=True)

    if not args.no_asr:
        from transformers import pipeline
        asr = pipeline("automatic-speech-recognition", "openai/whisper-small.en", device=0, torch_dtype=torch.float16)
        for row in rows:
            row["asr"] = {}
            for name, c in row["_pcm"].items():
                x = c.astype(np.float32) / 32767
                x = np.concatenate([np.zeros(2400, np.float32), x, np.zeros(4800, np.float32)])
                at_16k = np.interp(np.arange(0, len(x), 1.5), np.arange(len(x)), x).astype(np.float32)
                hyp = asr({"raw": at_16k, "sampling_rate": 16000})["text"].strip()
                row["asr"][name] = {"text": hyp, "errors": min(len(words(row["text"])), wer(words(row["text"]), words(hyp)))}
    for row in rows:
        del row["_pcm"]

    # Today's server for a short phrase then a sentence: separate chunks, trimmed, joined.
    sentence = fillers[0]
    sent_pcm, _ = render(sentence["ids"], pack[len(sentence["ids"]) - 1])
    chunked = []
    for i, (text, ph) in enumerate(CHUNKED):
        ids = [vocab[c] for c in ph if c in vocab]
        entry = {"text": f"{text} {sentence['text']}", "clips": {}}
        for name, pk in (("orig", pack), ("fitted", patched)):
            pcm, _ = render(ids, pk[len(ids) - 1])
            joined = np.concatenate([trimmed(pcm), trimmed(sent_pcm)])
            fn = f"c{i}-{name}.wav"
            write_wav(os.path.join(args.out, "wav", fn), joined.astype("<i2").tobytes())
            entry["clips"][name] = fn
        full = ids + [SPACE] + sentence["ids"]
        pcm, _ = render(full, pack[len(full) - 1])
        fn = f"c{i}-together.wav"
        write_wav(os.path.join(args.out, "wav", fn), trimmed(pcm).astype("<i2").tobytes())
        entry["clips"]["together"] = fn
        chunked.append(entry)

    # Rows 30+ are untouched, so anything of 31+ phonemes renders the same, bit for bit.
    same_rows = bool(torch.equal(patched[len(fitted):], pack[len(fitted):]))
    long_same = []
    for f in data["fillers"]["train"] + data["fillers"]["test"]:
        a, _ = render(f["ids"], pack[len(f["ids"]) - 1])
        b, _ = render(f["ids"], patched[len(f["ids"]) - 1])
        long_same.append(bool(np.array_equal(a, b)))

    def summarize(sel):
        out = {"phrases": len(sel)}
        if not sel:
            return out
        for name in ("orig", "fitted"):
            dr = [r["level"][name]["rms_db"] - r["level"]["context"]["rms_db"] for r in sel
                  if r["level"][name]["rms_db"] is not None and r["level"]["context"]["rms_db"] is not None]
            ds = [r["level"][name]["speech_ms"] / max(1, r["level"]["context"]["speech_ms"]) for r in sel]
            df = [r["frames"][name] - r["frames"]["context"] for r in sel]
            out[name] = {"rms_db_vs_context": round(float(np.mean(dr)), 2),
                         "abs_rms_db_vs_context": round(float(np.mean(np.abs(dr))), 2),
                         "speech_len_ratio_vs_context": round(float(np.median(ds)), 3),
                         "frames_vs_context": round(float(np.mean(df)), 2),
                         "abs_frames_vs_context": round(float(np.mean(np.abs(df))), 2)}
        if sel and "asr" in sel[0]:
            nw = sum(len(words(r["text"])) for r in sel)
            for name in ("orig", "fitted", "context"):
                e = sum(r["asr"][name]["errors"] for r in sel)
                out.setdefault(name, {})["wer_pct"] = round(100 * e / nw, 1)
                out[name]["exact_pct"] = round(100 * np.mean([r["asr"][name]["errors"] == 0 for r in sel]), 1)
        return out

    summary = {"all": summarize(rows)}
    for lo, hi in BUCKETS:
        summary[f"{lo}-{hi}"] = summarize([r for r in rows if lo <= r["n"] <= hi])
    summary["no_numbers"] = summarize([r for r in rows if r["kind"] not in ("number", "letter")])
    doc = {"summary": summary, "rows_30_plus_unchanged": same_rows, "long_renders_identical": long_same,
           "phrases": rows, "chunked": chunked, "fillers": [f["text"] for f in fillers]}
    json.dump(doc, open(os.path.join(args.out, "metrics.json"), "w"), ensure_ascii=False, indent=1)
    print(json.dumps(summary, indent=1))
    print("rows 30+ unchanged:", same_rows, "long renders identical:", long_same)


if __name__ == "__main__":
    main()
