"""The native vs anglicized listening page. Writes OUT_DIR/body.html and copies the clips.

    python3 page.py PLAN.json WAV_DIR OUT_DIR
"""

import hashlib
import html
import json
import shutil
import sys
from pathlib import Path

LABELS = {"current": "Today (main pipeline)", "native": "Native", "anglicized": "Anglicized"}


def main(plan_path, wav_dir, out_dir):
    plan = json.load(open(plan_path, encoding="utf-8"))
    out = Path(out_dir)
    (out / "audio").mkdir(parents=True, exist_ok=True)
    h = ["<p>Each clip is “Then NAME arrived.” in <code>af_heart</code>, with the name read three ways: as the "
         "pipeline reads it today, in its own language's sounds (limited to sounds Kokoro has), and as an "
         "English speaker might say it. The native and anglicized readings are written by hand for this test; "
         "the native ones start from espeak-ng's reading in that language, corrected where espeak is wrong. "
         "“Today” for Nguyễn already includes the fix in PR #74. Clips with identical phonemes are the same clip.</p>",
         "<p>Things to listen for: do the native sounds that English lacks (Polish ɨ and ɕ, rolled r, x as in Bach, "
         "ø, nasal vowels) sound like a careful English speaker, or out of place? Do the diaspora names "
         "(Brzezinski, Kowalczyk, Nguyễn) sound wrong native?</p>"]
    names = list(dict.fromkeys(r["name"] for r in plan))
    for name in names:
        rows = [r for r in plan if r["name"] == name]
        note = rows[0]["note"]
        h.append(f"<h3>{html.escape(name)} <small>({html.escape(rows[0]['language'])})</small></h3>")
        if note:
            h.append(f"<p class='note'>{html.escape(note)}</p>")
        h.append("<table class='arms'><tbody>")
        for r in rows:
            k = hashlib.sha1(bytes(r["ids"]) + r.get("voice", "af_heart").encode()).hexdigest()[:16]
            src = Path(wav_dir) / f"{k}.wav"
            if src.exists():
                shutil.copy(src, out / "audio" / f"{k}.wav")
            word = r["phonemes"].removeprefix("ˈðɛn ").removesuffix(" əˈɹa͡ɪvd.")
            h.append(f"<tr><td>{LABELS[r['arm']]}</td><td><code>{html.escape(word)}</code></td>"
                     f"<td><audio controls preload='none' src='audio/{k}.wav'></audio></td></tr>")
        h.append("</tbody></table>")
    h.append("<style>table.arms td:first-child{width:11em} audio{height:2em;max-width:100%} "
             "p.note{color:var(--muted, inherit);margin:0 0 .4em}</style>")
    (out / "body.html").write_text("\n".join(h), encoding="utf-8")


if __name__ == "__main__":
    main(*sys.argv[1:])
