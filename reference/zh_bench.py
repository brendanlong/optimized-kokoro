"""Times misaki's ZHG2P() on each sentence of a corpus, for comparison with
`cargo run --release -p kokoro-g2p-zh --example bench -- FILE`.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I zh_bench.py corpus/wiki-zh.jsonl
"""

import json
import re
import sys
import time
import warnings

warnings.simplefilter("ignore")
import jieba  # noqa: E402

jieba.setLogLevel(60)
from misaki.zh import ZHG2P  # noqa: E402

g2p = ZHG2P()
g2p("银行行长")
sentences = []
for line in open(sys.argv[1], encoding="utf-8"):
    if line.strip():
        sentences += re.findall(r"[^。！？]*[。！？]|[^。！？]+$", json.loads(line)["text"])
repeat = 5
chars = 0
start = time.perf_counter()
for _ in range(repeat):
    for s in sentences:
        chars += len(s)
        try:
            g2p(s)
        except Exception:
            pass
elapsed = (time.perf_counter() - start) * 1e6
n = len(sentences) * repeat
print(f"{len(sentences)} sentences: {elapsed / n:.1f} µs per sentence, {elapsed / chars:.3f} µs per character")
