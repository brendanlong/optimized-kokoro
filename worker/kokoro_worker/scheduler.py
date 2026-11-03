"""The request queue: (priority, arrival) order, cancellation, and batch picking. No torch here."""

import threading
from collections import deque

from .protocol import Synthesize


class Closed(Exception):
    pass


class RequestQueue:
    """Shared by the connection's reader thread (put/cancel/close) and the GPU loop (take/done)."""

    def __init__(self, max_outstanding: int):
        self.max_outstanding = max_outstanding
        self._cond = threading.Condition()
        self._queues = (deque(), deque())  # by priority
        self._outstanding: set[int] = set()  # queued or running, until its last frame is sent
        self._closed = False

    def put(self, request: Synthesize) -> bool:
        """False if the request id is already outstanding or there are too many outstanding."""
        with self._cond:
            if request.request_id in self._outstanding or len(self._outstanding) >= self.max_outstanding:
                return False
            self._outstanding.add(request.request_id)
            self._queues[request.priority].append(request)
            self._cond.notify()
            return True

    def cancel(self, request_id: int) -> bool:
        """True if the request was still queued; it is then dropped and no longer outstanding."""
        with self._cond:
            for q in self._queues:
                for request in q:
                    if request.request_id == request_id:
                        q.remove(request)
                        self._outstanding.discard(request_id)
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

    def done(self, request_id: int) -> None:
        with self._cond:
            self._outstanding.discard(request_id)

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._queues[0].clear()
            self._queues[1].clear()
            self._outstanding.clear()
            self._cond.notify_all()
