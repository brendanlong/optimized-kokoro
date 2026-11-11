"""Listening page for a short-rows run: metrics, loss curves, and the clips.

    python3 train/make_page.py RESULTS_DIR OUT_DIR TEMPLATE_HTML

RESULTS_DIR is the job's results/ (fit/ and score/); TEMPLATE_HTML has {{TITLE}} and {{BODY}}.
Writes OUT_DIR/index.html and copies the clips to OUT_DIR/wav/.
"""

import html
import json
import os
import shutil
import sys

VARIANTS = [("orig", "original row, alone"), ("fitted", "fitted row, alone"), ("context", "in context (target)")]


def esc(s):
    return html.escape(str(s))


def audio(fn):
    return f'<audio controls preload="none" src="wav/{esc(fn)}"></audio>'


def curve_svg(log, weights):
    """Training loss (total, smoothed over 5 logged points) and held-out loss, inline SVG."""
    w, h, pad = 640, 260, 44
    train = [r for r in log if r["split"] == "train"]
    test = [r for r in log if r["split"] == "test"]
    tr = [(r["step"], sum(weights[k] * r[k] for k in weights)) for r in train]
    sm = [(s, sum(v for _, v in tr[max(0, i - 4): i + 1]) / len(tr[max(0, i - 4): i + 1])) for i, (s, _) in enumerate(tr)]
    te = [(r["step"], r["total"]) for r in test]
    pts = sm + te
    if not pts:
        return ""
    x1 = max(s for s, _ in pts) or 1
    lo, hi = min(v for _, v in pts) * 0.95, max(v for _, v in pts) * 1.05

    def X(s):
        return pad + (w - 2 * pad) * s / x1

    def Y(v):
        return h - pad - (h - 2 * pad) * (v - lo) / (hi - lo)

    out = [f'<svg viewBox="0 0 {w} {h}" width="100%" style="max-width:{w}px" role="img" '
           f'aria-label="Loss against step: training (smoothed) and held-out">']
    for i in range(5):
        v = lo + (hi - lo) * i / 4
        out.append(f'<line x1="{pad}" x2="{w - pad}" y1="{Y(v):.1f}" y2="{Y(v):.1f}" stroke="#e1e0d9" '
                   f'style="stroke:var(--grid)"/><text x="{pad - 6}" y="{Y(v) + 4:.1f}" font-size="11" '
                   f'text-anchor="end" fill="#555" style="fill:var(--ink)">{v:.2f}</text>')
    for s in range(0, x1 + 1, max(1, x1 // 5)):
        out.append(f'<text x="{X(s):.1f}" y="{h - pad + 16}" font-size="11" text-anchor="middle" fill="#555" '
                   f'style="fill:var(--ink)">{s}</text>')
    out.append(f'<text x="{w / 2}" y="{h - 6}" font-size="12" text-anchor="middle" fill="#555" style="fill:var(--ink)">step</text>')
    path = " ".join(f"{'M' if i == 0 else 'L'}{X(s):.1f},{Y(v):.1f}" for i, (s, v) in enumerate(sm))
    out.append(f'<path d="{path}" fill="none" stroke="#2a78c8" stroke-width="2"/>')
    path = " ".join(f"{'M' if i == 0 else 'L'}{X(s):.1f},{Y(v):.1f}" for i, (s, v) in enumerate(te))
    out.append(f'<path d="{path}" fill="none" stroke="#d1603d" stroke-width="2"/>')
    for s, v in te:
        out.append(f'<circle cx="{X(s):.1f}" cy="{Y(v):.1f}" r="3.5" fill="#d1603d"/>')
    out.append(f'<text x="{w - pad}" y="{pad - 18}" font-size="12" text-anchor="end" fill="#2a78c8">training (batch mean, smoothed)</text>')
    out.append(f'<text x="{w - pad}" y="{pad - 4}" font-size="12" text-anchor="end" fill="#d1603d">held-out phrases</text>')
    out.append("</svg>")
    return "\n".join(out)


def metrics_table(summary):
    rows = []
    for bucket, s in summary.items():
        if not s.get("phrases"):
            continue
        label = {"all": "all", "no_numbers": "no numbers/letters"}.get(bucket, f"{bucket} phonemes")
        for name, title in VARIANTS:
            v = s.get(name, {})
            cells = [v.get("rms_db_vs_context"), v.get("abs_rms_db_vs_context"), v.get("frames_vs_context"),
                     v.get("abs_frames_vs_context"), v.get("speech_len_ratio_vs_context"), v.get("wer_pct"),
                     v.get("exact_pct")]
            if name == "context":
                cells = ["", "", "", "", "", v.get("wer_pct"), v.get("exact_pct")]
            rows.append(f"<tr><td>{esc(label)} ({s['phrases']})</td><td>{esc(title)}</td>"
                        + "".join(f"<td>{'' if c is None else esc(c)}</td>" for c in cells) + "</tr>")
    return ("<table><thead><tr><th>phrases</th><th>variant</th><th>RMS dB vs context</th><th>|RMS dB|</th>"
            "<th>frames vs context</th><th>|frames|</th><th>speech length ratio (median)</th><th>WER %</th>"
            "<th>exact %</th></tr></thead><tbody>" + "\n".join(rows) + "</tbody></table>")


def gap_table(phrases):
    """Per length bucket, how far each alone render is from the in-context one, next to how far the
    two in-context renders (different fillers) are from each other: the floor."""
    import math

    def mean(xs):
        xs = [x for x in xs if x is not None]
        return sum(xs) / len(xs) if xs else float("nan")

    def speech_gap(a, b):
        return abs(math.log(a["speech_ms"] / b["speech_ms"])) if a["speech_ms"] and b["speech_ms"] else None

    def db_gap(a, b):
        return abs(a["rms_db"] - b["rms_db"]) if a["rms_db"] is not None and b["rms_db"] is not None else None

    rows = []
    for lo, hi in [(1, 5), (6, 10), (11, 20), (21, 30), (1, 30)]:
        sel = [p for p in phrases if lo <= p["n"] <= hi]
        if not sel:
            continue
        cells = []
        for gap in (speech_gap, db_gap):
            for k in ("orig", "fitted"):
                cells.append(mean([gap(p["level"][k], p["level"]["context"]) for p in sel]))
            cells.append(mean([gap(p["level"]["context_all"][1], p["level"]["context_all"][0]) for p in sel
                               if len(p["level"]["context_all"]) > 1]))
        for k in ("orig", "fitted"):
            cells.append(mean([abs(p["frames"][k] - p["frames"]["context"]) for p in sel]))
        for k in ("orig", "fitted", "context"):
            cells.append(100 * mean([p["asr"][k]["errors"] == 0 for p in sel]) if "asr" in sel[0] else float("nan"))
        speech = [f"{100 * (math.exp(c) - 1):.1f}%" for c in cells[:3]]
        rest = [f"{c:.2f}" for c in cells[3:6]] + [f"{c:.1f}" for c in cells[6:8]] + [f"{c:.0f}%" for c in cells[8:]]
        rows.append(f"<tr><td>{lo}&ndash;{hi} ({len(sel)})</td>" + "".join(f"<td>{c}</td>" for c in speech + rest) + "</tr>")
    return ("<table><thead><tr><th rowspan=2>phonemes (phrases)</th><th colspan=3>speech length gap</th>"
            "<th colspan=3>loudness gap, dB</th><th colspan=2>|frames &minus; context|</th>"
            "<th colspan=3>Whisper exact</th></tr><tr><th>original</th><th>fitted</th><th>floor</th>"
            "<th>original</th><th>fitted</th><th>floor</th><th>original</th><th>fitted</th>"
            "<th>original</th><th>fitted</th><th>context</th></tr></thead><tbody>" + "\n".join(rows) + "</tbody></table>")


def main():
    res, out, template = sys.argv[1:4]
    findings = open(sys.argv[4], encoding="utf-8").read() if len(sys.argv) > 4 else ""
    os.makedirs(os.path.join(out, "wav"), exist_ok=True)
    m = json.load(open(os.path.join(res, "score", "metrics.json"), encoding="utf-8"))
    fit = json.load(open(os.path.join(res, "fit", "summary.json"), encoding="utf-8"))
    log = [json.loads(line) for line in open(os.path.join(res, "fit", "log.jsonl"))]
    for fn in os.listdir(os.path.join(res, "score", "wav")):
        shutil.copyfile(os.path.join(res, "score", "wav", fn), os.path.join(out, "wav", fn))

    b = []
    b.append("""<p>Kokoro renders very short inputs badly: alone, "No." comes out louder and stretched compared
with the same word cut from a render where a sentence follows it. This prototype changes only
<code>af_heart</code>'s voice pack rows for inputs of 1&ndash;30 phonemes (rows 0&ndash;29), fitted so that
a short phrase rendered alone sounds like the same phrase rendered in context. Model weights and
rows 30+ are untouched, so anything of 31 or more phonemes renders exactly as before.</p>
""" + findings + """
<h2 id="method">Method</h2>
<ul>
<li><b>Data</b>: short phrases through the real pipeline (crates/pipeline, example <code>phonemize</code>):
short sentences and sentence beginnings from public-domain books, interjections, letters, numbers and
headings; up to 90 per length; 12% of each length held out.</li>
<li><b>Teacher</b>: the frozen model (fp32, excitation noise off) renders <i>phrase + filler sentence</i>
(4 different fillers of 60&ndash;100 phonemes) with the row for the whole length; the phrase's durations,
F0/energy curves and audio are cut from the start through its final punctuation's frames.</li>
<li><b>Student</b>: the phrase alone with its own (trainable) row. Losses: L1 of the duration predictor's
pre-rounding output against the teacher's; multi-resolution L1 of log STFT magnitudes of the student decoded
with the teacher's durations, against the teacher's cut (the student's trailing boundary frames are not
scored); L1 of the 25 ms RMS envelope in dB; L1 against the teacher's F0 and energy curves; a small pull of
each row toward its original. A first run that also had spectral convergence (as the brief suggested) came
out 1.2 dB quieter than the target: that term rewards shrinking the output wherever student and target
aren't perfectly correlated. It was replaced by the envelope term.</li>
<li><b>Scoring</b> (this page): the deployed forward pass (fp16, noise on, seed 0) on held-out phrases;
the in-context target uses filler sentences the fit never saw.</li>
</ul>""")
    b.append("<h2 id='metrics'>Held-out metrics</h2>")
    b.append("<p>Each gap is the mean over phrases of how far the phrase rendered alone is from the same "
             "phrase in context (first held-out filler). <i>Speech length</i>: total length of 5 ms windows "
             "above &minus;45 dBFS, as a ratio (shown as a percentage difference, from the mean |log ratio|). "
             "<i>Loudness</i>: RMS over those windows. <i>Floor</i>: the same gap between the two in-context "
             "renders (two different held-out fillers), i.e. how much context alone moves it. <i>Frames</i>: "
             "the phrase's predicted durations (25 ms each, final punctuation included, boundary ids not) against "
             "the in-context durations averaged over both fillers. <i>Whisper exact</i>: share of clips that "
             "Whisper small.en transcribes word for word (numbers under 1000 spelled out on both sides).</p>")
    b.append(gap_table(m["phrases"]))
    b.append("<details><summary>Full metrics (signed means, WER)</summary>"
             "<p>WER counts at most the reference's word count per clip, so one Whisper loop doesn't swamp it.</p>"
             + metrics_table(m["summary"]) + "</details>")
    b.append(f"<p>Rows 30+ unchanged: <b>{m['rows_30_plus_unchanged']}</b>; the {len(m['long_renders_identical'])} "
             f"filler sentences (60&ndash;100 phonemes) render bit-identically with the patched pack: "
             f"<b>{all(m['long_renders_identical'])}</b>.</p>")
    b.append("<h2 id='loss'>Loss curves</h2>")
    a = fit["args"]
    weights = {"stft": 1.0, "level": a.get("w_level", 0.0), "dur": a["w_dur"], "f0": a["w_f0"], "n": a["w_n"]}
    weights = {k: v for k, v in weights.items() if v}
    b.append("<p>Weighted total (" + " + ".join(f"{v} &times; {k}" for k, v in weights.items())
             + "), teacher-forced, fp32, noise off; the held-out loss uses held-out fillers.</p>")
    b.append(curve_svg(log, weights))
    keys = ["total", *weights]
    before, after = fit["test_before"], fit["test_after"]
    b.append("<table><thead><tr><th>held-out</th>" + "".join(f"<th>{k}</th>" for k in keys)
             + "</tr></thead><tbody>"
             + "".join(f"<tr><td>{lab}</td>" + "".join(f"<td>{r[k]}</td>" for k in keys) + "</tr>"
                       for lab, r in (("original rows", before), ("fitted rows", after)))
             + "</tbody></table>")
    pl_b, pl_a = before["per_len"], after["per_len"]
    b.append("<details><summary>Held-out total loss per phrase length</summary><table><thead><tr><th>phonemes</th>"
             "<th>original</th><th>fitted</th></tr></thead><tbody>"
             + "".join(f"<tr><td>{n}</td><td>{pl_b[n]}</td><td>{pl_a.get(n, '')}</td></tr>" for n in pl_b)
             + "</tbody></table></details>")

    b.append("<h2 id='chunked'>A short phrase, then a sentence, as separate chunks</h2>")
    b.append("<p>What the server does today when a short sentence precedes a longer one: each chunk rendered "
             "alone, trimmed, joined with the sentence gap. Only the first chunk changes between the "
             "first two columns; the third is both rendered in one request, for reference.</p>")
    b.append("<table><thead><tr><th>text</th><th>original rows</th><th>fitted rows</th><th>one request</th></tr></thead><tbody>")
    for c in m["chunked"]:
        b.append(f"<tr><td>{esc(c['text'])}</td>" + "".join(f"<td>{audio(c['clips'][k])}</td>" for k in ("orig", "fitted", "together")) + "</tr>")
    b.append("</tbody></table>")

    b.append("<h2 id='phrases'>Held-out phrases</h2>")
    b.append("<p>Two per length, plus the phrases the earlier short-words page measured. Under each clip: "
             "what Whisper heard, and RMS dB.</p>")
    b.append("<table><thead><tr><th>phrase</th><th>n</th>" + "".join(f"<th>{t}</th>" for _, t in VARIANTS)
             + "</tr></thead><tbody>")
    for p in m["phrases"]:
        if not p["listen"]:
            continue
        cells = []
        for k, _ in VARIANTS:
            heard = p.get("asr", {}).get(k, {})
            lv = p["level"][k]
            mark = "" if heard.get("errors", 0) == 0 else " &#x2717;"
            cells.append(f"<td>{audio(p['clips'][k])}<br><small>&ldquo;{esc(heard.get('text', '')[:80])}&rdquo;{mark} "
                         f"&middot; {'' if lv['rms_db'] is None else round(lv['rms_db'], 1)} dB &middot; "
                         f"{p['frames'][k] if k != 'context' else round(p['frames'][k], 1)} frames</small></td>")
        b.append(f"<tr><td>{esc(p['text'])}<br><small><code>{esc(p['phonemes'])}</code></small></td>"
                 f"<td>{p['n']}</td>{''.join(cells)}</tr>")
    b.append("</tbody></table>")
    b.append(f"<p><small>Fit: {fit['args']['steps']} steps of {fit['args']['batch']} phrases, Adam lr "
             f"{fit['args']['lr']}, teacher {fit['teacher_s']} s, training {fit['train_s']} s on an RTX 3060 Ti. "
             f"Held-out fillers: {esc(' / '.join(m['fillers']))}</small></p>")
    style = ("<style>table{border-collapse:collapse;font-size:0.9em;margin:1em 0}td,th{border-bottom:1px solid "
             "var(--grid);padding:4px 6px;vertical-align:top;text-align:left}audio{width:180px;height:32px}"
             "</style>")
    page = open(template, encoding="utf-8").read()
    page = page.replace("{{TITLE}}", "Fitting Kokoro's short voice-pack rows").replace("{{BODY}}", style + "\n".join(b))
    with open(os.path.join(out, "index.html"), "w", encoding="utf-8") as f:
        f.write(page)
    print("wrote", os.path.join(out, "index.html"))


if __name__ == "__main__":
    main()
