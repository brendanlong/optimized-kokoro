"""How many reference records the server's chunk packing (crates/server/src/pack.rs) changes: each
American English record's text through the server's `chunks` example, packed (LIMIT SOFT) vs not
(LIMIT 0, the reference's chunking).

    cargo build --release -p kokoro-server --example chunks
    python3 reference/pack_counts.py target/release/examples/chunks LIMIT SOFT DUMP.jsonl...
"""

import json
import subprocess
import sys


def chunks(binary, texts, limit, soft):
    res = subprocess.run([binary, str(limit), str(soft)], input="".join(json.dumps(t) + "\n" for t in texts),
                         capture_output=True, text=True, check=True)
    return [[(c["text"], len(c["ids"]), c["split"]) for c in json.loads(line)["chunks"]] for line in res.stdout.splitlines()]


def main():
    binary, limit, soft, *paths = sys.argv[1:]
    records = [json.loads(line) for path in paths for line in open(path, encoding="utf-8") if line.strip()]
    texts = [r["text"] for r in records if r["lang"] == "a" and not r.get("error")]
    before, after = chunks(binary, texts, limit, 0), chunks(binary, texts, limit, soft)
    n = lambda runs: sum(len(r) for r in runs)  # noqa: E731
    short = lambda runs: sum(1 for r in runs for c in r if c[1] < 25)  # noqa: E731
    cut = lambda runs: sum(1 for r in runs for c in r if c[2][1])  # noqa: E731
    over = lambda runs: sum(1 for r in runs for c in r if c[1] > int(soft))  # noqa: E731
    changed = sum(a != b for a, b in zip(before, after))
    lone_short = sum(1 for r in after if len(r) == 1 and r[0][1] < 25)
    print(f"limit {limit}, soft {soft}: {len(texts)} records, {changed} chunk differently; "
          f"chunks {n(before)} -> {n(after)}; under 25 ids {short(before)} -> {short(after)} "
          f"({lone_short} records that are one short chunk); clause cuts {cut(before)} -> {cut(after)}; "
          f"over the soft size {over(before)} -> {over(after)}")


if __name__ == "__main__":
    main()
