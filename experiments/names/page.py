"""The listening page: a summary table, then each picked name's arms with audio. Writes OUT_DIR/body.html and copies the clips."""

import hashlib
import html
import json
import shutil
import sys
from pathlib import Path

LABELS = {"key": "Answer key (Tolkien Gateway IPA)", "current": "Current pipeline (espeak)", "rules": "Tolkien's rules",
          "haiku_misaki": "Haiku 5.5, Kokoro phonemes", "opus_misaki": "Opus 5.5, Kokoro phonemes",
          "haiku_ipa": "Haiku 5.5, IPA", "opus_ipa": "Opus 5.5, IPA"}
ORDER = ["current", "rules", "haiku_misaki", "haiku_ipa", "opus_misaki", "opus_ipa"]


def main(scores_path, plan_path, wav_dir, out_dir):
    scores = json.load(open(scores_path))
    plan = json.load(open(plan_path))
    out = Path(out_dir)
    (out / "audio").mkdir(parents=True, exist_ok=True)
    by_name = {arm: {r["name"]: r for r in scores[arm]["rows"]} for arm in scores}
    h = ["<p>How close each way of producing Kokoro's phonemes gets to Tolkien's intended pronunciation, "
         f"over {scores['current']['summary']['names']} names with IPA on Tolkien Gateway. Stress is the share of words "
         "with the primary stress on the same vowel as the key; consonant error rate is the edit distance between "
         "consonant sequences over the key's consonants.</p>",
         "<table><thead><tr><th>Arm</th><th>Stress right</th><th>Consonant error rate</th><th>Consonants exact</th></tr></thead><tbody>"]
    for arm in ORDER:
        s = scores[arm]["summary"]
        h.append(f"<tr><td>{LABELS[arm]}</td><td>{s['stress_correct']:.0%}</td><td>{s['consonant_error_rate']:.0%}</td>"
                 f"<td>{s['consonants_exact']:.0%}</td></tr>")
    h.append("</tbody></table>")
    h.append("<h2>Listen</h2><p>Each clip is “Then NAME arrived.” in af_heart. ✗ marks stress that differs from the key. "
             "Rows that produce the same phonemes share a clip.</p>")
    names = list(dict.fromkeys(r["name"] for r in plan))
    for name in names:
        h.append(f"<h3>{html.escape(name)}</h3><table class='arms'><tbody>")
        for r in (r for r in plan if r["name"] == name):
            k = hashlib.sha1(bytes(r["ids"])).hexdigest()[:16]
            src = Path(wav_dir) / f"{k}.wav"
            if src.exists():
                shutil.copy(src, out / "audio" / f"{k}.wav")
            row = by_name.get(r["arm"], {}).get(name)
            mark = "" if r["arm"] == "key" or row is None or row["stress_ok"] is not False else " ✗"
            word = r["phonemes"].removeprefix("ðˈɛn ").removesuffix(" əɹˈIvd.")
            h.append(f"<tr><td>{LABELS[r['arm']]}</td><td><code>{html.escape(word)}</code>{mark}</td>"
                     f"<td><audio controls preload='none' src='audio/{k}.wav'></audio></td></tr>")
        h.append("</tbody></table>")
    h.append("<style>table.arms td:first-child{width:16em} audio{height:2em;max-width:100%}</style>")
    (out / "body.html").write_text("\n".join(h))


if __name__ == "__main__":
    main(*sys.argv[1:])
