"""Renders clauses alone and in their sentence (inputs from bench/clause_short_inputs.py), to see from
what length a clause rendered by itself sounds like the same words inside the sentence.

    python bench/clause_short.py SOCKET INPUTS OUT_DIR

Per sentence, OUT_DIR/short-N.npz: `whole` and `whole_dur` (the sentence in one request), and per clause k
`own_k`/`own_dur_k` (alone, with its own style row, as the server sends a chunk) and `row_k`/`row_dur_k`
(alone, with the whole sentence's style row). af_heart, speed 1.
"""

import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def main():
    from kokoro_worker import protocol as P
    from load import wait_for

    sock, path, out = sys.argv[1:4]
    os.makedirs(out, exist_ok=True)
    client = wait_for(sock, 900)
    voice = client.hello.voices.index("af_heart")

    def render(ids, style_len):
        client.send(P.synthesize(0, ids, voice, 100, style_len=style_len))
        kind, _, payload = client.receive()
        assert kind == "result", (kind, payload)
        durations, pcm = payload
        return np.frombuffer(pcm, dtype="<i2"), np.array(durations)

    for i, row in enumerate(json.load(open(path, encoding="utf-8"))):
        arrays = {}
        arrays["whole"], arrays["whole_dur"] = render(row["ids"], len(row["ids"]))
        for k, c in enumerate(row["clauses"]):
            arrays[f"own_{k}"], arrays[f"own_dur_{k}"] = render(c["ids"], len(c["ids"]))
            arrays[f"row_{k}"], arrays[f"row_dur_{k}"] = render(c["ids"], len(row["ids"]))
        np.savez(f"{out}/short-{i}.npz", **arrays)
        print(f"short {i}: {len(row['clauses'])} clauses", flush=True)


if __name__ == "__main__":
    main()
