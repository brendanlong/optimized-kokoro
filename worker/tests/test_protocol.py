import random
import struct

import pytest

from kokoro_worker import protocol as P

VOCAB = 178  # vocab_size: ids 1..177
SLOTS = 4


def parse(frame: bytes):
    (length,) = struct.unpack_from("<I", frame)
    assert length == len(frame) - 4
    return P.parse_request(frame[4:], slots=SLOTS, n_voices=3, vocab_size=VOCAB, max_phonemes=510)


def test_synthesize_round_trip():
    msg = parse(P.synthesize(3, [1, 16, 177], 2, speed=125, priority=255, waiting=0, style_len=300))
    assert msg == P.Synthesize(3, 255, 0, 125, 2, 300, (1, 16, 177))
    assert parse(P.synthesize(0, [1], 0, priority=0)).priority == 0
    assert parse(P.synthesize(0, [1, 16], 0)).style_len == 2


def test_cancel_round_trip():
    assert parse(P.cancel(SLOTS - 1)) == P.Cancel(SLOTS - 1)


@pytest.mark.parametrize(
    "kwargs",
    [
        dict(slot=SLOTS),
        dict(slot=65535),
        dict(waiting=2),
        dict(waiting=255),
        dict(speed=0),
        dict(speed=24),
        dict(speed=401),
        dict(speed=65535),
        dict(voice=3),
        dict(voice=65535),
        dict(ids=[]),
        dict(ids=[1] * 511),
        dict(ids=[0]),
        dict(ids=[178]),
        dict(style_len=0),
        dict(style_len=511),
    ],
)
def test_synthesize_rejects(kwargs):
    args = dict(slot=1, ids=[1, 2], voice=0, speed=100, priority=0, waiting=0)
    args.update(kwargs)
    with pytest.raises(P.ProtocolError):
        parse(P.synthesize(**args))


def test_extremes_accepted():
    parse(P.synthesize(SLOTS - 1, [1] * 510, 2, speed=25))
    parse(P.synthesize(0, [177], 0, speed=400, style_len=510))


@pytest.mark.parametrize("delta", [-1, 1])
def test_wrong_length_rejected(delta):
    body = P.synthesize(1, [1, 2, 3], 0)[4:]
    body = body[:delta] if delta < 0 else body + b"\x01"
    with pytest.raises(P.ProtocolError):
        P.parse_request(body, SLOTS, 3, VOCAB, 510)


@pytest.mark.parametrize("body", [b"", b"\x00", b"\x12", b"\x11", b"\x11\x00", b"\x11\x00\x00\x00",
                                  b"\x11\x04\x00", b"\x10" + bytes(11)])
def test_bad_bodies_rejected(body):
    with pytest.raises(P.ProtocolError):
        P.parse_request(body, SLOTS, 3, VOCAB, 510)


def test_largest_body_is_the_bound():
    assert len(P.synthesize(0, [1] * 510, 0)) - 4 == P.max_front_body(510) == 523


def test_fuzz_only_protocol_errors():
    """Random mutations of valid bodies and random bytes either parse to a valid request or raise
    ProtocolError; `tools/fuzz_front.py` runs this coverage-guided for longer."""
    rng = random.Random(0)
    good = [P.synthesize(2, [1, 2, 16], 1, speed=137)[4:], P.cancel(1)[4:]]
    parsed = 0
    for _ in range(20000):
        body = bytearray(rng.choice(good))
        for _ in range(rng.randint(1, 4)):
            op = rng.randrange(3)
            if op == 0 and body:
                body[rng.randrange(len(body))] = rng.randrange(256)
            elif op == 1:
                body.insert(rng.randrange(len(body) + 1), rng.randrange(256))
            elif body:
                del body[rng.randrange(len(body))]
        try:
            msg = P.parse_request(bytes(body), SLOTS, 3, VOCAB, 510)
        except P.ProtocolError:
            continue
        parsed += 1
        assert msg.slot < SLOTS
        if isinstance(msg, P.Synthesize):
            assert P.MIN_SPEED <= msg.speed <= P.MAX_SPEED and msg.voice < 3
            assert 1 <= min(msg.ids) and max(msg.ids) < VOCAB and 1 <= len(msg.ids) <= 510
    assert parsed > 200
    for _ in range(5000):
        body = bytes(rng.randrange(256) for _ in range(rng.randrange(40)))
        try:
            P.parse_request(body, SLOTS, 3, VOCAB, 510)
        except P.ProtocolError:
            pass


def test_result_frame():
    frame = P.result(7, [1, 2, 1], b"\x01\x00" * 2400)
    assert frame[:9] == struct.pack("<IBHH", 5 + 6 + 4800, P.RESULT, 7, 3)
    assert struct.unpack_from("<3H", frame, 9) == (1, 2, 1)
    with pytest.raises(ValueError):
        P.result(7, [1, 2, 1], b"\x01\x00" * 2399)


def test_integer_bounds():
    assert [P.max_duration(s) for s in (25, 30, 100, 400)] == [200, 167, 50, 13]
