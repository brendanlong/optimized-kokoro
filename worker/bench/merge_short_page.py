"""The listening page for bench/merge_short.py's renders: audio per variant with its chunking, WER,
and the pause at each sentence boundary.

    python3 bench/merge_short_page.py PLAN RESULTS_DIR PAGE_DIR

Pauses: the run of 5 ms windows below -45 dBFS RMS at a sentence boundary. Between chunks it is the
end of one of the server's sends plus the start of the next. Inside a merged chunk it is measured on
the worker's rendering of the chunk's ids, around the end of the frames of the sentence's last id
(before the space joining it to the next). Also, for each sentence under 25 phonemes: its length
(frames of its ids) and level (RMS over its loud windows) rendered alone vs inside its merged chunk.
Writes PAGE_DIR/body.html, PAGE_DIR/audio/*.mp3 and PAGE_DIR/numbers.json.
"""

import hashlib
import html
import json
import os
import statistics
import subprocess
import sys

import numpy as np

WIN = 120  # 5 ms
SPACE_ID = 16
LABELS = {"today": "today (main)", "m25": "MIN 25", "m50": "MIN 50", "m80": "MIN 80",
          "p25": "MIN 25, paragraphs kept", "p50": "MIN 50, paragraphs kept", "p80": "MIN 80, paragraphs kept"}


def silent(x):
    n = len(x) // WIN
    frames = x[: n * WIN].astype(np.float64).reshape(n, WIN) / 32767
    return np.sqrt((frames**2).mean(1)) < 10 ** (-45 / 20)


def run_at(quiet, i):
    """Length in windows of the silent run at window i (or the nearest within 20 windows)."""
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


def level_db(x):
    x = x.astype(np.float64) / 32767
    n = len(x) // WIN
    if n == 0:
        return None
    f = x[: n * WIN].reshape(n, WIN)
    rms = np.sqrt((f**2).mean(1))
    loud = f[rms > 10 ** (-45 / 20)]
    return None if loud.size == 0 else float(20 * np.log10(np.sqrt((loud**2).mean())))


def main():
    plan_path, results, page = sys.argv[1:4]
    os.makedirs(f"{page}/audio", exist_ok=True)
    plan = json.load(open(plan_path, encoding="utf-8"))
    asr = json.load(open(f"{results}/asr.json", encoding="utf-8"))
    sentence_ids = {}
    pauses = {}  # variant -> list of (passage, boundary, ms, inside)
    shorts = []
    body = []
    totals = {}
    for p in plan:
        today = p["variants"]["today"]["chunks"]
        for chunks in today:
            for c in chunks:
                sentence_ids[c["texts"][0]] = c["ids"]
        section = [f'<h3 id="{p["id"]}">{html.escape(p["id"])}</h3>',
                   f'<p class="text">{html.escape(p["text"]).replace(chr(10) * 2, " <span class=para>¶</span> ")}</p>',
                   "<table class=variants><thead><tr><th>Variant</th><th>Audio</th><th>Pauses (ms)</th><th>Whisper</th></tr></thead><tbody>"]
        for name, v in p["variants"].items():
            data = np.load(f"{results}/{name}-{p['id']}.npz")
            pcm, seg_lens = data["pcm"], list(data["segments"])
            chunks = [c for r in v["chunks"] for c in r]
            segs = np.split(pcm, np.cumsum(seg_lens)[:-1])
            ok = len(segs) == len(chunks)
            # The pause after each sentence but the last, in order.
            row = []
            for ci, c in enumerate(chunks):
                if len(c["texts"]) > 1:
                    w = np.load(f"{results}/chunk-{key(c['ids'])}.npz")
                    wq, dur = silent(w["pcm"]), list(w["durations"])
                    pos = 0
                    for t in c["texts"][:-1]:
                        pos += len(sentence_ids[t])
                        # durations[0] is the leading boundary id; the sentence's ids are 1..pos.
                        end = sum(dur[: pos + 1]) * 600
                        row.append((run_at(wq, end // WIN) * 5, True))
                        pos += 1  # the space
                if ci + 1 < len(chunks) and ok:
                    row.append(((edge(silent(segs[ci]), True) + edge(silent(segs[ci + 1]), False)) * 5, False))
            for b, (ms, inside) in enumerate(row):
                pauses.setdefault(name, []).append((p["id"], b, ms, inside))
            # Short sentences: alone (today's chunk) vs in the merged chunk.
            if name != "today":
                for c in chunks:
                    if len(c["texts"]) < 2:
                        continue
                    w = np.load(f"{results}/chunk-{key(c['ids'])}.npz")
                    dur, wpcm = list(w["durations"]), w["pcm"]
                    pos = 0
                    for t in c["texts"]:
                        n = len(sentence_ids[t])
                        if n < 25:
                            a = sum(dur[: pos + 1]) * 600
                            b = sum(dur[: pos + 1 + n]) * 600
                            alone = np.load(f"{results}/chunk-{key(sentence_ids[t])}.npz")
                            ad = list(alone["durations"])
                            shorts.append({"variant": name, "passage": p["id"], "text": t, "ids": n,
                                           "alone_ms": sum(ad[1:-1]) * 25, "merged_ms": (b - a) // 24,
                                           "alone_db": level_db(alone["pcm"]),
                                           "merged_db": level_db(wpcm[a:b])})
                        pos += n + 1
            fn = f"{name}-{p['id']}.mp3"
            subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "s16le", "-ar", "24000", "-ac", "1", "-i", "-",
                            "-b:a", "64k", f"{page}/audio/{fn}"], input=pcm.astype("<i2").tobytes(), check=True)
            a = asr[f"{name}-{p['id']}"]
            e, n = totals.get(name, (0, 0))
            totals[name] = (e + a["errors"], n + a["words"])
            cut = " <b class=cut>|</b> ".join(html.escape(" ".join(c["texts"])) for c in chunks)
            pause_cell = ", ".join(f"<b>{ms}</b>" if inside else str(ms) for ms, inside in row)
            section.append(f"<tr><td>{LABELS[name]}</td><td><audio controls preload=none src=\"audio/{fn}\"></audio>"
                           f"<div class=chunks>{cut}</div></td><td>{pause_cell}</td>"
                           f"<td>{a['errors']} err<div class=hyp>{html.escape(a['hyp'])}</div></td></tr>")
        section.append("</tbody></table>")
        body.append("\n".join(section))
    # Summary numbers.
    summary = {}
    for name, rows in pauses.items():
        inside = [ms for *_, ms, i in rows if i]
        between = [ms for *_, ms, i in rows if not i]
        e, n = totals[name]
        summary[name] = {"wer": 100 * e / n, "errors": e, "words": n,
                         "inside_n": len(inside), "inside_median": statistics.median(inside) if inside else None,
                         "inside_range": (min(inside), max(inside)) if inside else None,
                         "between_n": len(between), "between_median": statistics.median(between) if between else None,
                         "between_range": (min(between), max(between)) if between else None}
    # The same boundaries, today vs each variant, where the variant merged them.
    today_at = {(pid, b): ms for pid, b, ms, _ in pauses["today"]}
    for name, rows in pauses.items():
        diffs = [ms - today_at[(pid, b)] for pid, b, ms, i in rows if i]
        summary[name]["inside_minus_today_median"] = statistics.median(diffs) if diffs else None
    json.dump({"summary": summary, "pauses": pauses, "shorts": shorts}, open(f"{page}/numbers.json", "w"), indent=1, default=int)
    table = ["<table><thead><tr><th>Variant</th><th>WER</th><th>Pauses inside merged chunks: n, median ms (range)</th>"
             "<th>Pauses between chunks: n, median ms (range)</th><th>Merged pause minus today's at the same boundary (median ms)</th></tr></thead><tbody>"]
    for k, s in summary.items():
        inside = f"{s['inside_n']}: {s['inside_median']:.0f} ({s['inside_range'][0]}–{s['inside_range'][1]})" if s["inside_n"] else "none"
        between = f"{s['between_n']}: {s['between_median']:.0f} ({s['between_range'][0]}–{s['between_range'][1]})" if s["between_n"] else "none"
        diff = f"{s['inside_minus_today_median']:+.0f}" if s["inside_minus_today_median"] is not None else "–"
        table.append(f"<tr><td>{LABELS[k]}</td><td>{s['wer']:.1f}% ({s['errors']}/{s['words']})</td><td>{inside}</td><td>{between}</td><td>{diff}</td></tr>")
    table.append("</tbody></table>")
    short_rows = ["<table><thead><tr><th>Sentence</th><th>Ids</th><th>Variant</th><th>Length alone / merged (ms)</th><th>Level alone / merged (dBFS)</th></tr></thead><tbody>"]
    for s in shorts:
        if s["variant"] not in ("m50",):
            continue
        short_rows.append(f"<tr><td>{html.escape(s['text'])}</td><td>{s['ids']}</td><td>{LABELS[s['variant']]}</td>"
                          f"<td>{s['alone_ms']} / {s['merged_ms']}</td><td>{s['alone_db']:.1f} / {s['merged_db']:.1f}</td></tr>")
    short_rows.append("</tbody></table>")
    open(f"{page}/summary.html", "w").write("\n".join(table) + "\n<!--short-->\n" + "\n".join(short_rows))
    open(f"{page}/passages.html", "w").write("\n".join(body))
    for k, s in summary.items():
        print(k, json.dumps(s, default=int))
    if shorts:
        m = [s for s in shorts if s["variant"] == "m50"]
        print("m50 shorts: median length ratio alone/merged",
              statistics.median(s["alone_ms"] / s["merged_ms"] for s in m),
              "median level diff", statistics.median(s["alone_db"] - s["merged_db"] for s in m))


if __name__ == "__main__":
    main()
