"""Worker protocol v2 framing and validation (docs/worker-protocol.md). No torch here.

Every check the spec asks for happens in `parse_request`, except whether a slot is busy, which
the queue knows; anything rejected is a ProtocolError, and the caller closes the connection.
"""

import struct
from dataclasses import dataclass

VERSION = 2
MAGIC = b"KOKW"
MAX_PHONEMES = 510
MIN_SPEED, MAX_SPEED = 25, 400  # hundredths
SAMPLE_RATE = 24000
SAMPLES_PER_FRAME = 600
DURATION_BINS = 50  # Kokoro's max_dur: a duration is a sum of this many sigmoids, over speed

HELLO, RESULT, FAILED = 0x01, 0x02, 0x03
SYNTHESIZE, CANCEL = 0x10, 0x11

CANCELLED, INTERNAL_ERROR, TOO_LONG = 1, 2, 3

SYNTHESIZE_FIXED = 12  # type, slot, priority, speed, voice, n_ids, style_len


def max_front_body(max_phonemes: int) -> int:
    return SYNTHESIZE_FIXED + max_phonemes


class ProtocolError(Exception):
    pass


@dataclass(frozen=True)
class Synthesize:
    slot: int
    priority: int
    speed: int  # hundredths
    voice: int
    style_len: int  # voice-pack row style_len - 1; n_ids except for pieces of a stitched clause
    ids: tuple[int, ...]


@dataclass(frozen=True)
class Cancel:
    slot: int


def parse_request(body: bytes, slots: int, n_voices: int, vocab_size: int, max_phonemes: int):
    """Decode and validate one front -> worker frame body (type byte included)."""
    if not body:
        raise ProtocolError("empty body")
    kind = body[0]
    if kind == CANCEL:
        if len(body) != 3:
            raise ProtocolError("Cancel: wrong length")
        (slot,) = struct.unpack_from("<H", body, 1)
        if slot >= slots:
            raise ProtocolError("slot")
        return Cancel(slot)
    if kind != SYNTHESIZE:
        raise ProtocolError(f"unknown frame type {kind:#x}")

    if len(body) < SYNTHESIZE_FIXED:
        raise ProtocolError("Synthesize: short")
    slot, priority, speed, voice, n_ids, style_len = struct.unpack_from("<HBHHHH", body, 1)
    if slot >= slots:
        raise ProtocolError("slot")
    if priority > 1:
        raise ProtocolError("priority")
    if not MIN_SPEED <= speed <= MAX_SPEED:
        raise ProtocolError("speed")
    if voice >= n_voices:
        raise ProtocolError("voice index")
    if not 1 <= n_ids <= max_phonemes:
        raise ProtocolError("n_ids")
    if not 1 <= style_len <= max_phonemes:
        raise ProtocolError("style_len")
    if len(body) != SYNTHESIZE_FIXED + n_ids:
        raise ProtocolError("Synthesize: wrong length")
    ids = tuple(body[SYNTHESIZE_FIXED:])
    if min(ids) < 1 or max(ids) >= vocab_size:
        raise ProtocolError("phoneme id out of range")
    return Synthesize(slot, priority, speed, voice, style_len, ids)


def frame(body: bytes) -> bytes:
    return struct.pack("<I", len(body)) + body


def max_duration(speed: int) -> int:
    """The longest duration one id can get at `speed` hundredths: ceil(50 / speed)."""
    return -(-DURATION_BINS * 100 // speed)


def chunk_cap(max_chunk_samples: int, speed: int) -> int:
    """Most samples one request at `speed` hundredths may produce: ceil(max_chunk_samples / speed)."""
    return -(-max_chunk_samples * 100 // speed)


def hello(vocab_size: int, max_phonemes: int, max_batch: int, slots: int, max_chunk_samples: int,
          voice_names: list[str]) -> bytes:
    body = bytearray(struct.pack("<B4sHHHHHIIH", HELLO, MAGIC, VERSION, vocab_size, max_phonemes,
                                 max_batch, slots, SAMPLE_RATE, max_chunk_samples, len(voice_names)))
    for name in voice_names:
        raw = name.encode("ascii")
        if not 1 <= len(raw) <= 64:
            raise ValueError(name)
        body += struct.pack("<B", len(raw)) + raw
    return frame(bytes(body))


def result(slot: int, durations: list[int], pcm: bytes) -> bytes:
    if len(pcm) != 2 * SAMPLES_PER_FRAME * sum(durations):
        raise ValueError("audio length doesn't match the durations")
    return frame(struct.pack(f"<BHH{len(durations)}H", RESULT, slot, len(durations), *durations) + pcm)


def failed(slot: int, code: int) -> bytes:
    return frame(struct.pack("<BHB", FAILED, slot, code))


# Front side, for the test client and tests.


def synthesize(slot: int, ids, voice: int, speed: int = 100, priority: int = 0, style_len=None) -> bytes:
    style_len = len(ids) if style_len is None else style_len
    return frame(struct.pack("<BHBHHHH", SYNTHESIZE, slot, priority, speed, voice, len(ids), style_len) + bytes(ids))


def cancel(slot: int) -> bytes:
    return frame(struct.pack("<BH", CANCEL, slot))

