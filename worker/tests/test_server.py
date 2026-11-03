"""The connection logic against a fake model: violations close the connection, nothing hangs."""

import socket
import struct
import threading

import pytest

from kokoro_worker import protocol as P
from kokoro_worker.client import Client
from kokoro_worker.model import Result
from kokoro_worker.server import Connection


class FakeSynth:
    vocab_size = 178
    vocab_ids = frozenset([1, 2, 3, 16])
    max_phonemes = 510
    voice_names = ["af_heart", "am_adam"]

    def __init__(self):
        self.gate = threading.Event()
        self.gate.set()
        self.batches = []

    def style(self, voices, n_ids):
        return None

    def run(self, requests):
        self.batches.append(len(requests))
        self.gate.wait(10)
        results = []
        for i, r in enumerate(requests):
            if r.ids[0] == 3:
                results.append(Result(i, None, None, "too_long"))
            else:
                results.append(Result(i, b"\x01\x00" * len(r.ids), [1] * (len(r.ids) + 2)))
        yield results


@pytest.fixture
def worker():
    synth = FakeSynth()
    a, b = socket.socketpair()
    conn = Connection(a, synth, max_batch=4)
    thread = threading.Thread(target=conn.serve, daemon=True)
    thread.start()
    client = Client.__new__(Client)
    client.sock = b
    b.settimeout(10)
    kind, body = client.read()
    assert kind == P.HELLO
    client.hello = Client._parse_hello(body)
    yield client, synth, thread
    b.close()
    thread.join(10)
    assert not thread.is_alive()


def closed(client) -> bool:
    try:
        while True:
            if client.sock.recv(65536) == b"":
                return True
    except ConnectionResetError:
        return True


def test_hello(worker):
    client, _, _ = worker
    assert client.hello.voices == ["af_heart", "am_adam"]
    assert client.hello.max_batch == 4


def test_synthesize(worker):
    client, _, _ = worker
    client.send(P.synthesize(9, [1, 2, 16], [(1, 1.0)]))
    assert client.receive() == ("durations", 9, [1, 1, 1, 1, 1])
    assert client.receive() == ("audio", 9, b"\x01\x00" * 3)


def test_too_long(worker):
    client, _, _ = worker
    client.send(P.synthesize(4, [3, 1], [(0, 1.0)]))
    assert client.receive() == ("failed", 4, P.TOO_LONG)


def test_cancel_queued(worker):
    client, synth, _ = worker
    synth.gate.clear()
    client.send(P.synthesize(1, [1], [(0, 1.0)]))
    while not synth.batches:
        pass
    client.send(P.synthesize(2, [1], [(0, 1.0)]))
    client.send(P.cancel(2))
    assert client.receive() == ("failed", 2, P.CANCELLED)
    client.send(P.cancel(1))  # running: finishes normally
    client.send(P.cancel(12345))  # unknown: ignored
    synth.gate.set()
    assert client.receive()[0:2] == ("durations", 1)
    assert client.receive()[0:2] == ("audio", 1)


def test_batches_queued_requests(worker):
    client, synth, _ = worker
    synth.gate.clear()
    client.send(P.synthesize(1, [1], [(0, 1.0)]))
    while not synth.batches:
        pass
    for rid in range(2, 8):
        client.send(P.synthesize(rid, [1], [(0, 1.0)], priority=rid % 2))
    synth.gate.set()
    seen = set()
    while len(seen) < 7:
        kind, rid, _ = client.receive()
        if kind == "audio":
            seen.add(rid)
    assert synth.batches[0] == 1 and max(synth.batches) <= 4


@pytest.mark.parametrize(
    "data",
    [
        struct.pack("<I", 0),
        struct.pack("<I", P.MAX_FRONT_BODY + 1),
        struct.pack("<I", 0xFFFFFFFF),
        struct.pack("<I", 1) + b"\x99",
        P.synthesize(1, [5], [(0, 1.0)]),  # id not in vocab
        P.synthesize(1, [1], [(2, 1.0)]),  # voice out of range
        struct.pack("<I", 3) + b"\x11\x00\x00",
    ],
)
def test_violation_closes(worker, data):
    client, _, _ = worker
    client.send(data)
    assert closed(client)


def test_duplicate_id_closes(worker):
    client, synth, _ = worker
    synth.gate.clear()
    client.send(P.synthesize(1, [1], [(0, 1.0)]))
    while not synth.batches:  # still running, so still outstanding
        pass
    client.send(P.synthesize(1, [1], [(0, 1.0)]))
    assert closed(client)
    synth.gate.set()


def test_truncated_frame_then_eof(worker):
    client, _, thread = worker
    client.send(P.synthesize(1, [1, 2], [(0, 1.0)])[:-1])
    client.sock.shutdown(socket.SHUT_WR)
    thread.join(10)
    assert not thread.is_alive()
