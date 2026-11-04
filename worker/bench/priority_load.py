"""Priorities and load shedding through the HTTP server: many low-priority streams, plus
occasional high-priority requests.

    python bench/priority_load.py URL API_KEY [--low-streams 32] [--high-every 0.4] [--seconds 60]
        [--low 10] [--high 200] [--json out.json]

Each low stream is a closed loop: it sends --low-chunks consecutive chunks of the corpus at priority
--low (4, a paragraph, or 1, a follow-up request for one sentence), reads the whole response as fast as it can, and sends the next; after a refusal (503 or
429) it waits --backoff seconds, as a client that fell back to another provider would before its
next request. High-priority requests arrive open loop, one every --high-every seconds on average
(exponential gaps), each one sentence at priority --high, the first audio a listener is waiting
for. Reports, per priority, time to first audio for served requests and how fast refusals come
back, and how many of each.
"""

import argparse
import datetime
import http.client
import json
import os
import random
import sys
import threading
import time
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402


def corpus(per_text):
    """Texts of `per_text` consecutive chunks of one corpus record each."""
    out = []
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") not in ("a", "b"):
                continue
            chunks = [c["text"] for c in row.get("chunks", [])]
            for i in range(0, len(chunks) - per_text + 1, per_text):
                out.append(" ".join(chunks[i : i + per_text]))
    return out


def pct(xs, p):
    if not xs:
        return float("nan")
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(p / 100 * len(xs)))]


def summary(xs):
    if not xs:
        return "none"
    return (f"n {len(xs):4d}  p50 {pct(xs, 50):7.1f}  p90 {pct(xs, 90):7.1f}  p99 {pct(xs, 99):7.1f}  "
            f"max {max(xs):7.1f} ms")


class Recorder:
    def __init__(self):
        self.lock = threading.Lock()
        self.rows = []  # (priority, status, ms to first byte or to the refusal, total ms, start)

    def add(self, *row):
        with self.lock:
            self.rows.append(row)


def speak(url, key, text, priority, voice):
    """(status, ms to the first byte of audio or the error response, ms in all)."""
    body = json.dumps({"model": "kokoro", "voice": voice, "input": text, "response_format": "pcm",
                       "stream": True, "priority": priority})
    conn = http.client.HTTPConnection(url.hostname, url.port, timeout=120)
    t = time.perf_counter()
    try:
        conn.request("POST", "/v1/audio/speech", body, {"Authorization": f"Bearer {key}",
                                                        "Content-Type": "application/json"})
        resp = conn.getresponse()
        if resp.status != 200:
            resp.read()
            ms = 1000 * (time.perf_counter() - t)
            return resp.status, ms, ms
        resp.read(1)
        first = 1000 * (time.perf_counter() - t)
        while resp.read(65536):
            pass
        return 200, first, 1000 * (time.perf_counter() - t)
    finally:
        conn.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("url")
    ap.add_argument("key")
    ap.add_argument("--low-streams", type=int, default=32)
    ap.add_argument("--low-chunks", type=int, default=4, help="corpus chunks per low-priority request")
    ap.add_argument("--high-every", type=float, default=0.4, help="mean seconds between high requests")
    ap.add_argument("--seconds", type=float, default=60)
    ap.add_argument("--warmup", type=float, default=10, help="seconds of load before measuring")
    ap.add_argument("--low", type=int, default=10)
    ap.add_argument("--high", type=int, default=200)
    ap.add_argument("--backoff", type=float, default=0.5)
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--json")
    ap.add_argument("--server-log", help="the server's log at RUST_LOG=debug, to compare its estimates")
    args = ap.parse_args()
    url = urllib.parse.urlparse(args.url)
    paragraphs, single = corpus(args.low_chunks), corpus(1)
    rec = Recorder()
    wall0 = time.time()
    t0 = time.perf_counter()
    stop = t0 + args.warmup + args.seconds

    def low_stream(i):
        rng = random.Random(i)
        while time.perf_counter() < stop:
            start = time.perf_counter() - t0
            status, first, total = speak(url, args.key, rng.choice(paragraphs), args.low, args.voice)
            rec.add(args.low, status, first, total, start)
            if status != 200:
                time.sleep(args.backoff)

    def high_one(text):
        start = time.perf_counter() - t0
        status, first, total = speak(url, args.key, text, args.high, args.voice)
        rec.add(args.high, status, first, total, start)

    threads = [threading.Thread(target=low_stream, args=(i,), daemon=True) for i in range(args.low_streams)]
    for t in threads:
        t.start()
    rng = random.Random(12345)
    highs = []
    while time.perf_counter() < stop:
        time.sleep(rng.expovariate(1 / args.high_every))
        h = threading.Thread(target=high_one, args=(rng.choice(single),), daemon=True)
        h.start()
        highs.append(h)
    for t in threads + highs:
        t.join(180)

    rows = [r for r in rec.rows if r[4] >= args.warmup]
    print(f"{args.low_streams} low streams of {args.low_chunks}-chunk requests at priority {args.low}, a high request (priority {args.high}) "
          f"every {args.high_every} s on average, {args.seconds:.0f} s measured after {args.warmup:.0f} s")
    out = {"args": vars(args), "rows": rows}
    for priority in (args.high, args.low):
        mine = [r for r in rows if r[0] == priority]
        served = [r[2] for r in mine if r[1] == 200]
        refused = [r[2] for r in mine if r[1] != 200]
        statuses = {s: sum(1 for r in mine if r[1] == s) for s in sorted({r[1] for r in mine})}
        print(f"priority {priority:3d}: {len(mine)} requests, statuses {statuses}")
        print(f"  first audio  {summary(served)}")
        print(f"  refusals     {summary(refused)}")
        out[str(priority)] = {"first_audio_ms": served, "refused_ms": refused, "statuses": statuses}
    if args.server_log:
        # The server's debug lines: "admitted a request at priority P: estimated first audio X ms".
        since = wall0 + args.warmup
        for priority in (args.high, args.low):
            est = []
            with open(args.server_log, encoding="utf-8") as f:
                for line in f:
                    if f"admitted a request at priority {priority}:" not in line:
                        continue
                    when = datetime.datetime.strptime(line.split()[0], "%Y-%m-%dT%H:%M:%S.%fZ")
                    if when.replace(tzinfo=datetime.timezone.utc).timestamp() >= since:
                        est.append(float(line.rsplit(" ", 2)[-2]))
            print(f"priority {priority:3d}: estimate at admission  {summary(est)}")
            out[str(priority)]["estimate_ms"] = est
    if args.json:
        with open(args.json, "w") as f:
            json.dump(out, f)


if __name__ == "__main__":
    main()
