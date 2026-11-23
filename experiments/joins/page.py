"""The listening page: each sentence's three readings side by side. The split reading's parts are
joined here with SILENCE_MS of silence. Writes OUT_DIR/index.html and its audio.

    python3 page.py PLAN WAV_DIR OUT_DIR
"""

import hashlib
import html
import json
import sys
import wave
from pathlib import Path

SILENCE_MS = 150
LABELS = {"joined": "As the pipeline reads it", "comma": "Comma at each switch", "split": f"Parts rendered apart, {SILENCE_MS} ms between"}
LANGS = {"a": "English", "e": "Spanish", "f": "French", "h": "Hindi", "i": "Italian", "j": "Japanese", "z": "Chinese"}


def clip_id(ids, voice):
    return hashlib.sha1(bytes(ids) + voice.encode()).hexdigest()[:16]


def concat(paths, out):
    frames, params = [], None
    for i, p in enumerate(paths):
        with wave.open(str(p)) as w:
            params = w.getparams()
            if i:
                frames.append(b"\0" * (w.getframerate() * SILENCE_MS // 1000) * w.getsampwidth())
            frames.append(w.readframes(w.getnframes()))
    with wave.open(str(out), "wb") as w:
        w.setparams(params)
        w.writeframes(b"".join(frames))


def main(plan_path, wav_dir, out_dir):
    out = Path(out_dir)
    (out / "audio").mkdir(parents=True, exist_ok=True)
    rows = json.load(open(plan_path, encoding="utf-8"))
    h = ["<!doctype html><meta charset=utf-8><title>Joins between languages</title>",
         "<style>body{font-family:system-ui,sans-serif;max-width:60em;margin:2em auto;padding:0 1em;line-height:1.4}"
         "td{padding:.3em .6em;vertical-align:top}code{font-size:.9em}audio{height:2em}</style>",
         "<h1>Joins between languages</h1>",
         "<p>Each sentence switches language mid-sentence, read three ways. Which sounds most natural at the switch? "
         "The first is what the pipeline does now (PR #70): a space where the languages meet. The second adds a comma "
         "there, so the model pauses. The third renders each language's part on its own and puts "
         f"{SILENCE_MS} ms of silence between, like separate chunks.</p>"]
    for n in sorted({r["sentence"] for r in rows}):
        group = [r for r in rows if r["sentence"] == n]
        h.append(f"<h2>{html.escape(LANGS[group[0]['lang']])}, {html.escape(group[0]['voice'])}: {html.escape(group[0]['text'])}</h2><table>")
        for r in group:
            parts = [Path(wav_dir) / f"{clip_id(p['ids'], r['voice'])}.wav" for p in r["parts"]]
            name = f"{n:02d}_{r['condition']}.wav"
            concat(parts, out / "audio" / name)
            ps = " ‖ ".join(p["phonemes"] for p in r["parts"])
            h.append(f"<tr><td>{LABELS[r['condition']]}</td><td><audio controls preload=none src='audio/{name}'></audio></td>"
                     f"<td><code>{html.escape(ps)}</code></td></tr>")
        h.append("</table>")
    (out / "index.html").write_text("\n".join(h), encoding="utf-8")
    print(f"wrote {out / 'index.html'}")


if __name__ == "__main__":
    main(*sys.argv[1:])
