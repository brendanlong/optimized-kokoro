"""Coverage-guided fuzzing of the worker's front-frame decoder with atheris (libFuzzer).

    uv run --with atheris python tools/fuzz_front.py [CORPUS_DIR] -max_total_time=600

Each input is a stream of front->worker frames, read as the connection's reader thread reads
them: the length bound, `parse_request`, then the slot table (a Synthesize for a busy slot is a
violation; Cancel frees a queued slot). Anything but a ProtocolError is a bug, and so is a parsed
frame that breaks the spec's ranges.
"""

import os
import struct
import sys

import atheris

with atheris.instrument_imports():
    sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
    from kokoro_worker import protocol as P
    from kokoro_worker.scheduler import RequestQueue

SLOTS, VOICES, VOCAB, MAX_PHONEMES = 8, 54, 178, 510


def read_stream(data: bytes) -> None:
    queue = RequestQueue(SLOTS)
    pos = 0
    while pos + 4 <= len(data):
        (length,) = struct.unpack_from("<I", data, pos)
        pos += 4
        if not 1 <= length <= P.max_front_body(MAX_PHONEMES):
            raise P.ProtocolError("length")
        if pos + length > len(data):
            return  # a truncated frame: the reader would wait, then time out
        msg = P.parse_request(data[pos : pos + length], SLOTS, VOICES, VOCAB, MAX_PHONEMES)
        pos += length
        assert msg.slot < SLOTS
        if isinstance(msg, P.Cancel):
            queue.cancel(msg.slot)
            continue
        assert 0 <= msg.priority <= 1 and P.MIN_SPEED <= msg.speed <= P.MAX_SPEED and msg.voice < VOICES
        assert 1 <= len(msg.ids) <= MAX_PHONEMES and 1 <= msg.style_len <= MAX_PHONEMES
        assert min(msg.ids) >= 1 and max(msg.ids) < VOCAB
        if not queue.put(msg):
            raise P.ProtocolError("busy slot")


def test_one_input(data: bytes) -> None:
    try:
        read_stream(data)
    except P.ProtocolError:
        pass


if __name__ == "__main__":
    atheris.Setup(sys.argv, test_one_input)
    atheris.Fuzz()
