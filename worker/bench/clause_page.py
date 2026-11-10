"""The clause-split listening page, from bench/clause_joins.py's renders (job_clause_joins.sh).

    python bench/clause_page.py SENTENCES RESULTS_DIR PHONEMIZE OUT_DIR [SHORT_INPUTS]

PHONEMIZE is the pipeline's phonemize example (`cargo build --release -p kokoro-pipeline --example
phonemize`), used to recompute each variant's chunks so every join between two server sends is
known as a clause cut or a stitch crossfade. Writes OUT_DIR/body.html (an html fragment), audio as
OUT_DIR/audio/*.mp3 (ffmpeg) and OUT_DIR/pauses.json.

A pause is the run of 5 ms windows quieter than -45 dBFS (the trimming threshold) around a point:
a join between two sends, or for the whole render the middle of a clause mark's frames.

With SHORT_INPUTS (bench/clause/short.json, rendered by bench/clause_short.py in the same job), the page
also has short clauses alone against the same words in their sentence, and how much longer a clause
alone takes by its length (bench/clause_short_analysis.py).
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
        ranges = [(0, len(ids))]
        if len(c["phonemes"]) > 450 or len(ids) > ri:
            window = min(max(ri, 2), 510)
            ranges = pieces(ids, min(max(window * 450 // 510, 1), window - 1), new)
        # For a stitched piece, how many of the chunk's words come before the cut after it.
        sends += [(ci, c["text"], marks, k == len(ranges) - 1, ids[:end].count(SPACE) + 1)
                  for k, (_, end) in enumerate(ranges)]
    return sends


def marked_text(sends):
    """The sentence with | at each clause cut and ‖ at each stitch crossfade (at the word the piece
    ends with, counted in the phonemes' words)."""
    out, cuts = [], set()
    for k, (ci, text, _, last, words) in enumerate(sends):
        if not last:
            cuts.add(words)
        if last:
            ws = text.split()
            out.append(" ".join(w + (" ‖" if j + 1 in cuts else "") for j, w in enumerate(ws)))
            cuts = set()
    return " | ".join(out)


def final_clause_mark(ids):
    """audio.rs final_clause_mark."""
    for i in range(len(ids) - 1, -1, -1):
        if ids[i] in CLAUSE:
            return i
        if ids[i] > SPACE:
            return None
    return None


def join_like_c(pcm, dur, ids, first, last, speed):
    """The part of one chunk's rendering the server keeps (audio.rs keep_range) where the chunk is cut
    from its sentence before it unless `first`, and after it unless `last` (then its text ends a
    sentence)."""
    a = pcm[24:-24]
    loud = np.nonzero(np.abs(a.astype(np.int32)) > THRESHOLD)[0]
    start = 24 + (max(0, loud[0] - 1200) if first else loud[0])
    end = 24 + min(len(a), loud[-1] + int(np.ceil((9840 - 1200) / speed)))
    mark = None if last else final_clause_mark(ids)
    if mark is not None:
        end = int(600 * np.sum(dur[: mark + 2]))
    return pcm[start:end]


def clauses(sentence):
    import clause_joins

    return clause_joins.clauses(sentence)


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
    """Kokoro-FastAPI's trimming of a sentence end (audio.rs keep_range without cuts, text ending in '.')."""
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
    ("c", "C. clause split, model's comma timing", "this PR: cut after the comma's frames, next starts at its first sound"),
    ("whole", "D. whole sentence", "one request, cap 60 s: Kokoro's own phrasing"),
    ("e", "E. every clause its own chunk", "bench only: each clause rendered alone, joined as C"),
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
                ci, text, marks, last, _ = sends[s]
                clause = last  # between two chunks
                j = {"kind": "clause" if clause else "stitch", "ms": pause_at(q, at), "after": text[-40:]}
                if clause and 0 < marks <= len(comma):
                    j["whole_ms"] = comma[marks - 1]
                joins.append(j)
            mp3(pcm, f"{out}/audio/{label}-{i}.mp3")
            entry["variants"][label] = {"joins": joins, "seconds": round(len(pcm) / RATE, 2),
                                        "split": marked_text(sends)}
        entry["variants"]["whole"] = {"seconds": round(len(trimmed) / RATE, 2), "split": row["text"]}
        cl = row["clauses"]
        segs = [join_like_c(whole[f"clause_{k}"], whole[f"clause_dur_{k}"], ids, k == 0, k + 1 == len(cl), speed)
                for k, ids in enumerate(cl)]
        pcm = np.concatenate(segs)
        q = quiet(pcm)
        joins, at, marks = [], 0, 0
        texts = clauses(row["text"])
        for k, seg in enumerate(segs[:-1]):
            at += len(seg)
            marks += sum(1 for x in cl[k] if x in CLAUSE)
            j = {"kind": "clause", "ms": pause_at(q, at), "after": texts[k][-40:]}
            if 0 < marks <= len(comma):
                j["whole_ms"] = comma[marks - 1]
            joins.append(j)
        mp3(pcm, f"{out}/audio/e-{i}.mp3")
        entry["variants"]["e"] = {"joins": joins, "seconds": round(len(pcm) / RATE, 2), "split": " | ".join(texts)}
        data.append(entry)
    json.dump(data, open(f"{out}/pauses.json", "w"), indent=1)
    short = shorts(sys.argv[5], results, out) if len(sys.argv) > 5 else None
    he = he_clips(rows, results, out)
    write_body(data, f"{out}/body.html", short, he)


def find(ids, sub):
    return next((k for k in range(len(ids) - len(sub) + 1) if ids[k:k + len(sub)] == sub), None)


def peakiness(x):
    """Median over 10 ms frames of the strongest 1-8 kHz bin over the mean one: a ringing tone is a
    narrow peak."""
    x = x.astype(np.float64)
    n = 1 + max(0, len(x) - 1024) // 240
    out = []
    for i in range(n):
        f = np.abs(np.fft.rfft(np.pad(x[i * 240:i * 240 + 1024], (0, max(0, 1024 - len(x[i * 240:i * 240 + 1024])))) * np.hanning(1024)))
        band = f[43:342]
        if band.mean() > 0:
            out.append(band.max() / band.mean())
    return float(np.median(out)) if out else 0.0


def he_clips(rows, results, out):
    """Row 4's "he,": where it falls in each render, with its frames and a ringing measure."""
    i = next(k for k, r in enumerate(rows) if r["id"] == "gutenberg-moby-dick-01077")
    row, r = rows[i], np.load(f"{results}/whole-{i}.npz")
    texts = clauses(row["text"])
    k = texts.index("he,")
    he = row["clauses"][k]
    found = []

    def clip(name, pcm, dur, ids, label):
        at = find(list(ids), he)
        cum = np.concatenate([[0], np.cumsum(dur)]) * 600
        a, b = int(cum[at + 1]), int(cum[at + 1 + len(he)])
        lo, hi = max(0, a - 12000), min(len(pcm), b + 12000)
        mp3(pcm[lo:hi], f"{out}/audio/he-{name}.mp3")
        found.append({"name": name, "label": label, "ms": round((b - a) * 1000 / RATE),
                      "peak": round(peakiness(pcm[a:b]), 1), "at": at, "of": len(ids)})

    clip("whole", r["pcm"], r["durations"], row["ids"], "D: the whole sentence")
    for c, ids in enumerate(row["chunks"]):
        if find(list(ids), he) is not None:
            clip("c", r[f"chunk_{c}"], r[f"chunk_dur_{c}"], ids, f"C's chunk {c + 1} of {len(row['chunks'])}")
    clip("e", r[f"clause_{k}"], r[f"clause_dur_{k}"], he, "E: “he,” alone")
    return {"row": i + 1, "found": found, "chunks": [len(c) for c in row["chunks"]]}


def shorts(inputs, results, out):
    """Examples of clauses alone vs in their sentence, and the length ratio by clause length."""
    import clause_short_analysis as csa

    rows = json.load(open(inputs, encoding="utf-8"))
    ratios = []
    examples = []
    for i, row in enumerate(rows):
        r = np.load(f"{results}/short-{i}.npz")
        for k, c in enumerate(row["clauses"]):
            n = len(c["ids"])
            a, b = csa.span(r["whole_dur"], c["offset"], n)
            pa, pb = csa.span(r[f"own_dur_{k}"], 0, n)
            ra, rb = csa.span(r[f"row_dur_{k}"], 0, n)
            ratios.append((n, (pb - pa) / max(b - a, 1), (rb - ra) / max(b - a, 1)))
            pick = c["text"] == "he," or (i < 10 and n < 12) or (i < 10 and 40 <= n <= 45 and len(examples) < 9)
            if pick and len(examples) < 10:
                name = f"short-{i}-{k}"
                alone = r[f"own_{k}"]
                mp3(alone, f"{out}/audio/{name}-alone.mp3")
                mp3(r["whole"][max(0, a - 2400):b + 2400], f"{out}/audio/{name}-in.mp3")
                examples.append({"name": name, "text": c["text"], "ids": n, "sentence": row["text"],
                                 "alone_s": round((pb - pa) / RATE, 2), "in_s": round((b - a) / RATE, 2)})
    table = []
    for lo, hi in [(1, 5), (6, 10), (11, 15), (16, 20), (21, 24), (25, 30), (31, 40), (41, 60), (61, 510)]:
        xs = [x for n, x, _ in ratios if lo <= n <= hi]
        ys = [y for n, _, y in ratios if lo <= n <= hi]
        if xs:
            table.append((f"{lo}–{hi}", len(xs), float(np.median(xs)), float(np.percentile(xs, 90)),
                          float(np.median(ys))))
    return {"examples": examples, "table": table, "clauses": len(ratios), "sentences": len(rows)}


def stats(xs):
    return f"{np.mean(xs):.0f} ms (median {np.median(xs):.0f}, {min(xs)}–{max(xs)}, n={len(xs)})" if xs else "none"


def joins_of(data, label, kind):
    return [j for d in data for j in d["variants"][label]["joins"] if j["kind"] == kind]


def summary(data, h):
    c_joins = joins_of(data, "c", "clause")
    same = [j["whole_ms"] for j in c_joins if "whole_ms" in j]
    rows = [
        ("D: Kokoro's own pause at the clause marks where B and C cut", same),
        ("D: Kokoro's own pause at every clause mark (<code>, ; :</code>)", [m for d in data for m in d["whole_comma_ms"]]),
        ("C: clause joins (this PR)", [j["ms"] for j in c_joins]),
        ("C minus D, join by join", [j["ms"] - j["whole_ms"] for j in c_joins if "whole_ms" in j]),
        ("B: clause joins, today's padding", [j["ms"] for j in joins_of(data, "b", "clause")]),
        ("E: every clause cut, joined as C", [j["ms"] for j in joins_of(data, "e", "clause")]),
        ("E minus D, join by join", [j["ms"] - j["whole_ms"] for j in joins_of(data, "e", "clause") if "whole_ms" in j]),
        ("A: stitch crossfades (main)", [j["ms"] for j in joins_of(data, "main", "stitch")]),
        ("A: clause joins above 450 phonemes (main)", [j["ms"] for j in joins_of(data, "main", "clause")]),
    ]
    for what, xs in rows:
        h.append(f"<tr><td>{what}</td><td>{stats(xs)}</td></tr>")


def write_short(short, h):
    h.append("<h2>Short clauses alone</h2>")
    h.append(f"<p>Each clause of {short['sentences']} Gutenberg sentences ({short['clauses']} clauses, the sentences above "
             "first) rendered by itself, as a chunk is sent (its own style row), against the same ids inside the "
             "whole sentence. Kokoro drawls a short input: the frames of its ids last much longer alone. From 25 "
             "phonemes on a clause alone takes about as long as in its sentence, so no chunk cut from a sentence is "
             "now shorter than that (<code>MIN_CLAUSE_CHUNK</code>). The last column renders the clause with the whole "
             "sentence's style row instead, which doesn't help.</p>")
    h.append("<table><thead><tr><th>Clause ids</th><th>Clauses</th><th>Alone ÷ in sentence, median</th>"
             "<th>90th percentile</th><th>With the sentence's style row, median</th></tr></thead><tbody>")
    for rng, n, med, p90, row in short["table"]:
        h.append(f"<tr><td>{rng}</td><td>{n}</td><td>{med:.2f}×</td><td>{p90:.2f}×</td><td>{row:.2f}×</td></tr>")
    h.append("</tbody></table>")
    h.append("<p>Examples: the clause alone (as the reference would render it if it were cut out by itself), and the "
             "same words cut from the whole sentence with 100 ms either side. <code>he,</code> is from row 4, "
             "<em>Not so the sailor…</em>.</p>")
    h.append('<table class="clips"><thead><tr><th>Clause</th><th>Alone</th><th>In its sentence</th></tr></thead><tbody>')
    for e in short["examples"]:
        h.append(f'<tr><td><b>{html.escape(e["text"])}</b><br><span class="note">{e["ids"]} ids; from '
                 f'“{html.escape(e["sentence"][:80])}…”</span></td>'
                 f'<td><audio controls preload="none" src="audio/{e["name"]}-alone.mp3"></audio><br>'
                 f'<span class="note">ids {e["alone_s"]} s</span></td>'
                 f'<td><audio controls preload="none" src="audio/{e["name"]}-in.mp3"></audio><br>'
                 f'<span class="note">ids {e["in_s"]} s</span></td></tr>')
    h.append("</tbody></table>")


def write_he(he, h):
    h.append("<h2>Row 4: “, he, ”</h2>")
    h.append(f"<p>In C (and B, which renders the same chunks) the sentence is two chunks of {he['chunks'][0]} and "
             f"{he['chunks'][1]} ids, cut after “misery,”. “he,” is inside the first, far from the cut, so in C it is "
             "neither a chunk of its own nor next to a seam. A stitches this sentence (one crossfade, between “his” and "
             "“misery,”), also far from it. Each clip is “he,” with 0.5 s either side; the ringing measure is "
             "the median, over 10 ms frames of the word, of the strongest 1–8 kHz bin over the mean one.</p>")
    h.append("<table><thead><tr><th>Render</th><th>“he,” at id</th><th>Its frames</th><th>Ringing measure</th>"
             "<th>Clip</th></tr></thead><tbody>")
    for f in he["found"]:
        h.append(f'<tr><td>{html.escape(f["label"])}</td><td>{f["at"]} of {f["of"]}</td><td>{f["ms"]} ms</td>'
                 f'<td>{f["peak"]}</td><td><audio controls preload="none" src="audio/he-{f["name"]}.mp3"></audio>'
                 "</td></tr>")
    h.append("</tbody></table>")


def write_body(data, path, short=None, he=None):
    h = []
    h.append("<p>Long sentences rendered by the real server and GPU worker (RTX 3060 Ti), voice af_heart, worker "
             "<code>--max-chunk-seconds 30</code>, for brendanlong/optimized-kokoro branch "
             "<code>brendanlong/clause-splits</code>. At a 30 s cap one request may hold 270 ids at speed 1 and 120 at "
             "speed 0.5, so these sentences must be split. The last two rows are at speed 0.5.</p>")
    h.append("<ul><li><b>A</b> is main: the chunker cuts only above 450 phonemes, so each sentence is one chunk "
             "stitched from overlapping pieces crossfaded at a space (and the 468-phoneme one is also cut at a "
             "clause, with today's trimming).</li>"
             "<li><b>B</b> cuts at clauses to fit the cap and trims every chunk as Kokoro-FastAPI trims a sentence: "
             "about 280 ms × 1/speed kept after a comma, 50 ms before the next chunk.</li>"
             "<li><b>C</b> (this branch) ends a chunk right after its comma's frames, from the worker's durations, "
             "and starts the next at its first sound above −45 dBFS. (Starting it at the end of its leading boundary "
             "id's frames, the first try, clicked: the first word had begun there, up to 4,140 in amplitude.)</li>"
             "<li><b>D</b> is the whole sentence in one request to a worker at a 60 s cap, trimmed as a sentence: "
             "how Kokoro itself phrases it.</li>"
             "<li><b>E</b> (for judging by ear only, not something the server does) cuts at every clause mark: each "
             "clause rendered alone at the sentence's speed, joined the C way. Short clauses here are exactly what "
             "the 25-phoneme minimum now prevents.</li></ul>"
             "<p>Under each player, the text shows where that variant cuts: <b>|</b> a clause cut, <b>‖</b> a stitch "
             "crossfade (after the word the piece ends with).</p>")
    h.append("<h2>Pauses at the joins</h2>")
    h.append("<p>A pause is the run of 5 ms windows quieter than −45 dBFS around a join between two of the server's "
             "sends (for D, around the middle of a clause mark's frames in the whole render). Mean (median, range, "
             "count).</p>")
    for speed in sorted({d["speed"] for d in data}, reverse=True):
        group = [d for d in data if d["speed"] == speed]
        h.append(f"<h3>Speed {speed:g}</h3>")
        h.append("<table><thead><tr><th>What</th><th>Pause</th></tr></thead><tbody>")
        summary(group, h)
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
            if label != "whole":
                cell.append(f'<span class="split">{html.escape(v["split"])}</span>')
            if label == "whole":
                cell.append('<span class="note">clause marks: ' + ", ".join(f"{m}" for m in d["whole_comma_ms"])
                            + " ms</span>")
            else:
                parts = []
                for j in v["joins"]:
                    if j["kind"] == "stitch":
                        parts.append(f'{j["ms"]}<em>s</em>')
                    else:
                        parts.append(f'{j["ms"]}' + (f' [{j["whole_ms"]}]' if "whole_ms" in j else "")
                                     + f' after “…{html.escape(j["after"][-20:])}”')
                cell.append('<span class="note">joins: ' + (", ".join(parts) or "none") + " ms</span>")
            h.append("<td>" + "<br>".join(cell) + "</td>")
        h.append("</tr>")
    h.append("</tbody></table></div>")
    if he:
        write_he(he, h)
    if short:
        write_short(short, h)
    h.append("<style>.clips td{vertical-align:top}.clips td.text{min-width:18em;max-width:28em}"
             ".clips audio{width:14em}.note{font-size:.85em;color:var(--muted, #666)}"
             ".wide{overflow-x:auto}.split{font-size:.8em;display:block;max-width:22em}</style>")
    open(path, "w").write("\n".join(h))


if __name__ == "__main__":
    main()
