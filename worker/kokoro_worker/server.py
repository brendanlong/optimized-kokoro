"""The worker process: load the model, listen on a Unix socket, serve one connection at a time.

    python -m kokoro_worker.server --socket /run/kokoro/worker.sock --models /models

Per connection, a reader thread decodes and validates frames into the queue, and the main thread
runs the GPU loop. Any protocol violation closes the connection; queued work is dropped with it.
"""

import argparse
import logging
import os
import socket
import struct
import sys
import threading
import time

from . import protocol as P
from .scheduler import Closed, RequestQueue

log = logging.getLogger("kokoro_worker")

MAX_OUTSTANDING = 1024


def _recv_exact(conn: socket.socket, n: int) -> bytes:
    buf = bytearray(n)
    view = memoryview(buf)
    got = 0
    while got < n:
        k = conn.recv_into(view[got:])
        if k == 0:
            raise EOFError
        got += k
    return bytes(buf)


class Connection:
    def __init__(self, conn: socket.socket, synth, max_batch: int):
        self.conn = conn
        self.synth = synth
        self.max_batch = max_batch
        self.queue = RequestQueue(MAX_OUTSTANDING)
        self.send_lock = threading.Lock()
        self.n_voices = len(synth.voice_names)
        self.stats = {"batches": 0, "requests": 0, "groups": 0}

    def send(self, data: bytes) -> None:
        with self.send_lock:
            self.conn.sendall(data)

    def close(self) -> None:
        self.queue.close()
        try:
            self.conn.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass

    def read_loop(self) -> None:
        try:
            while True:
                (length,) = struct.unpack("<I", _recv_exact(self.conn, 4))
                if not 1 <= length <= P.MAX_FRONT_BODY:
                    raise P.ProtocolError(f"frame length {length}")
                body = _recv_exact(self.conn, length)
                msg = P.parse_request(body, self.n_voices, self.synth.vocab_ids, self.synth.max_phonemes)
                if isinstance(msg, P.Cancel):
                    if self.queue.cancel(msg.request_id):
                        self.send(P.failed(msg.request_id, P.CANCELLED))
                elif not self.queue.put(msg):
                    raise P.ProtocolError("duplicate request id or too many outstanding")
        except EOFError:
            log.info("front closed the connection")
        except P.ProtocolError as e:
            log.warning("protocol violation, closing: %s", e)
        except OSError as e:
            log.info("connection error: %s", e)
        except Exception:
            log.exception("reader failed, closing")
        finally:
            self.close()

    def gpu_loop(self) -> None:
        import torch

        from .model import Request

        while True:
            try:
                batch = self.queue.take(self.max_batch)
            except Closed:
                return
            pending = {m.request_id for m in batch}
            self.stats["batches"] += 1
            self.stats["requests"] += len(batch)
            try:
                requests = [Request(list(m.ids), self.synth.style(list(m.voices), len(m.ids)), m.speed)
                            for m in batch]
                for results in self.synth.run(requests):
                    self.stats["groups"] += 1
                    for r in results:
                        rid = batch[r.index].request_id
                        if r.pcm is None:
                            code = P.TOO_LONG if r.error == "too_long" else P.INTERNAL_ERROR
                            if r.error != "too_long":
                                log.error("request failed: %s", r.error)
                            out = P.failed(rid, code)
                        else:
                            out = P.durations(rid, r.durations) + P.audio(rid, r.pcm)
                        pending.discard(rid)
                        self.queue.done(rid)
                        self.send(out)
            except OSError:
                self.close()
                return
            except Exception as e:
                log.exception("batch failed")
                if isinstance(e, torch.OutOfMemoryError):
                    torch.cuda.empty_cache()
                elif "CUDA" in str(e) or "cuda" in str(e):
                    # The context may be unusable now; let the supervisor restart a clean process.
                    log.critical("CUDA error, exiting")
                    os._exit(3)
                try:
                    for rid in pending:
                        self.queue.done(rid)
                        self.send(P.failed(rid, P.INTERNAL_ERROR))
                except OSError:
                    self.close()
                    return

    def serve(self) -> None:
        self.send(P.hello(self.synth.vocab_size, self.synth.max_phonemes, self.max_batch, self.synth.voice_names))
        reader = threading.Thread(target=self.read_loop, daemon=True)
        reader.start()
        try:
            self.gpu_loop()
        finally:
            log.info("connection done: %s", self.stats)
            self.close()
            reader.join()
            self.conn.close()


def warmup(synth, max_batch: int) -> None:
    """Compile before accepting connections. Batch 1 and >1 specialize, and a decoder group with
    padding takes the masked path, so warm each of those."""
    from .model import Request

    ids = sorted(synth.vocab_ids)
    shapes = [[12], [60], [180], [400]]
    if max_batch >= 2:
        shapes += [[50, 50], [40, 44], [100, 104, 110][:max_batch]]
    for lengths in shapes:
        reqs = [Request([ids[(i * 7 + n) % len(ids)] for i in range(n)], synth.style([(0, 1.0)], n), 1.0)
                for n in lengths]
        t = time.monotonic()
        for _ in synth.run(reqs):
            pass
        log.info("warmup %s: %.1f s", lengths, time.monotonic() - t)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket", required=True)
    parser.add_argument("--models", required=True, help="directory written by kokoro_worker.convert")
    parser.add_argument("--max-batch", type=int, default=8)
    parser.add_argument("--max-frames", type=int, default=4096, help="longest output per request (40 per second)")
    parser.add_argument("--pad-slack", type=float, default=0.25,
                        help="decoder groups may hold this fraction of padding frames")
    parser.add_argument("--no-exact", action="store_true",
                        help="pad decoder groups without masking (faster, changes the audio slightly)")
    parser.add_argument("--device", default="cuda")
    parser.add_argument("--fp32", action="store_true")
    parser.add_argument("--no-compile", action="store_true")
    args = parser.parse_args()
    if not 1 <= args.max_batch <= 64:
        parser.error("--max-batch must be 1..64")
    if not 1 <= args.max_frames * 600 * 2 + 13 <= P.MAX_WORKER_BODY:
        parser.error("--max-frames: Audio frames must fit the protocol's 8 MiB cap")
    logging.basicConfig(level=logging.INFO, stream=sys.stderr, format="%(asctime)s %(levelname)s %(message)s")

    from .model import Synth

    t = time.monotonic()
    synth = Synth(args.models, device=args.device, half=not args.fp32, compile=not args.no_compile,
                  max_frames=args.max_frames, pad_slack=args.pad_slack, exact=not args.no_exact)
    log.info("loaded in %.1f s: %d voices", time.monotonic() - t, len(synth.voice_names))
    warmup(synth, args.max_batch)
    log.info("ready in %.1f s", time.monotonic() - t)

    try:
        os.unlink(args.socket)
    except FileNotFoundError:
        pass
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(args.socket)
    os.chmod(args.socket, 0o660)
    listener.listen(1)
    log.info("listening on %s", args.socket)
    while True:
        conn, _ = listener.accept()
        log.info("front connected")
        Connection(conn, synth, args.max_batch).serve()


if __name__ == "__main__":
    main()
