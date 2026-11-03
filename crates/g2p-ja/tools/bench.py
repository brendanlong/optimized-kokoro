"""Times the reference JAG2P on the texts of a JSON-lines file (the same as `examples/bench.rs`).

    ../../reference/.cache/kfa/.venv/bin/python -I tools/bench.py TEXTS.jsonl [REPEAT]
"""

import json
import resource
import sys
import time


def main():
    texts = [json.loads(l)["text"] for l in open(sys.argv[1]) if l.strip()]
    repeat = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    before = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
    start = time.perf_counter()
    from misaki.ja import JAG2P

    g2p = JAG2P()
    g2p("日本")
    load = time.perf_counter() - start
    after = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
    print(f"load {load:.2f} s, max RSS {before:.0f} MB -> {after:.0f} MB")
    best = float("inf")
    for _ in range(repeat):
        start = time.perf_counter()
        for t in texts:
            try:
                g2p(t)
            except Exception:
                pass
        best = min(best, time.perf_counter() - start)
    chars = sum(len(t) for t in texts)
    status = dict(l.split(':', 1) for l in open('/proc/self/status'))
    print('RSS after the texts: ' + ', '.join(f"{k} {int(status[k].split()[0]) / 1024:.0f} MB" for k in ('VmRSS', 'RssAnon', 'RssFile')))
    print(f"{len(texts)} texts, {chars} chars: {best * 1e6 / len(texts):.1f} µs per text, {best * 1e6 / chars:.2f} µs per char (best of {repeat})")


if __name__ == "__main__":
    main()
