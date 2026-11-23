"""Render each row of a listening plan through the worker: a row's `ids`, or each of its `parts`'
(experiments/names/listen_plan.py, experiments/joins/plan.py).

    python bench/name_listen.py SOCKET PLAN OUT_DIR   -> OUT_DIR/<clip_id>.wav
"""

import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from kokoro_worker import protocol as P  # noqa: E402
from kokoro_worker.client import write_wav  # noqa: E402
from load import wait_for  # noqa: E402


def clip_id(ids, voice):
    return hashlib.sha1(bytes(ids) + voice.encode()).hexdigest()[:16]


def main(sock, plan, out):
    client = wait_for(sock, 900)
    os.makedirs(out, exist_ok=True)
    done = set()
    for row in json.load(open(plan, encoding="utf-8")):
        voice = row.get("voice", "af_heart")
        for ids in [p["ids"] for p in row["parts"]] if "parts" in row else [row["ids"]]:
            k = clip_id(ids, voice)
            if k in done or os.path.exists(f"{out}/{k}.wav"):
                continue
            client.send(P.synthesize(0, ids, client.hello.voices.index(voice), 100))
            kind, _, payload = client.receive()
            assert kind == "result", (kind, payload)
            write_wav(f"{out}/{k}.wav", payload[1], client.hello.sample_rate)
            done.add(k)
    print(f"rendered {len(done)}", flush=True)


if __name__ == "__main__":
    main(*sys.argv[1:])
