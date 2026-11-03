"""Worker protocol v1 framing and validation (docs/worker-protocol.md). No torch here.

Every check the spec asks for happens in `parse_request`; anything it rejects is a ProtocolError,
and the caller closes the connection.
"""

import math
import struct
from dataclasses import dataclass

VERSION = 1
MAGIC = b"KOKW"
MAX_FRONT_BODY = 64 * 1024
MAX_WORKER_BODY = 8 * 1024 * 1024
MAX_PHONEMES = 510
MAX_MIX = 8
MIN_SPEED, MAX_SPEED = 0.25, 4.0
SAMPLE_RATE = 24000

HELLO, AUDIO, DURATIONS, FAILED = 0x01, 0x02, 0x03, 0x04
SYNTHESIZE, CANCEL = 0x10, 0x11

CANCELLED, INTERNAL_ERROR, TOO_LONG = 1, 2, 3


class ProtocolError(Exception):
    pass


@dataclass(frozen=True)
class Synthesize:
    request_id: int
    priority: int
    speed: float
    voices: tuple[tuple[int, float], ...]  # (voice index, weight), weights > 0, not yet normalized
    style_len: int  # voice-pack row style_len - 1; n_ids except for pieces of a stitched clause
    ids: tuple[int, ...]


@dataclass(frozen=True)
class Cancel:
    request_id: int


def parse_request(body: bytes, n_voices: int, vocab_ids: frozenset[int], max_phonemes: int):
    """Decode and validate one front -> worker frame body (type byte included)."""
    if not body:
        raise ProtocolError("empty body")
    kind = body[0]
    if kind == CANCEL:
        if len(body) != 9:
            raise ProtocolError("Cancel: wrong length")
        return Cancel(struct.unpack_from("<Q", body, 1)[0])
    if kind != SYNTHESIZE:
        raise ProtocolError(f"unknown frame type {kind:#x}")

    if len(body) < 15:
        raise ProtocolError("Synthesize: short")
    request_id, priority, speed, mix = struct.unpack_from("<QBfB", body, 1)
    if priority > 1:
        raise ProtocolError("priority")
    if not (math.isfinite(speed) and MIN_SPEED <= speed <= MAX_SPEED):
        raise ProtocolError("speed")
    if not 1 <= mix <= MAX_MIX:
        raise ProtocolError("n_voices")
    pos = 15
    if len(body) < pos + 6 * mix + 4:
        raise ProtocolError("Synthesize: short")
    voices = []
    for _ in range(mix):
        index, weight = struct.unpack_from("<Hf", body, pos)
        pos += 6
        if index >= n_voices:
            raise ProtocolError("voice index")
        if not (math.isfinite(weight) and weight > 0):
            raise ProtocolError("voice weight")
        voices.append((index, weight))
    n_ids, style_len = struct.unpack_from("<HH", body, pos)
    pos += 4
    if not 1 <= n_ids <= max_phonemes:
        raise ProtocolError("n_ids")
    if not 1 <= style_len <= max_phonemes:
        raise ProtocolError("style_len")
    if len(body) != pos + n_ids:
        raise ProtocolError("Synthesize: wrong length")
    ids = tuple(body[pos:])
    if not vocab_ids.issuperset(ids):
        raise ProtocolError("phoneme id not in vocab")
    return Synthesize(request_id, priority, speed, tuple(voices), style_len, ids)


def frame(body: bytes) -> bytes:
    return struct.pack("<I", len(body)) + body


def hello(vocab_size: int, max_phonemes: int, max_batch: int, voice_names: list[str]) -> bytes:
    body = bytearray(struct.pack("<B4sHHHHIH", HELLO, MAGIC, VERSION, vocab_size, max_phonemes,
                                 max_batch, SAMPLE_RATE, len(voice_names)))
    for name in voice_names:
        raw = name.encode("ascii")
        if not 1 <= len(raw) <= 64:
            raise ValueError(name)
        body += struct.pack("<B", len(raw)) + raw
    return frame(bytes(body))


def audio(request_id: int, pcm: bytes) -> bytes:
    return frame(struct.pack("<BQI", AUDIO, request_id, len(pcm) // 2) + pcm)


def durations(request_id: int, frames: list[int]) -> bytes:
    return frame(struct.pack(f"<BQH{len(frames)}H", DURATIONS, request_id, len(frames), *frames))


def failed(request_id: int, code: int) -> bytes:
    return frame(struct.pack("<BQB", FAILED, request_id, code))


# Front side, for the test client and tests.


def synthesize(request_id: int, ids, voices, speed: float = 1.0, priority: int = 0, style_len=None) -> bytes:
    body = bytearray(struct.pack("<BQBfB", SYNTHESIZE, request_id, priority, speed, len(voices)))
    for index, weight in voices:
        body += struct.pack("<Hf", index, weight)
    body += struct.pack("<HH", len(ids), len(ids) if style_len is None else style_len) + bytes(ids)
    return frame(bytes(body))


def cancel(request_id: int) -> bytes:
    return frame(struct.pack("<BQ", CANCEL, request_id))
