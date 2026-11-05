"""Why single words come out oddly (issue #46): each short item rendered three ways, with its
loudness and length.

    uv run python bench/short_words.py MODELS OUT_DIR

  alone     on its own, with the voice pack row for its own length (what the server does today)
  long_row  on its own, with the row for a typical sentence's length (LONG_ROW phonemes)
  context   followed by a real sentence in one request, then cut out using the durations

  carrier   followed by a fixed carrier sentence, then cut out (for a request with nothing after it)
  ellipsis  on its own, ending in "…" instead of its punctuation

and, for listening, the item followed by that sentence three ways:
  today     each rendered alone, joined with the server's sentence gap
  split     rendered together, split at the boundary using the durations, with the same gap
  merged    rendered together, with Kokoro's own pause

If long_row fixes alone, the voice pack's short rows are the cause; if only context does, it's the
instance norms (statistics over a short span), and rendering with context is the fix.
"""

import json
import os
import sys

import numpy as np
import torch

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from kokoro_worker.client import write_wav  # noqa: E402
from kokoro_worker.model import SAMPLES_PER_FRAME, Request, Synth  # noqa: E402

LONG_ROW = 80
VOICES = ["af_heart", "am_adam"]
# (label, phonemes) in misaki's notation, as the pipeline would produce them.
ITEMS = [
    ("One.", "wˈʌn."),
    ("Chapter one.", "ʧˈæptəɹ wˈʌn."),
    ("Twelve.", "twˈɛlv."),
    ("Yes.", "jˈɛs."),
    ("No.", "nˈO."),
    ("Wait.", "wˈAt."),
    ("he said.", "hi sˈɛd."),
    ("Silent?", "sˈIlənt?"),
]
# What follows each item in the context render.
CONTEXT = "ðə sˈʌn ɹˈOz ˌOvəɹ ðə hˈɪlz, ənd ðə vˈɪlɪʤ wˈOk slˈOli tə ə kˈOld bɹˈIt mˈɔɹnɪŋ."
# For a request that is only the short item: a fixed carrier rendered after it, then cut off.
CARRIER = "ðæt wʌz ˈɔl."


def level(samples: np.ndarray) -> dict:
    """RMS over the speech (5 ms windows above -45 dBFS), in dBFS, and the speech's length."""
    x = samples.astype(np.float64) / 32767
    win = 120
    n = len(x) // win
    if n == 0:
        return {"rms_db": None, "speech_ms": 0}
    frames = x[: n * win].reshape(n, win)
    rms = np.sqrt((frames**2).mean(1))
    loud = rms > 10 ** (-45 / 20)
    if not loud.any():
        return {"rms_db": None, "speech_ms": 0}
    speech = frames[loud]
    return {"rms_db": round(20 * np.log10(np.sqrt((speech**2).mean())), 1),
            "peak_db": round(20 * np.log10(np.abs(speech).max()), 1),
            "speech_ms": int(loud.sum() * win / 24)}


def main():
    models, out = sys.argv[1], sys.argv[2]
    os.makedirs(out, exist_ok=True)
    synth = Synth(models, device="cuda", half=True, compile=False)
    vocab = json.load(open(os.path.join(models, "config.json"), encoding="utf-8"))["vocab"]

    def ids(ph):
        return [vocab[c] for c in ph if c in vocab]

    def render(ids_, voice, style_len):
        torch.manual_seed(0)
        r = next(iter(synth.run([Request(ids_, synth.style(voice, style_len), 100)])))[0]
        return np.frombuffer(r.pcm, dtype="<i2"), r.durations

    def trimmed(pcm, lead, tail):
        """Speech plus `lead` samples before and `tail` after (the server's trim, roughly)."""
        pcm = np.concatenate([pcm, np.zeros(tail, dtype=pcm.dtype)])  # room for the tail
        x = np.abs(pcm.astype(np.float64)) > 32767 * 10 ** (-45 / 20)
        if not x.any():
            return pcm
        a, b = int(np.argmax(x)), len(x) - int(np.argmax(x[::-1]))
        return pcm[max(0, a - lead) : b + tail]

    lead, gap = 1200, 9840  # 50 ms, and 410 ms after a sentence
    rows = []
    ctx = ids(CONTEXT)
    for name in VOICES:
        v = synth.voice_names.index(name)
        for label, ph in ITEMS:
            item = ids(ph)
            row = {"voice": name, "text": label, "ids": len(item)}
            clips = {}
            clips["alone"], _ = render(item, v, len(item))
            clips["long_row"], _ = render(item, v, LONG_ROW)
            full = item + [vocab[" "]] + ctx
            pcm, durations = render(full, v, len(full))
            # durations[0] is the leading boundary id; the item's ids follow it.
            end = sum(durations[: 1 + len(item)]) * SAMPLES_PER_FRAME
            clips["context"] = pcm[: end + SAMPLES_PER_FRAME * 4]
            ctx_alone, _ = render(ctx, v, len(ctx))
            clips["today"] = np.concatenate([trimmed(clips["alone"], lead, gap), trimmed(ctx_alone, lead, gap)])
            # Split after the item's last id (its punctuation): its frames, then the gap.
            head, tail = pcm[:end], pcm[end:]
            clips["split"] = np.concatenate([trimmed(head, lead, gap), trimmed(tail, lead, gap)])
            clips["merged"] = trimmed(pcm, lead, gap)
            # A fixed carrier after the item, cut off; and the item ending in "…" instead of its own
            # punctuation, alone.
            with_carrier = item + [vocab[" "]] + ids(CARRIER)
            pcm_c, durations_c = render(with_carrier, v, len(with_carrier))
            clips["carrier"] = pcm_c[: sum(durations_c[: 1 + len(item)]) * SAMPLES_PER_FRAME + SAMPLES_PER_FRAME * 4]
            clips["ellipsis"], _ = render(item[:-1] + [vocab["…"]], v, len(item))
            for k, pcm_k in clips.items():
                fn = f"{name}-{ITEMS.index((label, ph))}-{k}.wav"
                write_wav(os.path.join(out, fn), pcm_k.astype("<i2").tobytes())
                if k in ("alone", "long_row", "context", "carrier", "ellipsis"):
                    row[k] = level(pcm_k)
            rows.append(row)
            print(json.dumps(row, ensure_ascii=False), flush=True)
    with open(os.path.join(out, "short_words.json"), "w") as f:
        json.dump(rows, f, ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
