"""The request queue: slots, priority order, cancellation, and batch picking. No torch here.

Most urgent first: by priority (the request's level, 0..255, higher first), then a chunk a listener
is waiting for before prefetch at the same level, then by arrival."""

import heapq
import itertools
import threading

from .protocol import Synthesize


class Closed(Exception):
    pass


def _urgency(request: Synthesize, arrival: int) -> tuple[int, int, int]:
    """Sort key: smallest is most urgent."""
    return (-request.priority, -request.waiting, arrival)


class RequestQueue:
    """Shared by the connection's reader thread (put/cancel/close) and the GPU loop (take/done).

    A slot is busy from its Synthesize until done() (or a successful cancel()), which the caller
    calls before sending the slot's terminal reply, so the front can reuse it as soon as it sees
    the reply."""

    def __init__(self, slots: int):
        self._cond = threading.Condition()
        self._heap: list[tuple[tuple[int, int, int], Synthesize]] = []
        self._arrivals = itertools.count()
        self._busy = [False] * slots
        self._closed = False

    def put(self, request: Synthesize) -> bool:
        """False if the request's slot is busy."""
        with self._cond:
            if self._busy[request.slot]:
                return False
            self._busy[request.slot] = True
            heapq.heappush(self._heap, (_urgency(request, next(self._arrivals)), request))
            self._cond.notify()
            return True

    def cancel(self, slot: int) -> bool:
        """True if the slot's request was still queued; it is then dropped and the slot freed."""
        with self._cond:
            for i, (_, request) in enumerate(self._heap):
                if request.slot == slot:
                    self._heap[i] = self._heap[-1]
                    self._heap.pop()
                    heapq.heapify(self._heap)
                    self._busy[slot] = False
                    return True
            return False

    def take(self, max_batch: int) -> list[Synthesize]:
        """Block until something is queued; return up to max_batch requests, most urgent first.

        Never waits for more requests to arrive once one is there."""
        with self._cond:
            while not self._closed and not self._heap:
                self._cond.wait()
            if self._closed:
                raise Closed
            batch = []
            while self._heap and len(batch) < max_batch:
                batch.append(heapq.heappop(self._heap)[1])
            return batch

    def done(self, slot: int) -> None:
        with self._cond:
            self._busy[slot] = False

    def close(self) -> None:
        with self._cond:
            self._closed = True
            self._heap.clear()
            self._busy = [False] * len(self._busy)
            self._cond.notify_all()
