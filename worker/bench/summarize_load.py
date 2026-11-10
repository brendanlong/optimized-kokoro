"""One line per concurrency of a lookahead_load.py --json file: throughput and waiting-chunk latency.

    python bench/summarize_load.py OUT.json LABEL...
"""

import json
import sys

for r in json.load(open(sys.argv[1]))["runs"]:
    w = r["waiting_ms"]
    print(*sys.argv[2:], f"c{r['concurrency']}: {r['x_realtime']}x, waiting n {w['n']} "
          f"p50 {w.get('p50')} p95 {w.get('p95')} ms")
