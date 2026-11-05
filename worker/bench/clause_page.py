"""The clause-split listening page, from bench/clause_joins.py's renders (job_clause_joins.sh).

    python bench/clause_page.py SENTENCES RESULTS_DIR PHONEMIZE OUT_DIR

PHONEMIZE is the pipeline's phonemize example (`cargo build --release -p kokoro-pipeline --example
phonemize`), used to recompute each variant's chunks so every join between two server sends is
known as a clause cut or a stitch crossfade. Writes OUT_DIR/body.html (an html fragment), audio as
OUT_DIR/audio/*.mp3 (ffmpeg) and OUT_DIR/pauses.json.

A pause is the run of 5 ms windows quieter than -45 dBFS (the trimming threshold) around a point:
a join between two sends, or for the whole render the middle of a clause mark's frames.
"""

import html
import json
import os
import subprocess
import sys

import numpy as np

RATE = 24000
SPACE, CLAUSE = 16, (1, 2, 3)
THRESHOLD = 32767 * 10 ** (-45 / 20)
WINDOW = 120
CAP30 = 30 * RATE


def render_ids(cap_samples, speed):
    frames = cap_samples // 600 * round(speed * 100) // 100
    return min(510, max(0, frames - 120) * 10 // 40)


def pieces(ids, max_len, prefer_clause):
    """stitch.rs `pieces`, with or without the clause preference."""
    out, start = [], 0
    while start < len(ids):
        end = start + max_len
        if end >= len(ids):
            end = len(ids)
        else:
            cut = None
            if prefer_clause:
                lo = max(start + max_len - max_len * 40 // 100, start + 2)
                cut = next((i for i in range(end, lo - 1, -1) if ids[i] == SPACE and ids[i - 1] in CLAUSE), None)
            if cut is None:
                sub = ids[start + 1 : end + 1]
                if SPACE in sub:
                    cut = start + 1 + len(sub) - 1 - sub[::-1].index(SPACE)
            if cut is not None:
                end = cut
        out.append((start, end))
        start = end + 1 if end < len(ids) and ids[end] == SPACE else end
    return out


def chunks(phonemize, text, limit):
    res = subprocess.run([phonemize, "a", str(limit)], input=json.dumps(text) + "\n", capture_output=True,
                         text=True, check=True)
    return json.loads(res.stdout)["chunks"]


def plan(phonemize, text, speed, new):
    """Per server send: (chunk index, clause marks before the chunk's end, is the chunk's last piece)."""
    ri = render_ids(CAP30, speed)
    limit = min(450, ri) if new else 450
    sends, marks = [], 0
    for ci, c in enumerate(chunks(phonemize, text, limit)):
        ids = c["ids"]
        marks += sum(1 for i in ids if i in CLAUSE)
        n = 1
        if len(c["phonemes"]) > 450 or len(ids) > ri:
            window = min(max(ri, 2), 510)
            n = len(pieces(ids, min(max(window * 450 // 510, 1), window - 1), new))
        sends += [(ci, c["text"], marks, k == n - 1) for k in range(n)]
    return sends


def quiet(pcm):
    n = len(pcm) // WINDOW
    env = np.abs(pcm[: n * WINDOW].astype(np.int32)).reshape(n, WINDOW).max(axis=1)
    return env <= THRESHOLD


def pause_at(q, sample):
    """ms of quiet windows around `sample` (0 if the window there is loud)."""
    w = min(sample // WINDOW, len(q) - 1)
    lo, hi = w, w
    if not q[w] and w > 0 and q[w - 1]:
        lo = hi = w - 1
    if not q[lo]:
        return 0
    while lo > 0 and q[lo - 1]:
        lo -= 1
    while hi + 1 < len(q) and q[hi + 1]:
        hi += 1
    return round((hi - lo + 1) * WINDOW * 1000 / RATE)


def trim(pcm, speed):
    """Kokoro-FastAPI's trimming of a sentence end (audio.rs trim_range, text ending in '.')."""
    a = pcm[24:-24]
    loud = np.nonzero(np.abs(a.astype(np.int32)) > THRESHOLD)[0]
    start = max(0, loud[0] - 1200)
    end = min(len(a), loud[-1] + int(np.ceil((9840 - 1200) / speed)))
    return a[start:end], 24 + start


def mp3(pcm, path):
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-f", "s16le", "-ar", str(RATE), "-ac", "1", "-i", "-",
                    "-b:a", "128k", path], input=pcm.astype("<i2").tobytes(), check=True)


VARIANTS = [
    ("main", "A. main today", "stitched at spaces; clause-split only above 450 phonemes, 410 ms × 0.8 padding"),
    ("b", "B. clause split, today's padding", "split at clauses to fit the cap, each piece trimmed as a sentence"),
    ("c", "C. clause split, model's comma timing", "this PR: cut after the comma's frames, next starts at its first id"),
    ("whole", "D. whole sentence", "one request, cap 60 s: Kokoro's own phrasing"),
]


def main():
    sentences, results, phonemize, out = sys.argv[1:5]
    rows = json.load(open(sentences, encoding="utf-8"))
    os.makedirs(f"{out}/audio", exist_ok=True)
    data = []
    for i, row in enumerate(rows):
        speed = row["speed"]
        whole = np.load(f"{results}/whole-{i}.npz")
        dur = whole["durations"]
        frames = np.concatenate([[0], np.cumsum(dur)])
        wq = quiet(whole["pcm"])
        mark_ids = [k for k, x in enumerate(row["ids"]) if x in CLAUSE]
        # Pause at each clause mark: the middle of its frames (duration index k + 1).
        comma = [pause_at(wq, int(600 * (frames[k + 1] + frames[k + 2]) / 2)) for k in mark_ids]
        trimmed, _ = trim(whole["pcm"], speed)
        mp3(trimmed, f"{out}/audio/whole-{i}.mp3")
        entry = {"id": row["id"], "text": row["text"], "speed": speed, "phonemes": len(row["phonemes"]),
                 "whole_comma_ms": comma, "variants": {}}
        for label, *_ in VARIANTS[:3]:
            r = np.load(f"{results}/{label}-{i}.npz")
            pcm, seg = r["pcm"], r["segments"]
            sends = plan(phonemize, row["text"], speed, label != "main")
            if len(sends) != len(seg):
                raise SystemExit(f"{label} {i}: {len(seg)} sends, expected {len(sends)}")
            q = quiet(pcm)
            joins = []
            for s in range(len(seg) - 1):
                at = int(np.sum(seg[: s + 1]))
                ci, text, marks, last = sends[s]
                clause = last  # between two chunks
                j = {"kind": "clause" if clause else "stitch", "ms": pause_at(q, at), "after": text[-40:]}
                if clause and 0 < marks <= len(comma):
                    j["whole_ms"] = comma[marks - 1]
                joins.append(j)
            mp3(pcm, f"{out}/audio/{label}-{i}.mp3")
            entry["variants"][label] = {"joins": joins, "seconds": round(len(pcm) / RATE, 2)}
        entry["variants"]["whole"] = {"seconds": round(len(trimmed) / RATE, 2)}
        data.append(entry)
    json.dump(data, open(f"{out}/pauses.json", "w"), indent=1)
    write_body(data, f"{out}/body.html")


def ms_list(joins, kind):
    return [j["ms"] for j in joins if j["kind"] == kind]


def stats(xs):
    return f"{np.mean(xs):.0f} ms (median {np.median(xs):.0f}, {min(xs)}–{max(xs)}, n={len(xs)})" if xs else "none"


def write_body(data, path):
    h = []
    c_joins = [j for d in data for j in d["variants"]["c"]["joins"] if j["kind"] == "clause"]
    b_joins = [j for d in data for j in d["variants"]["b"]["joins"] if j["kind"] == "clause"]
    same = [j["whole_ms"] for j in c_joins if "whole_ms" in j]
    all_commas = [m for d in data for m in d["whole_comma_ms"]]
    h.append("<p>Long sentences rendered by the real server and GPU worker (RTX 3060 Ti), voice af_heart, worker "
             "<code>--max-chunk-seconds 30</code>, for brendanlong/optimized-kokoro branch "
             "<code>brendanlong/clause-splits</code>. At a 30 s cap one request may hold 270 ids at speed 1 and 120 at "
             "speed 0.5, so these sentences must be split. The last two rows are at speed 0.5.</p>")
    h.append("<h2>Pauses at the joins</h2>")
    h.append("<p>A pause is the run of 5 ms windows quieter than −45 dBFS around a join between two of the server's "
             "sends (for D, around the middle of a clause mark's frames in the whole render).</p>")
    h.append("<table><thead><tr><th>What</th><th>Mean pause</th></tr></thead><tbody>")
    h.append(f"<tr><td>D: Kokoro's own pause at the clause marks where C splits</td><td>{stats(same)}</td></tr>")
    h.append(f"<tr><td>D: Kokoro's own pause at every clause mark (<code>, ; :</code>)</td><td>{stats(all_commas)}</td></tr>")
    h.append(f"<tr><td>C: clause joins (this PR)</td><td>{stats([j['ms'] for j in c_joins])}</td></tr>")
    h.append(f"<tr><td>B: clause joins, today's padding</td><td>{stats([j['ms'] for j in b_joins])}</td></tr>")
    a_st = [m for d in data for m in ms_list(d["variants"]["main"]["joins"], "stitch")]
    a_cl = [m for d in data for m in ms_list(d["variants"]["main"]["joins"], "clause")]
    h.append(f"<tr><td>A: stitch crossfades (main)</td><td>{stats(a_st)}</td></tr>")
    h.append(f"<tr><td>A: clause joins above 450 phonemes (main)</td><td>{stats(a_cl)}</td></tr>")
    h.append("</tbody></table>")
    h.append("<h2>Sentences</h2>")
    h.append("<p>Under each player: the pause at each join, in order; <em>s</em> marks a stitch crossfade, the rest are "
             "clause cuts, with Kokoro's own pause at that mark (from D) in brackets.</p>")
    h.append('<div class="wide"><table class="clips"><thead><tr><th>Sentence</th>')
    for _, name, note in VARIANTS:
        h.append(f'<th>{html.escape(name)}<br><span class="note">{html.escape(note)}</span></th>')
    h.append("</tr></thead><tbody>")
    for i, d in enumerate(data):
        h.append(f'<tr><td class="text">{html.escape(d["text"])}<br><span class="note">{d["phonemes"]} phonemes, '
                 f'speed {d["speed"]:g}</span></td>')
        for label, *_ in VARIANTS:
            v = d["variants"][label]
            cell = [f'<audio controls preload="none" src="audio/{label}-{i}.mp3"></audio>',
                    f'<span class="note">{v["seconds"]} s</span>']
            if label == "whole":
                cell.append('<span class="note">clause marks: ' + ", ".join(f"{m}" for m in d["whole_comma_ms"])
                            + " ms</span>")
            else:
                parts = []
                for j in v["joins"]:
                    if j["kind"] == "stitch":
                        parts.append(f'{j["ms"]}<em>s</em>')
                    else:
                        parts.append(f'{j["ms"]}' + (f' [{j["whole_ms"]}]' if "whole_ms" in j else ""))
                cell.append('<span class="note">joins: ' + (", ".join(parts) or "none") + " ms</span>")
            h.append("<td>" + "<br>".join(cell) + "</td>")
        h.append("</tr>")
    h.append("</tbody></table></div>")
    h.append("<style>.clips td{vertical-align:top}.clips td.text{min-width:18em;max-width:28em}"
             ".clips audio{width:14em}.note{font-size:.85em;color:var(--muted, #666)}"
             ".wide{overflow-x:auto}</style>")
    open(path, "w").write("\n".join(h))


if __name__ == "__main__":
    main()
