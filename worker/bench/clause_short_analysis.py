"""How much a clause rendered alone differs from the same ids inside its sentence, by clause length
(bench/clause_short.py's renders).

    python bench/clause_short_analysis.py INPUTS RESULTS_DIR [--json OUT]

For each clause: the log-mel spectrogram of its ids' frames alone (boundary frames cut) against the same
ids' frames in the whole sentence, stretched to the same length: the correlation of the two (1 = same
sound), and the alone render's length over the in-sentence length. With its own style row (as the
server sends a chunk) and with the sentence's.
"""

import json
import sys

import numpy as np

RATE, HOP, NFFT, MELS = 24000, 240, 1024, 64
BINS = [(1, 5), (6, 10), (11, 15), (16, 20), (21, 25), (26, 30), (31, 40), (41, 60), (61, 100), (101, 510)]


def mel_filters():
    def hz_to_mel(f):
        return 2595 * np.log10(1 + f / 700)

    pts = 700 * (10 ** (np.linspace(hz_to_mel(60), hz_to_mel(11000), MELS + 2) / 2595) - 1)
    bins = np.floor((NFFT + 1) * pts / RATE).astype(int)
    fb = np.zeros((MELS, NFFT // 2 + 1))
    for m in range(1, MELS + 1):
        lo, c, hi = bins[m - 1], bins[m], bins[m + 1]
        fb[m - 1, lo:c] = (np.arange(lo, c) - lo) / max(c - lo, 1)
        fb[m - 1, c:hi] = (hi - np.arange(c, hi)) / max(hi - c, 1)
    return fb


FB = mel_filters()


def logmel(x):
    x = x.astype(np.float64) / 32768
    if len(x) < NFFT:
        x = np.pad(x, (0, NFFT - len(x)))
    n = 1 + (len(x) - NFFT) // HOP
    frames = np.stack([x[i * HOP:i * HOP + NFFT] * np.hanning(NFFT) for i in range(n)])
    return np.log(FB @ (np.abs(np.fft.rfft(frames, axis=1)) ** 2).T + 1e-8)


def stretch(m, n):
    t = np.linspace(0, m.shape[1] - 1, n)
    return np.stack([np.interp(t, np.arange(m.shape[1]), row) for row in m])


def span(dur, first, count):
    """Samples of ids first..first+count (duration index + 1 for the leading boundary)."""
    cum = np.concatenate([[0], np.cumsum(dur)]) * 600
    return int(cum[first + 1]), int(cum[first + 1 + count])


def main():
    inputs, results = sys.argv[1:3]
    rows = json.load(open(inputs, encoding="utf-8"))
    out = []
    for i, row in enumerate(rows):
        r = np.load(f"{results}/short-{i}.npz")
        for k, c in enumerate(row["clauses"]):
            n = len(c["ids"])
            a, b = span(r["whole_dur"], c["offset"], n)
            ref = logmel(r["whole"][a:b])
            item = {"ids": n, "text": c["text"]}
            for kind in ("own", "row"):
                pa, pb = span(r[f"{kind}_dur_{k}"], 0, n)
                got = stretch(logmel(r[kind + "_" + str(k)][pa:pb]), ref.shape[1])
                item[kind] = float(np.corrcoef(ref.ravel(), got.ravel())[0, 1])
                item[kind + "_len"] = (pb - pa) / max(b - a, 1)
            out.append(item)
    print(f"{len(out)} clauses; correlation of log-mel alone vs in the sentence (1 = same), and length ratio")
    print(f"{'ids':>9} {'n':>4} {'own row: median':>16} {'p10':>6} {'len':>5} {'sentence row: median':>21} {'p10':>6}")
    for lo, hi in BINS:
        xs = [o for o in out if lo <= o["ids"] <= hi]
        if not xs:
            continue
        own = [o["own"] for o in xs]
        row = [o["row"] for o in xs]
        ln = [o["own_len"] for o in xs]
        print(f"{lo:>4}-{hi:<4} {len(xs):>4} {np.median(own):>16.3f} {np.percentile(own, 10):>6.3f} "
              f"{np.median(ln):>5.2f} {np.median(row):>21.3f} {np.percentile(row, 10):>6.3f}")
    worst = sorted(out, key=lambda o: o["own"])[:12]
    print("lowest (own row):", [(o["ids"], round(o["own"], 3), o["text"][:30]) for o in worst])
    if "--json" in sys.argv:
        json.dump(out, open(sys.argv[sys.argv.index("--json") + 1], "w"), ensure_ascii=False, indent=0)


if __name__ == "__main__":
    main()
