"""Closed-loop load against a running worker: C streams, each reading the corpus in order and
sending its next chunk as soon as the previous one's audio arrives (waiting, at the default priority).

    python bench/load.py SOCKET CONFIG_JSON --concurrency 4 --requests 400 [--json out.json]

Reports throughput (audio seconds per wall second) and per-request latency (send to audio).
"""

import argparse
import json
import os
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker import protocol as P  # noqa: E402
from kokoro_worker.client import Client  # noqa: E402


def wait_for(path: str, timeout: float) -> Client:
    deadline = time.monotonic() + timeout
    while True:
        try:
            return Client(path)
        except (FileNotFoundError, ConnectionRefusedError, EOFError):
            if time.monotonic() > deadline:
                raise
            time.sleep(1)


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(p / 100 * len(xs)))]


def run(client: Client, chunks, concurrency: int, n_requests: int, voice: int, offset: int = 0, speed: int = 100):
    streams = [(offset + k * len(chunks) // concurrency) % len(chunks) for k in range(concurrency)]
    # Each stream has one request out at a time, so its index is its slot.
    sent_at = {}
    sent = 0
    latencies, audio_s, longest_s = [], 0.0, 0.0
    failures = 0

    def send(stream: int):
        nonlocal sent
        ids = chunks[streams[stream] % len(chunks)][1]
        streams[stream] += 1
        sent += 1
        sent_at[stream] = time.monotonic()
        client.send(P.synthesize(stream, ids, voice, speed))

    start = time.monotonic()
    for k in range(min(concurrency, n_requests)):
        send(k)
    done = 0
    while done < n_requests:
        kind, slot, payload = client.receive()
        now = time.monotonic()
        done += 1
        latency = now - sent_at.pop(slot)
        if kind == "result":
            latencies.append(latency)
            audio_s += len(payload[1]) / 2 / P.SAMPLE_RATE
            longest_s = max(longest_s, len(payload[1]) / 2 / P.SAMPLE_RATE)
        else:
            failures += 1
        if sent < n_requests:
            send(slot)

    wall = time.monotonic() - start
    return {
        "concurrency": concurrency,
        "requests": n_requests,
        "failures": failures,
        "wall_s": round(wall, 3),
        "audio_s": round(audio_s, 2),
        "x_realtime": round(audio_s / wall, 1),
        "longest_s": round(longest_s, 2),
        "latency_ms": {
            "p50": round(1000 * statistics.median(latencies), 1),
            "p90": round(1000 * pct(latencies, 90), 1),
            "max": round(1000 * max(latencies), 1),
            "mean": round(1000 * statistics.mean(latencies), 1),
        },
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("socket")
    ap.add_argument("config")
    ap.add_argument("--concurrency", type=int, nargs="+", default=[1, 4, 16])
    ap.add_argument("--requests", type=int, default=0, help="per run; default 100 x concurrency, at least 200")
    ap.add_argument("--warmup", type=int, default=200, help="requests at concurrency 8 before measuring")
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--wait", type=float, default=600, help="seconds to wait for the socket")
    ap.add_argument("--json")
    args = ap.parse_args()

    vocab = json.load(open(args.config, encoding="utf-8"))["vocab"]
    chunks = sentences.load(vocab)
    client = wait_for(args.socket, args.wait)
    voice = client.hello.voices.index(args.voice)
    results = {"max_batch": client.hello.max_batch, "chunks": len(chunks), "runs": []}
    if args.warmup:
        w = run(client, chunks, 8, args.warmup, voice, offset=len(chunks) // 3)
        print(f"warmup: {w['x_realtime']}x", flush=True)
    for c in args.concurrency:
        n = args.requests or max(200, 100 * c)
        r = run(client, chunks, c, n, voice)
        results["runs"].append(r)
        print(json.dumps(r), flush=True)
    client.close()
    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=1)


if __name__ == "__main__":
    main()
