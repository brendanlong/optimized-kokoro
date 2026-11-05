"""Inputs for bench/clause_short.py: Gutenberg sentences with several clauses, as whole-sentence ids and
each clause's ids alone, located in the whole sentence's ids.

    python bench/clause_short_inputs.py GUTENBERG_DUMP PHONEMIZE OUT [N [FIRST]]

GUTENBERG_DUMP is the reference dump (reference/out/gutenberg.jsonl), PHONEMIZE the pipeline's phonemize
example. Clauses are cut as the chunker cuts them (at `,;:，、；：`, each keeping its mark) and phonemized
one by one. A sentence is kept when its clauses' ids, joined with spaces, give exactly its whole ids, so
every clause's place in the whole render is known. FIRST (a JSON list of {"text"}, like
bench/clause/sentences.json) puts those sentences first.
"""

import json
import random
import re
import subprocess
import sys

SPACE = 16
CLAUSE = re.compile(r"([,;:，、；：])")


def clauses(sentence):
    """split.rs `clauses`: each clause stripped, with the punctuation that ended it."""
    parts = CLAUSE.split(sentence)
    out = []
    for i in range(0, len(parts), 2):
        text = parts[i].strip()
        if text:
            out.append(text + (parts[i + 1] if i + 1 < len(parts) else ""))
    return out


def run(phonemize, texts, limit):
    res = subprocess.run([phonemize, "a", str(limit)], input="".join(json.dumps(t) + "\n" for t in texts),
                         capture_output=True, text=True, check=True)
    return [json.loads(line)["chunks"] for line in res.stdout.splitlines()]


def main():
    dump, phonemize, out = sys.argv[1:4]
    n = int(sys.argv[4]) if len(sys.argv) > 4 else 80
    sentences = []
    for line in open(dump, encoding="utf-8"):
        for s in json.loads(line)["sentences"]:
            s = s.strip()
            if 3 <= sum(s.count(c) for c in ",;:") and 80 <= len(s) <= 400:
                sentences.append(s)
    random.Random(5).shuffle(sentences)
    if len(sys.argv) > 5:
        sentences = [r["text"] for r in json.load(open(sys.argv[5], encoding="utf-8"))] + sentences
    rows = []
    for start in range(0, len(sentences), 200):
        batch = sentences[start:start + 200]
        wholes = run(phonemize, batch, 100000)
        texts = [clauses(t) for t in batch]
        flat = run(phonemize, [c for cs in texts for c in cs], 100000)
        k = 0
        for text, whole, cs in zip(batch, wholes, texts):
            chunks, k = flat[k:k + len(cs)], k + len(cs)
            if len(whole) != 1 or len(whole[0]["ids"]) > 510 or any(len(c) != 1 for c in chunks):
                continue
            ids = whole[0]["ids"]
            joined, parts = [], []
            for c in chunks:
                c = c[0]
                if joined:
                    joined.append(SPACE)
                parts.append({"text": c["text"], "ids": c["ids"], "offset": len(joined)})
                joined += c["ids"]
            if joined == ids:
                rows.append({"text": text, "ids": ids, "clauses": parts})
        if len(rows) >= n:
            break
    json.dump(rows[:n], open(out, "w"), ensure_ascii=False)
    lens = sorted(len(c["ids"]) for r in rows[:n] for c in r["clauses"])
    print(f"{len(rows[:n])} sentences, {len(lens)} clauses, {sum(x < 30 for x in lens)} under 30 ids")


if __name__ == "__main__":
    main()
