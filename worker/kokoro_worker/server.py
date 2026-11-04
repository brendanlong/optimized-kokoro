"""The worker process: load the model, listen on a Unix socket, serve one connection at a time.

    python -m kokoro_worker.server --socket /run/kokoro/worker.sock --models /models
    python -m kokoro_worker.server --models /models --export-cache DIR   # warm up, write caches, exit

Per connection, a reader thread decodes and validates frames into the queue, and the main thread
runs the GPU loop. Any protocol violation closes the connection; queued work is dropped with it.
Nothing a front does can stop the accept loop: each connection's errors end that connection only.
"""

import argparse
import logging
import os
import select
import shutil
import socket
import struct
import sys
import threading
import time

from . import protocol as P
from .scheduler import Closed, RequestQueue

log = logging.getLogger("kokoro_worker")

# Requests a front may have here at once (queued or running), each in its own slot. A queued one
# costs a few hundred bytes.
SLOTS = 1024
# A frame, once started, must arrive whole within this; a front that stalls mid-frame would
# otherwise hold the only connection slot. Sends get the same deadline (the socket timeout).
FRAME_TIMEOUT_S = 10.0


class FrameTimeout(Exception):
    pass


def _recv_exact(conn: socket.socket, n: int, deadline: float) -> bytes:
    buf = bytearray(n)
    view = memoryview(buf)
    got = 0
    while got < n:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([conn], [], [], remaining)[0]:
            raise FrameTimeout
        k = conn.recv_into(view[got:])
        if k == 0:
            raise EOFError
        got += k
    return bytes(buf)


def _wait_readable(conn: socket.socket) -> None:
    """Block until the front sends something (an idle front may wait as long as it likes)."""
    while not select.select([conn], [], [], 3600)[0]:
        pass


class Connection:
    def __init__(self, conn: socket.socket, synth, max_batch: int, frame_timeout: float = FRAME_TIMEOUT_S,
                 slots: int = SLOTS):
        self.conn = conn
        self.frame_timeout = frame_timeout
        conn.settimeout(frame_timeout)  # bounds each sendall; reads use their own deadline
        self.synth = synth
        self.max_batch = max_batch
        self.slots = slots
        self.queue = RequestQueue(slots)
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
                _wait_readable(self.conn)
                deadline = time.monotonic() + self.frame_timeout
                (length,) = struct.unpack("<I", _recv_exact(self.conn, 4, deadline))
                if not 1 <= length <= P.max_front_body(self.synth.max_phonemes):
                    raise P.ProtocolError(f"frame length {length}")
                body = _recv_exact(self.conn, length, deadline)
                msg = P.parse_request(body, self.slots, self.n_voices, self.synth.vocab_size,
                                      self.synth.max_phonemes)
                if isinstance(msg, P.Cancel):
                    if self.queue.cancel(msg.slot):
                        self.send(P.failed(msg.slot, P.CANCELLED))
                elif not self.queue.put(msg):
                    raise P.ProtocolError(f"Synthesize for slot {msg.slot}, which is busy")
        except EOFError:
            log.info("front closed the connection")
        except FrameTimeout:
            log.warning("front stalled mid-frame for %.0f s, closing", self.frame_timeout)
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
            pending = {m.slot for m in batch}
            self.stats["batches"] += 1
            self.stats["requests"] += len(batch)
            try:
                requests = [Request(list(m.ids), self.synth.style(m.voice, m.style_len), m.speed) for m in batch]
                for results in self.synth.run(requests):
                    self.stats["groups"] += 1
                    for r in results:
                        slot = batch[r.index].slot
                        if r.pcm is None:
                            code = P.TOO_LONG if r.error == "too_long" else P.INTERNAL_ERROR
                            if r.error != "too_long":
                                log.error("request failed: %s", r.error)
                            out = P.failed(slot, code)
                        else:
                            out = P.result(slot, r.durations, r.pcm)
                        pending.discard(slot)
                        # Freed before the reply goes out, so the front may reuse it on receipt.
                        self.queue.done(slot)
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
                    for slot in pending:
                        self.queue.done(slot)
                        self.send(P.failed(slot, P.INTERNAL_ERROR))
                except OSError:
                    self.close()
                    return

    def serve(self) -> None:
        reader = None
        try:
            self.send(P.hello(self.synth.vocab_size, self.synth.max_phonemes, self.max_batch, self.slots,
                              self.synth.max_chunk_samples, self.synth.voice_names))
            reader = threading.Thread(target=self.read_loop, daemon=True)
            reader.start()
            self.gpu_loop()
        except OSError as e:
            log.info("connection error: %s", e)
        finally:
            log.info("connection done: %s, %s", self.stats, memory_report())
            self.close()
            if reader is not None:
                reader.join()
            self.conn.close()


def memory_report() -> str:
    """GPU memory the allocator holds (now and at most so far) and this process's resident memory."""
    parts = []
    torch = sys.modules.get("torch")
    if torch is not None and torch.cuda.is_initialized():
        parts.append(f"GPU reserved {torch.cuda.memory_reserved() >> 20} MiB "
                     f"(peak {torch.cuda.max_memory_reserved() >> 20} MiB)")
    try:
        with open("/proc/self/status") as f:
            rss = next(line.split()[1] for line in f if line.startswith("VmRSS:"))
        parts.append(f"RSS {int(rss) >> 10} MiB")
    except (OSError, StopIteration):
        pass
    return ", ".join(parts)


def accept_loop(listener: socket.socket, synth, max_batch: int, frame_timeout: float = FRAME_TIMEOUT_S) -> None:
    """Serve connections one at a time, forever."""
    while True:
        conn, _ = listener.accept()
        log.info("front connected")
        try:
            Connection(conn, synth, max_batch, frame_timeout).serve()
        except Exception:
            log.exception("connection failed")
            conn.close()


def warmup(synth, max_batch: int) -> None:
    """Compile before accepting connections: batch 1, batches > 1, and the masked path that a
    decoder group with padding takes all have graphs of their own."""
    from .model import Request

    ids = sorted(synth.vocab_ids)
    # Warm every shape even if --max-chunk-seconds would refuse some of these requests.
    cap, synth.max_chunk_samples = synth.max_chunk_samples, 2**32 - 1
    shapes = []
    if max_batch >= 2:
        shapes += [[50, 50], [40, 44], [100, 104, 110][:max_batch]]
    shapes += [[12], [60], [180], [400]]
    try:
        for lengths in shapes:
            reqs = [Request([ids[(i * 7 + n) % len(ids)] for i in range(n)], synth.style(0, n), 100)
                    for n in lengths]

            t = time.monotonic()
            for _ in synth.run(reqs):
                pass
            log.info("warmup %s: %.1f s", lengths, time.monotonic() - t)
    finally:
        synth.max_chunk_samples = cap


CACHE_DIRS = ("TORCHINDUCTOR_CACHE_DIR", "TRITON_CACHE_DIR")


def copy_caches(src_root: str | None, dst_root: str | None) -> None:
    """Copy the compile caches between their env-var locations and a seed directory.

    KOKORO_CACHE_SEED (read-only mount): copied into the tmpfs caches before compiling.
    --export-cache: a separate run that warms up, copies the caches there and exits without ever
    listening, so a seed is only written by a worker that never saw a request."""
    for var in CACHE_DIRS:
        live = os.environ.get(var)
        if not live:
            continue
        name = os.path.basename(live.rstrip("/"))
        src = os.path.join(src_root, name) if src_root else live
        dst = os.path.join(dst_root, name) if dst_root else live
        if os.path.isdir(src):
            shutil.copytree(src, dst, dirs_exist_ok=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--socket")
    parser.add_argument("--export-cache", metavar="DIR",
                        help="warm up, write the compile caches to DIR and exit without listening")
    parser.add_argument("--models", required=True, help="directory written by kokoro_worker.convert")
    # 1 by default. On the 3060 Ti, batching 8 gives more throughput at 16 concurrent requests (173x
    # vs 161x in gVisor) and about the same at 1; see README.md.
    parser.add_argument("--max-batch", type=int, default=1)
    # 60 s: real 510-phoneme chunks reach about 44 s at speed 1, and the slowest speed (0.25) then
    # allows 240 s (9,600 frames), which peaks at 3.9 GB of GPU memory; 120 s (19,200 frames) ran
    # out of memory on an 8 GB card. A packed decoder row is held to the same frame count.
    parser.add_argument("--max-chunk-seconds", type=float, default=60.0,
                        help="longest audio one request may produce at speed 1 (scaled by 1/speed); "
                             "real 510-phoneme chunks reach about 44 s")
    parser.add_argument("--device", default="cuda")
    parser.add_argument("--fp32", action="store_true")
    parser.add_argument("--no-compile", action="store_true")
    args = parser.parse_args()
    if not args.socket and not args.export_cache:
        parser.error("--socket is required (or --export-cache)")
    if not 1 <= args.max_batch <= 64:
        parser.error("--max-batch must be 1..64")
    max_chunk_samples = round(args.max_chunk_seconds * P.SAMPLE_RATE)
    if not 1 <= max_chunk_samples <= 2**32 - 1:
        parser.error("--max-chunk-seconds out of range")
    logging.basicConfig(level=logging.INFO, stream=sys.stderr, format="%(asctime)s %(levelname)s %(message)s")

    seed = os.environ.get("KOKORO_CACHE_SEED")
    if seed and args.export_cache:
        parser.error("--export-cache starts from an empty cache; unset KOKORO_CACHE_SEED")
    if seed and os.path.isdir(seed):
        t = time.monotonic()
        copy_caches(seed, None)
        log.info("compile caches seeded from %s in %.1f s", seed, time.monotonic() - t)

    from .model import Synth

    t = time.monotonic()
    synth = Synth(args.models, device=args.device, half=not args.fp32, compile=not args.no_compile,
                  max_chunk_samples=max_chunk_samples)
    log.info("loaded in %.1f s: %d voices", time.monotonic() - t, len(synth.voice_names))
    warmup(synth, args.max_batch)
    if args.export_cache:
        copy_caches(None, args.export_cache)
        log.info("compile caches exported to %s in %.1f s; exiting", args.export_cache, time.monotonic() - t)
        return
    log.info("ready in %.1f s, %s", time.monotonic() - t, memory_report())

    try:
        os.unlink(args.socket)
    except FileNotFoundError:
        pass
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(args.socket)
    os.chmod(args.socket, 0o660)
    listener.listen(1)
    log.info("listening on %s", args.socket)
    accept_loop(listener, synth, args.max_batch)


if __name__ == "__main__":
    main()
