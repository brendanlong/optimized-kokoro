"""The request queue: slots, (priority, arrival) order, cancellation, and batch picking. No torch here."""

import threading
from collections import deque

from .protocol import Synthesize


class Closed(Exception):
    pass


class RequestQueue:
    """Shared by the connection's reader thread (put/cancel/close) and the GPU loop (take/done).

    A slot is busy from its Synthesize until done() (or a successful cancel()), which the caller
    calls before sending the slot's terminal reply, so the front can reuse it as soon as it sees
    the reply."""

    def __init__(self, slots: int):
        self._cond = threading.Condition()
        self._queues = (deque(), deque())  # by priority
        self._busy = [False] * slots
        self._closed = False

    def put(self, request: Synthesize) -> bool:
        """False if the request's slot is busy."""
        with self._cond:
            if self._busy[request.slot]:
                return False
            self._busy[request.slot] = True
            self._queues[request.priority].append(request)
            self._cond.notify()
            return True

    def cancel(self, slot: int) -> bool:
        """True if the slot's request was still queued; it is then dropped and the slot freed."""
        with self._cond:
            for q in self._queues:
                for request in q:
                    if request.slot == slot:
                        q.remove(request)
                        self._busy[slot] = False
                        return True
            return False

    def take(self, max_batch: int) -> list[Synthesize]:
        """Block until something is queued; return up to max_batch requests, most urgent first.

        Never waits for more requests to arrive once one is there."""
        with self._cond:
            while not self._closed and not any(self._queues):
                self._cond.wait()
            if self._closed:
                raise Closed
            batch = []
            for q in self._queues:
                while q and len(batch) < max_batch:
                    batch.append(q.popleft())
            return batch

    def done(self, slot: int) -> None:
        with self._cond:
            self._busy[slot] = False

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._queues[0].clear()
            self._queues[1].clear()
            self._busy = [False] * len(self._busy)
            self._cond.notify_all()
