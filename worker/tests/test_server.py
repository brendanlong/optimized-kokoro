"""The connection logic against a fake model: violations close the connection, nothing hangs."""

import socket
import time
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
    max_chunk_samples = 2_880_000

    def __init__(self):
        self.gate = threading.Event()
        self.gate.set()
        self.batches = []

    def style(self, voice, style_len):
        return None

    def run(self, requests):
        self.batches.append(len(requests))
        self.gate.wait(10)
        results = []
        for i, r in enumerate(requests):
            if r.ids[0] == 3:
                results.append(Result(i, None, None, "too_long"))
            else:
                results.append(Result(i, PCM * (len(r.ids) + 2), [1] * (len(r.ids) + 2)))
        yield results


PCM = b"\x01\x00" * 600  # one duration unit of the fake model's audio
SLOTS = 8


@pytest.fixture
def worker():
    synth = FakeSynth()
    a, b = socket.socketpair()
    conn = Connection(a, synth, max_batch=4, slots=SLOTS)
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
    assert client.hello.slots == SLOTS
    assert client.hello.max_chunk_samples == 2_880_000


def test_synthesize(worker):
    client, _, _ = worker
    client.send(P.synthesize(SLOTS - 1, [1, 2, 16], 1))
    assert client.receive() == ("result", SLOTS - 1, ([1, 1, 1, 1, 1], PCM * 5))


def test_too_long(worker):
    client, _, _ = worker
    client.send(P.synthesize(4, [3, 1], 0))
    assert client.receive() == ("failed", 4, P.TOO_LONG)


def test_a_slot_is_free_again_once_answered(worker):
    client, synth, _ = worker
    for _ in range(3):
        client.send(P.synthesize(0, [1], 0))
        assert client.receive()[:2] == ("result", 0)
    synth.gate.clear()
    client.send(P.synthesize(1, [1], 0))
    while not synth.batches[3:]:
        pass
    client.send(P.synthesize(0, [1], 0))
    client.send(P.cancel(0))
    assert client.receive() == ("failed", 0, P.CANCELLED)
    client.send(P.synthesize(0, [2], 0))
    synth.gate.set()
    assert {client.receive()[:2] for _ in range(2)} == {("result", 0), ("result", 1)}


def test_cancel_queued(worker):
    client, synth, _ = worker
    synth.gate.clear()
    client.send(P.synthesize(1, [1], 0))
    while not synth.batches:
        pass
    client.send(P.synthesize(2, [1], 0))
    client.send(P.cancel(2))
    assert client.receive() == ("failed", 2, P.CANCELLED)
    client.send(P.cancel(1))  # running: finishes normally
    client.send(P.cancel(5))  # free: ignored, its reply crossed the cancel
    synth.gate.set()
    assert client.receive()[0:2] == ("result", 1)


def test_batches_queued_requests(worker):
    client, synth, _ = worker
    synth.gate.clear()
    client.send(P.synthesize(0, [1], 0))
    while not synth.batches:
        pass
    for slot in range(1, SLOTS):
        client.send(P.synthesize(slot, [1], 0, priority=slot % 2))
    synth.gate.set()
    seen = set()
    while len(seen) < SLOTS:
        kind, slot, _ = client.receive()
        assert kind == "result"
        seen.add(slot)
    assert synth.batches[0] == 1 and max(synth.batches) <= 4


@pytest.mark.parametrize(
    "data",
    [
        struct.pack("<I", 0),
        struct.pack("<I", P.max_front_body(510) + 1) + bytes(523),
        struct.pack("<I", 0xFFFFFFFF),
        struct.pack("<I", 1) + b"\x99",
        P.synthesize(1, [200], 0),  # id out of range
        P.synthesize(1, [0], 0),  # the boundary id
        P.synthesize(1, [1], 2),  # voice out of range
        P.synthesize(SLOTS, [1], 0),  # slot out of range
        P.synthesize(1, [1], 0, speed=401),
        P.cancel(SLOTS),
        struct.pack("<I", 2) + b"\x11\x00",
    ],
)
def test_violation_closes(worker, data):
    client, _, _ = worker
    client.send(data)
    assert closed(client)


@pytest.mark.parametrize("running", [True, False])
def test_synthesize_for_a_busy_slot_closes(worker, running):
    client, synth, _ = worker
    synth.gate.clear()
    if not running:
        client.send(P.synthesize(0, [1], 0))
        while not synth.batches:
            pass
    client.send(P.synthesize(1, [1], 0))
    if running:
        while not synth.batches:
            pass
    client.send(P.synthesize(1, [1], 0))
    assert closed(client)
    synth.gate.set()


def test_truncated_frame_then_eof(worker):
    client, _, thread = worker
    client.send(P.synthesize(1, [1, 2], 0)[:-1])

    client.sock.shutdown(socket.SHUT_WR)
    thread.join(10)
    assert not thread.is_alive()


def test_stall_mid_frame_closes():
    synth = FakeSynth()
    a, b = socket.socketpair()
    conn = Connection(a, synth, max_batch=4, frame_timeout=0.5)
    thread = threading.Thread(target=conn.serve, daemon=True)
    thread.start()
    b.settimeout(10)
    b.recv(65536)  # Hello
    b.sendall(P.synthesize(1, [1, 2], 0)[:7])  # part of a frame, then nothing
    t = time.monotonic()
    assert closed(_Raw(b))
    assert time.monotonic() - t < 5
    thread.join(5)
    assert not thread.is_alive()


def test_idle_front_is_not_a_stall():
    synth = FakeSynth()
    a, b = socket.socketpair()
    conn = Connection(a, synth, max_batch=4, frame_timeout=0.3)
    threading.Thread(target=conn.serve, daemon=True).start()
    b.settimeout(10)
    b.recv(65536)
    time.sleep(1.0)  # longer than the frame timeout, between frames
    b.sendall(P.synthesize(5, [1], 0))
    data = b.recv(65536)
    assert data[4] == P.RESULT
    b.close()


def test_clients_that_close_early_cannot_kill_the_accept_loop(tmp_path):
    from kokoro_worker.server import accept_loop

    path = str(tmp_path / "w.sock")
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(path)
    listener.listen(1)
    thread = threading.Thread(target=accept_loop, args=(listener, FakeSynth(), 4, 1.0), daemon=True)
    thread.start()
    for _ in range(30):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(path)
        s.close()  # before reading Hello
    for _ in range(5):  # and some that leave mid-frame
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(path)
        s.sendall(b"\x05\x00")
        s.close()
    client = Client(path)
    client.sock.settimeout(10)
    client.send(P.synthesize(1, [1, 2], 0))
    assert client.receive()[0] == "result"
    client.close()
    assert thread.is_alive()


class _Raw:
    def __init__(self, sock):
        self.sock = sock
