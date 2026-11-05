"""Renders the clause-split listening sentences (bench/clause/sentences.json) for the listening page.

    python bench/clause_joins.py http URL API_KEY SENTENCES OUT_DIR LABEL
    python bench/clause_joins.py worker SOCKET SENTENCES OUT_DIR

http: each sentence through the HTTP server, voice af_heart at its speed. The body is read as raw
chunked transfer encoding, so each of the server's sends (one per chunk, or per stitched piece)
stays a separate segment: OUT_DIR/LABEL-N.npz holds `pcm` and `segments` (their lengths).
worker: each sentence's whole ids in one request straight to the worker (no trimming), as a
reference for how Kokoro phrases the whole sentence: OUT_DIR/whole-N.npz holds `pcm` and
`durations`.
"""

import json
import os
import socket
import sys
import urllib.parse

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def read_line(f) -> bytes:
    line = f.readline()
    if not line.endswith(b"\r\n"):
        raise ValueError(f"bad line {line!r}")
    return line[:-2]


def speak(url: str, key: str, text: str, speed: float):
    u = urllib.parse.urlparse(url)
    body = json.dumps({"model": "kokoro", "input": text, "voice": "af_heart", "speed": speed,
                       "response_format": "pcm", "stream": True}).encode()
    head = (f"POST /v1/audio/speech HTTP/1.1\r\nHost: {u.netloc}\r\nAuthorization: Bearer {key}\r\n"
            f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\nConnection: close\r\n\r\n")
    with socket.create_connection((u.hostname, u.port), timeout=600) as s:
        s.sendall(head.encode() + body)
        f = s.makefile("rb")
        status = read_line(f)
        headers = {}
        while line := read_line(f):
            k, v = line.decode().split(":", 1)
            headers[k.strip().lower()] = v.strip()
        if b" 200 " not in status + b" ":
            raise RuntimeError(f"{status!r}: {f.read()[:500]!r}")
        assert headers.get("transfer-encoding") == "chunked", headers
        segments = []
        while True:
            n = int(read_line(f).split(b";")[0], 16)
            if n == 0:
                break
            segments.append(np.frombuffer(f.read(n), dtype="<i2"))
            assert read_line(f) == b""
    return segments


def main():
    mode = sys.argv[1]
    if mode == "http":
        url, key, path, out, label = sys.argv[2:7]
        os.makedirs(out, exist_ok=True)
        for i, row in enumerate(json.load(open(path, encoding="utf-8"))):
            segments = speak(url, key, row["text"], row["speed"])
            pcm = np.concatenate(segments)
            np.savez(f"{out}/{label}-{i}.npz", pcm=pcm, segments=np.array([len(s) for s in segments]))
            print(f"{label} {i}: {len(segments)} segments, {len(pcm) / 24000:.2f} s", flush=True)
    elif mode == "worker":
        from kokoro_worker import protocol as P
        from load import wait_for

        sock, path, out = sys.argv[2:5]
        os.makedirs(out, exist_ok=True)
        client = wait_for(sock, 900)
        voice = client.hello.voices.index("af_heart")
        for i, row in enumerate(json.load(open(path, encoding="utf-8"))):
            client.send(P.synthesize(0, row["ids"], voice, round(row["speed"] * 100)))
            kind, _, payload = client.receive()
            assert kind == "result", (kind, payload)
            durations, pcm = payload
            np.savez(f"{out}/whole-{i}.npz", pcm=np.frombuffer(pcm, dtype="<i2"), durations=np.array(durations))
            print(f"whole {i}: {len(pcm) / 48000:.2f} s", flush=True)
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main()
