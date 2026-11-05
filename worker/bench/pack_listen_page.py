"""The listening page for bench/pack_listen.py's renders.

    python3 bench/pack_listen_page.py PLAN RESULTS_DIR PAGE_DIR

Writes PAGE_DIR/body.html (render it with the html-doc template), PAGE_DIR/audio/*.mp3 and
PAGE_DIR/numbers.json. Pauses are runs of 5 ms windows below -45 dBFS RMS. Between chunks: the end
of one of the server's sends plus the start of the next. Inside a chunk: on the worker's rendering
of the chunk's ids, at the end of the frames of a unit's last id (before the space joining it to
the next unit), nearest silent run within 100 ms.
"""

import hashlib
import html
import json
import os
import statistics
import subprocess
import sys

import numpy as np

WIN = 120
LABELS = {"today": "today (main)", "min50": "MIN 50 (this PR's first version)"}


def label(v):
    return LABELS.get(v, f"greedy, soft size {v[1:]}")


def silent(x):
    n = len(x) // WIN
    frames = x[: n * WIN].astype(np.float64).reshape(n, WIN) / 32767
    return np.sqrt((frames**2).mean(1)) < 10 ** (-45 / 20)


def run_at(quiet, i):
    for d in range(21):
        for j in (i - d, i + d):
            if 0 <= j < len(quiet) and quiet[j]:
                a = b = j
                while a > 0 and quiet[a - 1]:
                    a -= 1
                while b + 1 < len(quiet) and quiet[b + 1]:
                    b += 1
                return b - a + 1
    return 0


def edge(quiet, from_end):
    q = quiet[::-1] if from_end else quiet
    n = 0
    while n < len(q) and q[n]:
        n += 1
    return n


def key(ids):
    return hashlib.sha1(bytes(ids)).hexdigest()[:12]


def stats(xs):
    if not xs:
        return "–"
    return f"{statistics.median(xs):.0f} ({min(xs)}–{max(xs)}), n={len(xs)}"


def main():
    plan_path, results, page = sys.argv[1:4]
    os.makedirs(f"{page}/audio", exist_ok=True)
    plan = json.load(open(plan_path, encoding="utf-8"))
    asr = json.load(open(f"{results}/asr.json", encoding="utf-8"))
    speed = json.load(open(f"{results}/speed.json", encoding="utf-8"))
    numbers = {"speed": speed, "sets": {}}
    body = {}
    for set_name, s in plan.items():
        variants = list(s["passages"][0]["variants"])
        agg = {v: {"errors": 0, "words": 0, "ttfa": [], "sentence": [], "clause": [], "between": [], "joins": [], "chunks": 0,
                   "first_ids": []} for v in variants}
        sections = []
        for p in s["passages"]:
            rows = []
            for v, chunks in p["variants"].items():
                k = f"{set_name}-{v}-{p['id']}"
                data = np.load(f"{results}/{k}.npz")
                pcm, segs = data["pcm"], np.split(data["pcm"], np.cumsum(data["segments"])[:-1])
                ttfa = sorted(1000 * data["ttfa"][1:])  # the first run warms this text's shapes
                a = agg[v]
                a["ttfa"].append(statistics.median(ttfa))
                a["chunks"] += len(chunks)
                a["first_ids"].append(len(chunks[0]["ids"]))
                pauses = []
                for ci, c in enumerate(chunks):
                    w = np.load(f"{results}/chunk-{key(c['ids'])}.npz")
                    q, dur = silent(w["pcm"]), list(w["durations"])
                    pos = 0
                    for u in c["units"][:-1]:
                        pos += len(u["ids"])
                        ms = run_at(q, sum(dur[: pos + 1]) * 600 // WIN) * 5
                        kind = "clause" if u["after"] else "sentence"
                        a[kind].append(ms)
                        pauses.append(f"<b>{ms}</b>" + ("<sup>c</sup>" if u["after"] else ""))
                        pos += 1
                    if ci + 1 < len(chunks) and len(segs) == len(chunks):
                        ms = (edge(silent(segs[ci]), True) + edge(silent(segs[ci + 1]), False)) * 5
                        clause = c["units"][-1]["after"]
                        a["joins" if clause else "between"].append(ms)
                        pauses.append(str(ms) + ("<sup>c</sup>" if clause else ""))
                r = asr[k]
                a["errors"] += r["errors"]
                a["words"] += r["words"]
                fn = f"{k}.mp3"
                subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "s16le", "-ar", "24000", "-ac", "1", "-i", "-",
                                "-b:a", "64k", f"{page}/audio/{fn}"], input=pcm.astype("<i2").tobytes(), check=True,
                               stderr=subprocess.DEVNULL)
                cut = " <b class=cut>|</b> ".join(f"{html.escape(c['text'])} <span class=n>{len(c['ids'])}</span>" for c in chunks)
                rows.append(f"<tr><td>{html.escape(label(v))}</td><td><audio controls preload=none src=\"audio/{fn}\"></audio>"
                            f"<div class=chunks>{cut}</div></td><td>{', '.join(pauses)}</td>"
                            f"<td>{statistics.median(ttfa):.0f}</td><td>{r['errors']}<div class=hyp>{html.escape(r['hyp'])}</div></td></tr>")
            text = html.escape(p["text"]).replace("\n\n", " <span class=para>¶</span> ")
            sections.append(f'<h3 id="{set_name}-{p["id"]}">{html.escape(p["id"])}</h3>\n<p class=text>{text}</p>\n'
                            "<table><thead><tr><th>Variant</th><th>Audio, and its chunks (ids)</th><th>Pauses, ms</th>"
                            "<th>First audio, ms</th><th>Whisper errors</th></tr></thead><tbody>" + "".join(rows) + "</tbody></table>")
        summary = ["<table><thead><tr><th>Variant</th><th>WER</th><th>First audio, median ms (range over passages)</th>"
                   "<th>First chunk, mean ids</th><th>Chunks</th><th>Pause after a sentence, inside a chunk: median ms (range)</th>"
                   "<th>At a clause mark inside a chunk</th><th>Between chunks, at a sentence end</th><th>Between chunks, at a clause cut</th></tr></thead><tbody>"]
        nums = {}
        for v, a in agg.items():
            t = a["ttfa"]
            nums[v] = {"wer": 100 * a["errors"] / a["words"], "errors": a["errors"], "words": a["words"],
                       "ttfa_median": statistics.median(t), "ttfa_range": [min(t), max(t)], "chunks": a["chunks"],
                       "first_ids_mean": statistics.mean(a["first_ids"]),
                       "sentence": a["sentence"], "clause": a["clause"], "between": a["between"], "joins": a["joins"]}
            summary.append(f"<tr><td>{html.escape(label(v))}</td><td>{nums[v]['wer']:.1f}% ({a['errors']}/{a['words']})</td>"
                           f"<td>{statistics.median(t):.0f} ({min(t):.0f}–{max(t):.0f})</td><td>{nums[v]['first_ids_mean']:.0f}</td>"
                           f"<td>{a['chunks']}</td><td>{stats(a['sentence'])}</td><td>{stats(a['clause'])}</td><td>{stats(a['between'])}</td><td>{stats(a['joins'])}</td></tr>")
        summary.append("</tbody></table>")
        numbers["sets"][set_name] = nums
        body[set_name] = ("\n".join(summary), "\n".join(sections))
    speed_rows = "".join(f"<tr><td>{x['ids']}</td><td>{x['ms']:.1f}</td><td>{x['min_ms']:.1f}</td></tr>" for x in speed)
    xs = np.array([x["ids"] for x in speed], dtype=float)
    ys = np.array([x["ms"] for x in speed])
    slope, intercept = np.polyfit(xs, ys, 1)
    numbers["speed_fit"] = {"ms_per_id": slope, "ms_fixed": intercept}
    json.dump(numbers, open(f"{page}/numbers.json", "w"), indent=1, default=float)
    open(f"{page}/parts.json", "w").write(json.dumps({"demo": body["demo"], "default": body["default"],
                                                      "speed": f"<table><thead><tr><th>Ids</th><th>Median ms</th><th>Fastest ms</th></tr></thead><tbody>{speed_rows}</tbody></table>",
                                                      "fit": [slope, intercept]}))
    for set_name, nums in numbers["sets"].items():
        for v, n in nums.items():
            print(set_name, v, f"WER {n['wer']:.1f}% ({n['errors']}/{n['words']})", f"ttfa {n['ttfa_median']:.0f} ({n['ttfa_range'][0]:.0f}-{n['ttfa_range'][1]:.0f})",
                  f"first {n['first_ids_mean']:.0f}", f"chunks {n['chunks']}", "sentence", stats(n["sentence"]),
                  "clause", stats(n["clause"]), "between", stats(n["between"]), "joins", stats(n["joins"]))
    print(f"decode: {slope:.3f} ms/id + {intercept:.1f} ms")


if __name__ == "__main__":
    main()
