"""Listening test for merging short sentences into one chunk (crates/server/src/merge.rs).

    python bench/merge_short.py plan PASSAGES PHONEMIZE OUT_PLAN      (locally, no GPU)
    python bench/merge_short.py http URL API_KEY PLAN OUT_DIR MIN     (in the job, per server binary)
    python bench/merge_short.py worker SOCKET PLAN OUT_DIR            (in the job)
    python bench/merge_short.py asr PLAN OUT_DIR                      (in the job)

plan: each passage's chunks per variant, from the pipeline's phonemize example (PHONEMIZE, the
chunks at 450 phonemes) and `merge` below, a copy of merge.rs's rule. Variants:
  today   one chunk per sentence (main)
  mN      merged to at least N phonemes (the server built with MIN_SENTENCE_CHUNK = N)
  pN      the same, but never across a paragraph break: each paragraph is its own request (with the
          period join_lines adds), and the audio is concatenated
http: renders every variant a server binary built with MIN serves (MIN 0 is today), keeping each of
the server's sends as a segment: OUT_DIR/VARIANT-ID.npz has `pcm` and `segments`.
worker: every chunk's ids straight to the worker, for its durations: OUT_DIR/chunk-HASH.npz.
asr: WER per variant with Whisper small.en (as bench/asr_check.py), to OUT_DIR/asr.json.
"""

import hashlib
import json
import os
import subprocess
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

MINS = [25, 50, 80]
LIMIT = 450
SPACE_ID = 16


def merge(chunks, max_len=LIMIT, min_len=50):
    """merge.rs `merge_short` on one request's speech chunks (no pauses in these passages). Each
    chunk is {"texts": [...], "phonemes": str, "ids": [...]}."""
    def join(a, b):
        return {"texts": a["texts"] + b["texts"], "phonemes": a["phonemes"] + " " + b["phonemes"],
                "ids": a["ids"] + [SPACE_ID] + b["ids"]}

    def fits(a, b):
        return len(a["phonemes"]) + 1 + len(b["phonemes"]) <= max_len

    out, held, current = [], None, None

    def finish(c):
        nonlocal held
        if held is not None and len(c["phonemes"]) < min_len and fits(held, c):
            held = join(held, c)
        else:
            if held is not None:
                out.append(held)
            held = c

    for n in chunks:
        if current is not None and len(current["phonemes"]) < min_len and fits(current, n):
            current = join(current, n)
        else:
            if current is not None:
                finish(current)
            current = n
    if current is not None:
        finish(current)
    if held is not None:
        out.append(held)
    return out


def paragraphs(text):
    """join_lines' paragraphs, each ending in punctuation as join_lines leaves it inside the text."""
    paras = [" ".join(p.split()) for p in text.split("\n\n") if p.strip()]
    return [p if i == len(paras) - 1 or p.rstrip("\"')]”’")[-1:] in ".!?:…" else p + "."
            for i, p in enumerate(paras)]


def phonemize(binary, texts):
    res = subprocess.run([binary, "a", str(LIMIT)], input="".join(json.dumps(t) + "\n" for t in texts),
                         capture_output=True, text=True, check=True)
    return [[{"texts": [c["text"]], "phonemes": c["phonemes"], "ids": c["ids"]} for c in json.loads(line)["chunks"]]
            for line in res.stdout.splitlines()]


def key(ids):
    return hashlib.sha1(bytes(ids)).hexdigest()[:12]


def plan(passages_path, binary, out):
    passages = json.load(open(passages_path, encoding="utf-8"))
    for p in passages:
        whole = phonemize(binary, [p["text"]])[0]
        paras = paragraphs(p["text"])
        per_para = phonemize(binary, paras)
        assert [c["texts"] for c in whole] == [c["texts"] for para in per_para for c in para], p["id"]
        variants = {"today": {"requests": [p["text"]], "chunks": [whole]}}
        for m in MINS:
            variants[f"m{m}"] = {"requests": [p["text"]], "chunks": [merge(whole, min_len=m)]}
            if len(paras) > 1:
                variants[f"p{m}"] = {"requests": paras, "chunks": [merge(c, min_len=m) for c in per_para]}
        p["sentences"] = [c["texts"][0] for c in whole]
        p["variants"] = variants
    json.dump(passages, open(out, "w"), ensure_ascii=False, indent=1)
    for p in passages:
        for v, d in p["variants"].items():
            print(f"{p['id']:12} {v:6} " + " | ".join(" ".join(c["texts"]) for r in d["chunks"] for c in r))


def read_line(f) -> bytes:
    line = f.readline()
    if not line.endswith(b"\r\n"):
        raise ValueError(f"bad line {line!r}")
    return line[:-2]


def speak(url, api_key, text):
    import socket
    import urllib.parse

    u = urllib.parse.urlparse(url)
    body = json.dumps({"model": "kokoro", "input": text, "voice": "af_heart", "speed": 1.0,
                       "response_format": "pcm", "stream": True}).encode()
    head = (f"POST /v1/audio/speech HTTP/1.1\r\nHost: {u.netloc}\r\nAuthorization: Bearer {api_key}\r\n"
            f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n")
    with socket.create_connection((u.hostname, u.port), timeout=600) as s:
        s.sendall(head.encode() + body)
        f = s.makefile("rb")
        status = read_line(f)
        while read_line(f):
            pass
        if b" 200 " not in status + b" ":
            raise RuntimeError(f"{status!r}: {f.read()[:500]!r}")
        segments = []
        while True:
            n = int(read_line(f).split(b";")[0], 16)
            if n == 0:
                break
            segments.append(np.frombuffer(f.read(n), dtype="<i2"))
            assert read_line(f) == b""
    return segments


def http(url, api_key, plan_path, out, min_len):
    os.makedirs(out, exist_ok=True)
    names = ["today"] if min_len == 0 else [f"m{min_len}", f"p{min_len}"]
    for p in json.load(open(plan_path, encoding="utf-8")):
        for name in names:
            v = p["variants"].get(name)
            if v is None:
                continue
            segments = []
            for text, chunks in zip(v["requests"], v["chunks"]):
                got = speak(url, api_key, text)
                if len(got) != len(chunks):
                    print(f"WARNING {name} {p['id']}: {len(got)} segments for {len(chunks)} chunks", flush=True)
                segments += got
            np.savez(f"{out}/{name}-{p['id']}.npz", pcm=np.concatenate(segments),
                     segments=np.array([len(s) for s in segments]))
            print(f"{name} {p['id']}: {len(segments)} segments", flush=True)


def worker(sock, plan_path, out):
    from kokoro_worker import protocol as P
    from load import wait_for

    client = wait_for(sock, 900)
    voice = client.hello.voices.index("af_heart")
    done = set()
    for p in json.load(open(plan_path, encoding="utf-8")):
        for v in p["variants"].values():
            for chunks in v["chunks"]:
                for c in chunks:
                    k = key(c["ids"])
                    if k in done:
                        continue
                    done.add(k)
                    client.send(P.synthesize(0, c["ids"], voice, 100))
                    kind, _, payload = client.receive()
                    assert kind == "result", (kind, payload)
                    durations, pcm = payload
                    np.savez(f"{out}/chunk-{k}.npz", pcm=np.frombuffer(pcm, dtype="<i2"), durations=np.array(durations))
    print(f"worker: {len(done)} chunks", flush=True)


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
    for p in json.load(open(plan_path, encoding="utf-8")):
        # The normalized sentences, so "12" is "twelve" in the reference too.
        ref = words(" ".join(p["sentences"]))
        for name in p["variants"]:
            x = np.load(f"{out}/{name}-{p['id']}.npz")["pcm"].astype(np.float32) / 32767
            at_16k = np.interp(np.arange(0, len(x), 1.5), np.arange(len(x)), x).astype(np.float32)
            text = model({"raw": at_16k, "sampling_rate": 16000}, return_timestamps=True)["text"]
            results[f"{name}-{p['id']}"] = {"hyp": text, "errors": wer(ref, words(text)), "words": len(ref)}
            print(name, p["id"], results[f"{name}-{p['id']}"]["errors"], text, flush=True)
    json.dump(results, open(f"{out}/asr.json", "w"), ensure_ascii=False, indent=1)


if __name__ == "__main__":
    mode = sys.argv[1]
    if mode == "plan":
        plan(*sys.argv[2:5])
    elif mode == "http":
        http(*sys.argv[2:6], int(sys.argv[6]))
    elif mode == "worker":
        worker(*sys.argv[2:5])
    elif mode == "asr":
        asr(*sys.argv[2:4])
    else:
        raise SystemExit(__doc__)
