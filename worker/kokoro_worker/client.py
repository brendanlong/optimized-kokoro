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

MAX_WORKER_BODY = P.MAX_WORKER_BODY


class WorkerError(Exception):
    pass


@dataclass
class Hello:
    version: int
    vocab_size: int
    max_phonemes: int
    max_batch: int
    sample_rate: int
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
        if not 1 <= length <= MAX_WORKER_BODY:
            raise WorkerError(f"frame length {length}")
        body = _recv_exact(self.sock, length)
        return body[0], body[1:]

    @staticmethod
    def _parse_hello(b: bytes) -> Hello:
        if len(b) < 18:
            raise WorkerError("short Hello")
        magic, version, vocab, max_ph, max_batch, rate, n = struct.unpack_from("<4sHHHHIH", b)
        if magic != P.MAGIC or version != P.VERSION:
            raise WorkerError("bad magic or version")
        pos, voices = 18, []
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
        return Hello(version, vocab, max_ph, max_batch, rate, voices)

    def send(self, data: bytes) -> None:
        self.sock.sendall(data)

    def receive(self):
        """Next response: ("audio", id, pcm bytes) | ("durations", id, [int]) | ("failed", id, code)."""
        kind, b = self.read()
        if kind == P.AUDIO and len(b) >= 12:
            rid, n = struct.unpack_from("<QI", b)
            if len(b) != 12 + 2 * n:
                raise WorkerError("Audio length")
            return "audio", rid, b[12:]
        if kind == P.DURATIONS and len(b) >= 10:
            rid, n = struct.unpack_from("<QH", b)
            if len(b) != 10 + 2 * n:
                raise WorkerError("Durations length")
            return "durations", rid, list(struct.unpack_from(f"<{n}H", b, 10))
        if kind == P.FAILED and len(b) == 9:
            rid, code = struct.unpack("<QB", b)
            return "failed", rid, code
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
    ap.add_argument("--voice", default="af_heart", help="a name, or a mix like af_heart:2,am_adam:1")
    ap.add_argument("--speed", type=float, default=1.0)
    ap.add_argument("--out", default="out.wav")
    args = ap.parse_args()

    vocab = json.load(open(args.config, encoding="utf-8"))["vocab"]
    ids = [vocab[c] for c in args.phonemes if c in vocab]
    client = Client(args.socket)
    print(f"worker: {client.hello.max_batch=} {len(client.hello.voices)} voices")
    mix = []
    for part in args.voice.split(","):
        name, _, weight = part.partition(":")
        mix.append((client.hello.voices.index(name), float(weight or 1)))
    client.send(P.synthesize(1, ids, mix, args.speed))
    while True:
        kind, rid, payload = client.receive()
        if kind == "durations":
            print(f"durations: {payload}")
        elif kind == "audio":
            write_wav(args.out, payload, client.hello.sample_rate)
            print(f"wrote {args.out}: {len(payload) // 2 / client.hello.sample_rate:.2f} s")
            break
        else:
            raise SystemExit(f"request failed, code {payload}")
    client.close()


if __name__ == "__main__":
    main()
