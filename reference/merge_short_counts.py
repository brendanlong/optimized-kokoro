"""How many reference records the server's short-sentence merging (crates/server/src/merge.rs)
changes: the reference's chunks of each record, merged by worker/bench/merge_short.py's copy of the
rule at the reference's 450 phonemes.

    python3 reference/merge_short_counts.py [MIN ...] < (records: reference/out/*.jsonl ...)

Pass the dumps as arguments after `--`, e.g.
    python3 reference/merge_short_counts.py 25 50 80 -- reference/fixtures/hard_cases.out.jsonl reference/out/gutenberg.jsonl reference/out/wiki-en.jsonl
"""

import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "../worker/bench"))
from merge_short import merge  # noqa: E402


def main():
    args = sys.argv[1:]
    split = args.index("--")
    mins, paths = [int(a) for a in args[:split]], args[split + 1 :]
    records = [json.loads(line) for path in paths for line in open(path, encoding="utf-8") if line.strip()]
    print(f"{len(records)} records, {sum(len(r['chunks']) for r in records)} chunks")
    for m in mins:
        changed = chunks_before = chunks_after = short_before = short_after = merged_in = 0
        for r in records:
            chunks = [{"texts": [c["text"]], "phonemes": c["phonemes"], "ids": [0] * len(c["phonemes"])}
                      for c in r["chunks"] if c["phonemes"]]
            out = merge(chunks, min_len=m)
            chunks_before += len(chunks)
            chunks_after += len(out)
            short_before += sum(len(c["phonemes"]) < m for c in chunks)
            short_after += sum(len(c["phonemes"]) < m for c in out)
            merged_in += sum(len(c["texts"]) for c in out if len(c["texts"]) > 1)
            changed += len(out) != len(chunks)
        print(f"MIN {m}: {changed} records change; chunks {chunks_before} -> {chunks_after} "
              f"({merged_in} chunks merged into {merged_in - (chunks_before - chunks_after)}); "
              f"chunks under MIN {short_before} -> {short_after}")


if __name__ == "__main__":
    main()
