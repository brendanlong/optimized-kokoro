"""Closed-loop load against a running worker shaped like the server's lookahead of 3, at one
priority, C streams.

    python bench/lookahead_load.py SOCKET CONFIG_JSON --concurrency 16 64 [--model front|fixed]
        [--chunks-per-request 8] [--requests N] [--json out.json]

--model front (default) does what the server does (crates/server/src/stream.rs): a stream reads
requests of --chunks-per-request consecutive corpus chunks, one after another, and keeps up to 3
chunks of the current one ahead of what it has received. A chunk is sent waiting if nothing of the
request is ahead of the listener when it is sent (the request's first chunk, or a stream that has
drained), prefetch otherwise; the flag isn't changed later. The listener reads as fast as audio
arrives, in order.

--model fixed keeps exactly one waiting and two prefetch requests out per stream: when one's audio
arrives, the stream sends its next chunk with the same flag. With C at least twice --max-batch,
enough waiting requests are queued at every take that the scheduler (waiting first) never reaches
the prefetches, so batches are all waiting and the rows the same as main's: an A/A control.

Reports throughput (audio seconds per wall second) and time to result (send to audio) for waiting
and prefetch chunks; for --model front also time to first audio per request.
"""

import argparse
import collections
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import sentences  # noqa: E402
from kokoro_worker import protocol as P  # noqa: E402
from load import pct, wait_for  # noqa: E402

LOOKAHEAD = 3


def stats(xs):
    if not xs:
        return {"n": 0}
    return {"n": len(xs), "p50": round(1000 * pct(xs, 50), 1), "p90": round(1000 * pct(xs, 90), 1),
            "p95": round(1000 * pct(xs, 95), 1), "max": round(1000 * max(xs), 1)}


class Stream:
    def __init__(self, k, position):
        self.slots = list(range(LOOKAHEAD * k, LOOKAHEAD * k + LOOKAHEAD))  # free slots
        self.position = position
        self.left = 0  # chunks of the current request not yet sent
        self.window = collections.deque()  # slots sent and not yet received in order
        self.arrived = set()
        self.request_start = None


def run(client, chunks, concurrency, n_requests, voice, model, per_request, offset=0):
    streams = [Stream(k, (offset + k * len(chunks) // concurrency) % len(chunks)) for k in range(concurrency)]
    sent_at, flag = {}, {}
    sent = 0
    latency = {True: [], False: []}
    first_audio = []
    audio_s, failures = 0.0, 0

    def send(s, slot, waiting):
        nonlocal sent
        chunk = chunks[s.position % len(chunks)]
        s.position += 1
        sent += 1
        sent_at[slot], flag[slot] = time.monotonic(), waiting
        client.send(P.synthesize(slot, chunk[1], voice, waiting=int(waiting),
                                 style_len=chunk[2] if len(chunk) > 2 else None))

    def fill(s):  # front model: top the window up
        while sent < n_requests and len(s.window) < LOOKAHEAD:
            if s.left == 0:
                if s.window:
                    return  # the next request starts once this one's audio is all in
                s.left, s.request_start = per_request, time.monotonic()
            slot = s.slots.pop()
            send(s, slot, waiting=not s.window)
            s.window.append(slot)
            s.left -= 1

    start = time.monotonic()
    for k, s in enumerate(streams):
        if model == "front":
            fill(s)
        else:
            for i, slot in enumerate(list(s.slots)):
                if sent < n_requests:
                    send(s, slot, waiting=i == 0)
    done = 0
    while done < n_requests:
        kind, slot, payload = client.receive()
        done += 1
        now = time.monotonic()
        latency_s = now - sent_at.pop(slot)
        if kind == "result":
            latency[flag[slot]].append(latency_s)
            audio_s += len(payload[1]) / 2 / P.SAMPLE_RATE
        else:
            failures += 1
        s = streams[slot // LOOKAHEAD]
        if model == "fixed":
            if sent < n_requests:
                send(s, slot, flag[slot])
            continue
        s.arrived.add(slot)
        while s.window and s.window[0] in s.arrived:  # the listener takes audio in order
            head = s.window.popleft()
            s.arrived.discard(head)
            s.slots.append(head)
            if s.request_start is not None:
                first_audio.append(now - s.request_start)
                s.request_start = None
        fill(s)
    wall = time.monotonic() - start
    out = {"model": model, "concurrency": concurrency, "requests": n_requests, "failures": failures,
           "wall_s": round(wall, 3), "audio_s": round(audio_s, 2), "x_realtime": round(audio_s / wall, 1),
           "waiting_ms": stats(latency[True]), "prefetch_ms": stats(latency[False])}
    if model == "front":
        out["first_audio_ms"] = stats(first_audio)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("socket")
    ap.add_argument("config")
    ap.add_argument("--concurrency", type=int, nargs="+", default=[16, 64])
    ap.add_argument("--model", choices=["front", "fixed"], default="front")
    ap.add_argument("--chunks-per-request", type=int, default=8)
    ap.add_argument("--requests", type=int, default=0, help="chunks per run; default 100 x streams, at least 1600")
    ap.add_argument("--warmup", type=int, default=0, help="chunks at 8 streams before measuring")
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--wait", type=float, default=600, help="seconds to wait for the socket")
    ap.add_argument("--json")
    args = ap.parse_args()

    vocab = json.load(open(args.config, encoding="utf-8"))["vocab"]
    chunks = sentences.load(vocab)
    client = wait_for(args.socket, args.wait)
    if client.hello.slots < LOOKAHEAD * max(args.concurrency + [8]):
        sys.exit("not enough slots")
    voice = client.hello.voices.index(args.voice)
    results = {"max_batch": client.hello.max_batch, "chunks": len(chunks), "runs": []}
    if args.warmup:
        run(client, chunks, 8, args.warmup, voice, args.model, args.chunks_per_request, offset=len(chunks) // 3)
    for c in args.concurrency:
        r = run(client, chunks, c, args.requests or max(1600, 100 * c), voice, args.model, args.chunks_per_request)
        results["runs"].append(r)
        print(json.dumps(r), flush=True)
    client.close()
    if args.json:
        with open(args.json, "w") as f:
            json.dump(results, f, indent=1)


if __name__ == "__main__":
    main()
