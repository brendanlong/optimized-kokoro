"""Listening test for packing sentences into chunks (crates/server/src/pack.rs).

    python bench/pack_listen.py plan PASSAGES CHUNKS_EXAMPLE OUT_PLAN    (locally, no GPU)
    python bench/pack_listen.py http URL API_KEY PLAN OUT_DIR SET VARIANT  (in the job, per server binary)
    python bench/pack_listen.py worker SOCKET PLAN OUT_DIR                (in the job)
    python bench/pack_listen.py asr PLAN OUT_DIR                          (in the job)

Two sets of passages (PASSAGES, bench/pack_listen/passages.json):
  demo     deliberately tiny limits, so every effect is audible in short clips: the worker at
           --max-chunk-seconds 10 (a chunk limit of 70 ids at speed 1) and soft sizes 30 and 60
  default  the shipped limits: the worker at 60 s (450 phonemes) and soft sizes 90, 130 and 200
Variants (one server binary each, bench/bin/kokoro-server-VARIANT):
  today    main: one chunk per sentence, a long one cut at clauses grouped up to the limit
  min50    this PR's first version: chunks merged until 50 phonemes (`merge` below)
  gN       packed greedily to N phonemes (`pack::SOFT_MAX_PHONEMES` = N)

plan: each variant's chunks, from the server's `chunks` example (CHUNKS_EXAMPLE LIMIT SOFT, which
runs the server's own pipeline and packing) and, for min50, `merge` over today's chunks.
http: renders each passage through the server 4 times (the first kept as audio, OUT_DIR/SET-VARIANT-ID.npz
with `pcm`, `segments` and `ttfa`, the seconds from sending the request to its first audio bytes).
worker: every chunk's ids straight to the worker for its durations (OUT_DIR/chunk-HASH.npz), and
the decode time at batch 1 for ids of several lengths (OUT_DIR/speed.json).
asr: Whisper small.en per rendering, to OUT_DIR/asr.json.
"""

import hashlib
import json
import os
import subprocess
import sys
import time

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

SPACE_ID = 16
SETS = {"demo": {"limit": 70, "variants": ["today", "min50", "g30", "g60"]},
        "default": {"limit": 450, "variants": ["today", "min50", "g90", "g130", "g200"]}}
RUNS = 4


def merge(units, max_len, min_len=50):
    """The first version of this PR (merge.rs at b56cbdd), over today's chunks."""
    def join(a, b):
        return {"units": a["units"] + b["units"], "n": a["n"] + 1 + b["n"]}

    def fits(a, b):
        return a["n"] + 1 + b["n"] <= max_len

    out, held, current = [], None, None

    def finish(c):
        nonlocal held
        if held is not None and c["n"] < min_len and fits(held, c):
            held = join(held, c)
        else:
            if held is not None:
                out.append(held)
            held = c

    for u in units:
        n = {"units": [u], "n": len(u["phonemes"])}
        if current is not None and current["n"] < min_len and fits(current, n):
            current = join(current, n)
        else:
            if current is not None:
                finish(current)
            current = n
    if current is not None:
        finish(current)
    if held is not None:
        out.append(held)
    chunks = []
    for c in out:
        ids = []
        for u in c["units"]:
            ids += ([SPACE_ID] if ids else []) + u["ids"]
        chunks.append({"text": " ".join(u["text"] for u in c["units"]), "ids": ids, "units": c["units"]})
    return chunks


def run_chunks(binary, texts, limit, soft):
    res = subprocess.run([binary, str(limit), str(soft)], input="".join(json.dumps(t) + "\n" for t in texts),
                         capture_output=True, text=True, check=True)
    return [json.loads(line)["chunks"] for line in res.stdout.splitlines()]


def key(ids):
    return hashlib.sha1(bytes(ids)).hexdigest()[:12]


def plan(passages_path, binary, out):
    sets = json.load(open(passages_path, encoding="utf-8"))
    result = {}
    for name, passages in sets.items():
        limit = SETS[name]["limit"]
        texts = [p["text"] for p in passages]
        today = run_chunks(binary, texts, limit, 0)
        for p, chunks in zip(passages, today):
            p["variants"] = {"today": chunks, "min50": merge([u for c in chunks for u in c["units"]], limit)}
        for v in SETS[name]["variants"]:
            if v.startswith("g"):
                for p, chunks in zip(passages, run_chunks(binary, texts, limit, int(v[1:]))):
                    p["variants"][v] = chunks
        result[name] = {"limit": limit, "passages": passages}
        for p in passages:
            for v, chunks in p["variants"].items():
                print(f"{name:8} {p['id']:15} {v:6} " + " | ".join(f"{c['text']} ({len(c['ids'])})" for c in chunks))
    json.dump(result, open(out, "w"), ensure_ascii=False, indent=1)


def read_line(f) -> bytes:
    line = f.readline()
    if not line.endswith(b"\r\n"):
        raise ValueError(f"bad line {line!r}")
    return line[:-2]


def speak(url, api_key, text):
    """The server's sends, and the seconds until the first audio bytes."""
    import socket
    import urllib.parse

    u = urllib.parse.urlparse(url)
    body = json.dumps({"model": "kokoro", "input": text, "voice": "af_heart", "speed": 1.0,
                       "response_format": "pcm", "stream": True}).encode()
    head = (f"POST /v1/audio/speech HTTP/1.1\r\nHost: {u.netloc}\r\nAuthorization: Bearer {api_key}\r\n"
            f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n")
    with socket.create_connection((u.hostname, u.port), timeout=600) as s:
        start = time.perf_counter()
        s.sendall(head.encode() + body)
        f = s.makefile("rb")
        status = read_line(f)
        while read_line(f):
            pass
        if b" 200 " not in status + b" ":
            raise RuntimeError(f"{status!r}: {f.read()[:500]!r}")
        segments, first = [], None
        while True:
            n = int(read_line(f).split(b";")[0], 16)
            if n == 0:
                break
            segments.append(np.frombuffer(f.read(n), dtype="<i2"))
            first = first or time.perf_counter() - start
            assert read_line(f) == b""
    return segments, first


def http(url, api_key, plan_path, out, set_name, variant):
    os.makedirs(out, exist_ok=True)
    passages = json.load(open(plan_path, encoding="utf-8"))[set_name]["passages"]
    speak(url, api_key, "Warming up the server and the worker.")
    for p in passages:
        runs = [speak(url, api_key, p["text"]) for _ in range(RUNS)]
        segments = runs[0][0]
        chunks = p["variants"][variant]
        if len(segments) != len(chunks):
            print(f"WARNING {set_name} {variant} {p['id']}: {len(segments)} segments for {len(chunks)} chunks", flush=True)
        ttfa = [r[1] for r in runs]
        np.savez(f"{out}/{set_name}-{variant}-{p['id']}.npz", pcm=np.concatenate(segments),
                 segments=np.array([len(s) for s in segments]), ttfa=np.array(ttfa))
        print(f"{set_name} {variant} {p['id']}: {len(segments)} segments, first audio "
              + " ".join(f"{t * 1000:.0f}" for t in ttfa) + " ms", flush=True)


def worker(sock, plan_path, out):
    from kokoro_worker import protocol as P
    from load import wait_for

    client = wait_for(sock, 900)
    voice = client.hello.voices.index("af_heart")

    def render(ids):
        start = time.perf_counter()
        client.send(P.synthesize(0, ids, voice, 100))
        kind, _, payload = client.receive()
        assert kind == "result", (kind, payload)
        return payload, time.perf_counter() - start

    done = set()
    sets = json.load(open(plan_path, encoding="utf-8"))
    pool = []
    for s in sets.values():
        for p in s["passages"]:
            for chunks in p["variants"].values():
                for c in chunks:
                    pool += c["ids"]
                    for ids in [c["ids"]] + [u["ids"] for u in c["units"]]:
                        k = key(ids)
                        if k in done:
                            continue
                        done.add(k)
                        (durations, pcm), _ = render(ids)
                        np.savez(f"{out}/chunk-{k}.npz", pcm=np.frombuffer(pcm, dtype="<i2"),
                                 durations=np.array(durations))
    print(f"worker: {len(done)} chunks", flush=True)
    # Decode time at batch 1: real ids, cut to each length (the worker pads to its compiled shapes).
    speed = []
    for n in [10, 20, 30, 50, 70, 90, 130, 170, 200, 250, 300, 380, 450]:
        ids = (pool * (1 + n // max(1, len(pool))))[:n]
        render(ids)
        times = sorted(render(ids)[1] for _ in range(7))
        speed.append({"ids": n, "ms": 1000 * times[len(times) // 2], "min_ms": 1000 * times[0]})
        print(f"speed {n} ids: {speed[-1]['ms']:.1f} ms", flush=True)
    json.dump(speed, open(f"{out}/speed.json", "w"), indent=1)


def asr(plan_path, out):
    import re
    import torch
    from transformers import pipeline

    def words(text):
        return re.sub(r"[^a-z0-9' ]", " ", text.lower()).split()

    def wer(ref, hyp):
        d = list(range(len(hyp) + 1))
        for i in range(1, len(ref) + 1):
            prev, d[0] = d[0], i
            for j in range(1, len(hyp) + 1):
                cur = d[j]
                d[j] = min(d[j] + 1, d[j - 1] + 1, prev + (ref[i - 1] != hyp[j - 1]))
                prev = cur
        return d[-1]

    model = pipeline("automatic-speech-recognition", "openai/whisper-small.en", device=0, torch_dtype=torch.float16)
    results = {}
    for set_name, s in json.load(open(plan_path, encoding="utf-8")).items():
        for p in s["passages"]:
            # The normalized text, so "12" is "twelve" in the reference too.
            ref = words(" ".join(c["text"] for c in p["variants"]["today"]))
            for name in p["variants"]:
                k = f"{set_name}-{name}-{p['id']}"
                x = np.load(f"{out}/{k}.npz")["pcm"].astype(np.float32) / 32767
                at_16k = np.interp(np.arange(0, len(x), 1.5), np.arange(len(x)), x).astype(np.float32)
                text = model({"raw": at_16k, "sampling_rate": 16000}, return_timestamps=True)["text"]
                results[k] = {"hyp": text, "errors": wer(ref, words(text)), "words": len(ref)}
                print(k, results[k]["errors"], text, flush=True)
    json.dump(results, open(f"{out}/asr.json", "w"), ensure_ascii=False, indent=1)


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "plan":
        plan(*sys.argv[2:5])
    elif mode == "http":
        http(*sys.argv[2:8])
    elif mode == "worker":
        worker(*sys.argv[2:5])
    elif mode == "asr":
        asr(*sys.argv[2:4])
    else:
        raise SystemExit(__doc__)
