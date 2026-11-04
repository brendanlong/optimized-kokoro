"""First-time vs repeated latency against a running worker (or the HTTP server in front of it).

    python bench/fresh_latency.py worker SOCKET CONFIG_JSON [--n 200]
    python bench/fresh_latency.py http URL API_KEY [--n 200] [--offset 203]

Takes --n distinct real chunks from the corpus, in a fixed random order, and sends each once, then
again at once: the first is usually a frame count the worker has not run before (worker mode counts
them), the second a size it just ran. One request at a time. Through HTTP it
sends the chunk's text and times the first byte of streamed PCM.
"""

import argparse
import http.client
import json
import os
import statistics
import sys
import time
import urllib.parse

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402


def pick(n):
    seen, out = set(), []
    with open(sentences.CORPUS, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") not in ("a", "b"):
                continue
            for c in row.get("chunks", []):
                if c["phonemes"] not in seen:
                    seen.add(c["phonemes"])
                    out.append((c["text"], c["phonemes"]))
    rng = np.random.default_rng(1)
    return [out[i] for i in rng.permutation(len(out))[:n]]


def stats(xs):
    return (f"median {statistics.median(xs):6.1f} ms, p10 {np.percentile(xs, 10):6.1f}, "
            f"p90 {np.percentile(xs, 90):6.1f}, max {max(xs):6.1f}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["worker", "http"])
    ap.add_argument("target")
    ap.add_argument("extra", help="worker: config.json; http: API key")
    ap.add_argument("--n", type=int, default=200)
    ap.add_argument("--offset", type=int, default=0, help="skip this many chunks (another run's)")
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--json")
    args = ap.parse_args()
    items = pick(args.offset + args.n + 3)[args.offset :]
    warm, items = items[args.n :], items[: args.n]
    frames_seen, new_frames = set(), 0

    if args.mode == "worker":
        from kokoro_worker import protocol as P
        from kokoro_worker.client import Client
        from load import wait_for

        vocab = json.load(open(args.extra, encoding="utf-8"))["vocab"]
        client = wait_for(args.target, 900)
        voice = client.hello.voices.index(args.voice)

        def once(item):
            ids = [vocab[c] for c in item[1] if c in vocab]
            t = time.perf_counter()
            client.send(P.synthesize(0, ids, voice))
            kind, _, payload = client.receive()
            ms = 1000 * (time.perf_counter() - t)
            assert kind == "result", kind
            nonlocal new_frames
            frames = sum(payload[0])
            new_frames += frames not in frames_seen
            frames_seen.add(frames)
            return ms
    else:
        url = urllib.parse.urlparse(args.target)

        def once(item):
            body = json.dumps({"model": "kokoro", "voice": args.voice, "input": item[0],
                               "response_format": "pcm", "stream": True})
            conn = http.client.HTTPConnection(url.hostname, url.port, timeout=60)
            t = time.perf_counter()
            conn.request("POST", "/v1/audio/speech", body, {"Authorization": f"Bearer {args.extra}",
                                                            "Content-Type": "application/json"})
            resp = conn.getresponse()
            assert resp.status == 200, (resp.status, resp.read()[:200])
            resp.read(1)
            first = 1000 * (time.perf_counter() - t)
            resp.read()
            conn.close()
            return first

    for item in warm:  # connection setup, not part of the measurement
        once(item)
    new_frames = 0
    first, repeated = [], []
    for item in items:
        first.append(once(item))
        repeated.append(once(item))
    print(f"{args.mode}: {len(items)} distinct chunks" +
          (f", {new_frames} with a frame count not sent before" if args.mode == "worker" else ""))
    print(f"  first time: {stats(first)}")
    print(f"  repeated:   {stats(repeated)}", flush=True)
    if args.json:
        with open(args.json, "w") as f:
            json.dump({"first": first, "repeated": repeated}, f)


if __name__ == "__main__":
    main()
