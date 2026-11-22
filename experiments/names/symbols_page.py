"""The symbol-set listening page: per name, current / English symbols / native symbols, one column per voice."""

import hashlib
import html
import json
import shutil
import sys
from pathlib import Path

ARMS = {"current": "Current pipeline", "english": "Rules, English symbols", "native": "Rules, native symbols"}


def clip_id(ids, voice):  # as worker/bench/name_listen.py
    return hashlib.sha1(bytes(ids) + voice.encode()).hexdigest()[:16]


def main(plan_path, wav_dir, out_dir):
    plan = json.load(open(plan_path))
    out = Path(out_dir)
    (out / "audio").mkdir(parents=True, exist_ok=True)
    voices = list(dict.fromkeys(r["voice"] for r in plan))
    h = ["<p>Tolkien's Appendix E rules, written two ways. <b>English symbols</b> use only the sounds misaki uses for "
         "English (English r, ɛ and ɔ for e and o, no ch as in Bach). <b>Native symbols</b> use sounds from Kokoro's other "
         "languages: rolled <code>r</code>, pure <code>e</code> and <code>o</code>, <code>x</code> for ch and ht, "
         "<code>y</code> for Sindarin y (French u), and <code>ː</code> for long vowels. Each clip is “Then NAME arrived.”</p>",
         "<table><thead><tr><th>Name</th><th></th><th>Phonemes</th>" + "".join(f"<th>{v}</th>" for v in voices) + "</tr></thead><tbody>"]
    for name in dict.fromkeys(r["name"] for r in plan):
        for arm, label in ARMS.items():
            rows = [r for r in plan if r["name"] == name and r["arm"] == arm]
            word = rows[0]["phonemes"].removeprefix("ðˈɛn ").removesuffix(" əɹˈIvd.")
            cells = []
            for r in rows:
                k = clip_id(r["ids"], r["voice"])
                shutil.copy(Path(wav_dir) / f"{k}.wav", out / "audio" / f"{k}.wav")
                cells.append(f"<td><audio controls preload='none' src='audio/{k}.wav'></audio></td>")
            first = f"<td rowspan='3'><b>{html.escape(name)}</b></td>" if arm == "current" else ""
            h.append(f"<tr>{first}<td>{label}</td><td><code>{html.escape(word)}</code></td>{''.join(cells)}</tr>")
    h.append("</tbody></table><style>audio{height:2em;width:11em} td{vertical-align:middle}</style>")
    (out / "body.html").write_text("\n".join(h))


if __name__ == "__main__":
    main(*sys.argv[1:])
