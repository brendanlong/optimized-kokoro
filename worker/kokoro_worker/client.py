"""A tiny front for testing the worker: phonemes in, WAV out.

    python -m kokoro_worker.client SOCKET --config models/config.json --voice af_heart \\
        --phonemes "həlˈO wˈɜɹld." --out hello.wav

The client checks the worker's frames as the real front must: lengths against caps before
allocating, known types, exact body sizes. No torch needed.
"""

import argparse
import json
import socket
import struct
import wave
from dataclasses import dataclass

from . import protocol as P

HELLO_MAX = 25 + 65535 * 65


class WorkerError(Exception):
    pass


@dataclass
class Hello:
    version: int
    vocab_size: int
    max_phonemes: int
    max_batch: int
    slots: int
    sample_rate: int
    max_chunk_samples: int
    voices: list[str]


def _recv_exact(sock: socket.socket, n: int) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise EOFError("worker closed the connection")
        buf += chunk
    return bytes(buf)


class Client:
    def __init__(self, path: str):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(path)
        kind, body = self.read()
        if kind != P.HELLO:
            raise WorkerError("first frame is not Hello")
        self.hello = self._parse_hello(body)

    def close(self) -> None:
        self.sock.close()

    def read(self) -> tuple[int, bytes]:
        (length,) = struct.unpack("<I", _recv_exact(self.sock, 4))
        # Before Hello, the bound is Hello's; after it, a Result with the most ids and the cap's samples.
        hello = getattr(self, "hello", None)
        bound = HELLO_MAX if hello is None else (
            5 + 2 * (hello.max_phonemes + 2) + 2 * hello.max_chunk_samples)
        if not 1 <= length <= bound:
            raise WorkerError(f"frame length {length}")
        body = _recv_exact(self.sock, length)
        return body[0], body[1:]

    @staticmethod
    def _parse_hello(b: bytes) -> Hello:
        if len(b) < 24:
            raise WorkerError("short Hello")
        magic, version, vocab, max_ph, max_batch, slots, rate, cap, n = struct.unpack_from("<4sHHHHHIIH", b)
        if magic != P.MAGIC or version != P.VERSION:
            raise WorkerError("bad magic or version")
        pos, voices = 24, []
        for _ in range(n):
            if pos >= len(b):
                raise WorkerError("short Hello")
            k = b[pos]
            name = b[pos + 1 : pos + 1 + k]
            if not 1 <= k <= 64 or len(name) != k:
                raise WorkerError("bad voice name")
            voices.append(name.decode("ascii"))
            pos += 1 + k
        if pos != len(b):
            raise WorkerError("long Hello")
        return Hello(version, vocab, max_ph, max_batch, slots, rate, cap, voices)

    def send(self, data: bytes) -> None:
        self.sock.sendall(data)

    def receive(self):
        """Next response: ("result", slot, ([durations], pcm bytes)) | ("failed", slot, code).

        Checks the frame's structure; the caller checks it against its request."""
        kind, b = self.read()
        if kind == P.RESULT and len(b) >= 4:
            slot, n = struct.unpack_from("<HH", b)
            if len(b) < 4 + 2 * n:
                raise WorkerError("Result length")
            durations = list(struct.unpack_from(f"<{n}H", b, 4))
            pcm = b[4 + 2 * n:]
            if len(pcm) != 2 * P.SAMPLES_PER_FRAME * sum(durations):
                raise WorkerError("Result: audio length doesn't match the durations")
            return "result", slot, (durations, pcm)
        if kind == P.FAILED and len(b) == 3:
            slot, code = struct.unpack("<HB", b)
            return "failed", slot, code
        raise WorkerError(f"bad frame type {kind:#x} or length")


def write_wav(path: str, pcm: bytes, rate: int = P.SAMPLE_RATE) -> None:
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("socket")
    ap.add_argument("--config", required=True, help="the model's config.json, for the phoneme vocab")
    ap.add_argument("--phonemes", required=True)
    ap.add_argument("--voice", default="af_heart")
    ap.add_argument("--speed", type=float, default=1.0, help="0.25 to 4, sent in hundredths")
    ap.add_argument("--out", default="out.wav")
    args = ap.parse_args()

    vocab = json.load(open(args.config, encoding="utf-8"))["vocab"]
    ids = [vocab[c] for c in args.phonemes if c in vocab]
    client = Client(args.socket)
    print(f"worker: {client.hello.max_batch=} {len(client.hello.voices)} voices")
    speed = round(args.speed * 100)
    client.send(P.synthesize(0, ids, client.hello.voices.index(args.voice), speed))
    kind, slot, payload = client.receive()
    if slot != 0:
        raise WorkerError(f"a reply for slot {slot}, which is free")
    if kind == "failed":
        raise SystemExit(f"request failed, code {payload}")
    durations, pcm = payload
    if len(durations) != len(ids) + 2 or not all(1 <= d <= P.max_duration(speed) for d in durations):
        raise WorkerError("bad durations")
    if len(pcm) // 2 > client.hello.max_chunk_samples:
        raise WorkerError("audio over the chunk cap")
    print(f"durations: {durations}")
    write_wav(args.out, pcm, client.hello.sample_rate)
    print(f"wrote {args.out}: {len(pcm) // 2 / client.hello.sample_rate:.2f} s")
    client.close()



if __name__ == "__main__":
    main()
