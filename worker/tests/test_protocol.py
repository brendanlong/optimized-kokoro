import math
import random
import struct

import pytest

from kokoro_worker import protocol as P

VOCAB = 178  # vocab_size: ids 1..177


def parse(frame: bytes):
    (length,) = struct.unpack_from("<I", frame)
    assert length == len(frame) - 4
    return P.parse_request(frame[4:], n_voices=3, vocab_size=VOCAB, max_phonemes=510)


def test_synthesize_round_trip():
    msg = parse(P.synthesize(7, [1, 16, 177], [(0, 1.0), (2, 0.5)], speed=1.25, priority=1, style_len=300))
    assert msg == P.Synthesize(7, 1, 1.25, ((0, 1.0), (2, 0.5)), 300, (1, 16, 177))
    assert parse(P.synthesize(7, [1, 16], [(0, 1.0)])).style_len == 2


def test_cancel_round_trip():
    assert parse(P.cancel(2**64 - 1)) == P.Cancel(2**64 - 1)


@pytest.mark.parametrize(
    "kwargs",
    [
        dict(priority=2),
        dict(speed=math.nan),
        dict(speed=math.inf),
        dict(speed=0.24),
        dict(speed=4.01),
        dict(voices=[]),
        dict(voices=[(0, 1.0)] * 9),
        dict(voices=[(3, 1.0)]),
        dict(voices=[(0, 0.0)]),
        dict(voices=[(0, -1.0)]),
        dict(voices=[(0, math.nan)]),
        dict(voices=[(0, math.inf)]),
        dict(ids=[]),
        dict(ids=[1] * 511),
        dict(ids=[0]),
        dict(ids=[178]),
        dict(style_len=0),
        dict(style_len=511),
    ],
)
def test_synthesize_rejects(kwargs):
    args = dict(request_id=1, ids=[1, 2], voices=[(0, 1.0)], speed=1.0, priority=0)
    args.update(kwargs)
    with pytest.raises(P.ProtocolError):
        parse(P.synthesize(**args))


def test_max_lengths_accepted():
    parse(P.synthesize(1, [1] * 510, [(0, 1.0)] * 8, speed=0.25))
    parse(P.synthesize(1, [1], [(0, 1.0)], speed=4.0, style_len=510))


@pytest.mark.parametrize("delta", [-1, 1])
def test_wrong_length_rejected(delta):
    body = P.synthesize(1, [1, 2, 3], [(0, 1.0)])[4:]
    body = body[:delta] if delta < 0 else body + b"\x01"
    with pytest.raises(P.ProtocolError):
        P.parse_request(body, 3, VOCAB, 510)


@pytest.mark.parametrize("body", [b"", b"\x00", b"\x12", b"\x11" + b"\x00" * 7, b"\x11" + b"\x00" * 9])
def test_bad_bodies_rejected(body):
    with pytest.raises(P.ProtocolError):
        P.parse_request(body, 3, VOCAB, 510)


def test_fuzz_only_protocol_errors():
    rng = random.Random(0)
    good = P.synthesize(5, [1, 2, 16], [(1, 0.5), (2, 2.0)])[4:]
    for _ in range(20000):
        body = bytearray(good)
        for _ in range(rng.randint(1, 4)):
            op = rng.randrange(3)
            if op == 0 and body:
                body[rng.randrange(len(body))] = rng.randrange(256)
            elif op == 1:
                body.insert(rng.randrange(len(body) + 1), rng.randrange(256))
            elif body:
                del body[rng.randrange(len(body))]
        try:
            P.parse_request(bytes(body), 3, VOCAB, 510)
        except P.ProtocolError:
            pass
    for _ in range(5000):
        body = bytes(rng.randrange(256) for _ in range(rng.randrange(40)))
        try:
            P.parse_request(body, 3, VOCAB, 510)
        except P.ProtocolError:
            pass


def test_extreme_valid_weights_accepted():
    biggest = struct.unpack("<f", struct.pack("<I", 0x7F7FFFFF))[0]
    smallest = struct.unpack("<f", struct.pack("<I", 0x00000001))[0]
    msg = parse(P.synthesize(1, [1], [(i % 3, biggest) for i in range(8)]))
    assert all(w == biggest for _, w in msg.voices)
    msg = parse(P.synthesize(1, [1], [(0, biggest), (1, smallest)]))
    assert msg.voices[1][1] == smallest
