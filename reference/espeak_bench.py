"""Time per call of misaki's espeak wrappers (C espeak-ng through phonemizer) over espeak_dump.py cases,
and of the bare espeak_TextToPhonemes calls inside them; compare with crates/espeak/examples/bench.rs.

    .cache/kfa/.venv/bin/python -I espeak_bench.py cases.jsonl
"""

import collections
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import espeak_dump  # noqa: E402

cases = collections.defaultdict(list)
with open(sys.argv[1]) as f:
    for line in f:
        case = json.loads(line)
        cases[(case["wrapper"], case["language"])].append(case)

wrappers = espeak_dump.Wrappers()
for (wrapper, language), group in sorted(cases.items()):
    w, _ = wrappers.get(wrapper, language)
    inner = w.backend._espeak
    Token = collections.namedtuple("Token", "text")
    run = (lambda c: w(Token(c["text"]))) if wrapper == "fallback" else (lambda c: w(c["text"]))
    run(group[0])
    start = time.perf_counter()
    for case in group:
        run(case)
    elapsed = time.perf_counter() - start
    chunks = [text for case in group for text, _ in case.get("espeak", [])]
    start = time.perf_counter()
    for text in chunks:
        inner.text_to_phonemes(text, "^")
    raw = time.perf_counter() - start
    words = sum(len(c["text"].split()) for c in group) or 1
    print(f"{wrapper:<8} {language:<6} {len(group):>6} calls {elapsed * 1e6 / len(group):>8.1f} µs/call "
          f"{elapsed * 1e6 / words:>7.1f} µs/word   (espeak alone {raw * 1e6 / len(group):>8.1f} µs/call)")
