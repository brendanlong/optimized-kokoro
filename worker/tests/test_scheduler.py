"""The queue's order: priority (higher first), then waiting before prefetch, then arrival."""

import random

from kokoro_worker import protocol as P
from kokoro_worker.scheduler import RequestQueue


def req(slot: int, priority: int, waiting: int) -> P.Synthesize:
    return P.Synthesize(slot, priority, waiting, 100, 0, 1, (1,))


def test_higher_priority_first_then_waiting_then_arrival():
    q = RequestQueue(16)
    arrivals = [(0, 0), (255, 0), (128, 1), (0, 1), (128, 0), (255, 1), (128, 1), (7, 1)]
    for slot, (priority, waiting) in enumerate(arrivals):
        assert q.put(req(slot, priority, waiting))
    order = [m.slot for m in q.take(16)]
    assert order == [5, 1, 2, 6, 4, 7, 3, 0]


def test_batches_take_the_most_urgent():
    q = RequestQueue(8)
    for slot in range(6):
        q.put(req(slot, slot % 3, 1))
    assert [m.slot for m in q.take(2)] == [2, 5]
    assert [m.slot for m in q.take(3)] == [1, 4, 0]
    assert [m.slot for m in q.take(3)] == [3]


def test_cancel_keeps_the_order_of_the_rest():
    rng = random.Random(0)
    for _ in range(200):
        q = RequestQueue(32)
        reqs = [req(slot, rng.randrange(4), rng.randrange(2)) for slot in range(rng.randrange(1, 32))]
        for r in reqs:
            q.put(r)
        cancelled = {r.slot for r in reqs if rng.random() < 0.3}
        for slot in cancelled:
            assert q.cancel(slot)
            assert not q.cancel(slot)
        kept = [r for r in reqs if r.slot not in cancelled]
        expected = sorted(kept, key=lambda r: (-r.priority, -r.waiting, r.slot))
        if expected:  # take() would wait for one
            assert [m.slot for m in q.take(32)] == [r.slot for r in expected]
        for slot in cancelled:
            assert q.put(req(slot, 0, 0)), "a cancelled request's slot is free"


def test_busy_slots_are_refused():
    q = RequestQueue(2)
    assert q.put(req(1, 0, 0))
    assert not q.put(req(1, 255, 1))
    q.take(1)
    assert not q.put(req(1, 0, 0)), "busy until done()"
    q.done(1)
    assert q.put(req(1, 0, 0))
