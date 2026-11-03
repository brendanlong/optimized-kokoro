"""Real phoneme sequences for tests and benchmarks, from the reference pipeline's committed fixture.

Each chunk the deployed server would send to the model, mapped to ids with the model's vocab.
"""

import json
import os

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "..", "..", "reference", "fixtures", "hard_cases.out.jsonl")
CORPUS = os.path.join(HERE, "corpus.jsonl")  # written by make_corpus.py, if present


def load(vocab: dict[str, int], path: str | None = None, max_len: int = 510) -> list[tuple[str, list[int]]]:
    """[(phonemes, ids)] for every English chunk, in file order."""
    path = path or (CORPUS if os.path.exists(CORPUS) else FIXTURE)
    out = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            if row.get("lang") not in ("a", "b") or "chunks" not in row:
                continue
            for chunk in row["chunks"]:
                ids = [vocab[c] for c in chunk["phonemes"] if c in vocab]
                if 1 <= len(ids) <= max_len:
                    out.append((chunk["phonemes"], ids))
    return out
