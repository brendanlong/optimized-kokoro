"""Listening plan: "Then NAME arrived." as the pipeline reads it now, and with the native and anglicized readings as links.

    python3 plan.py PHONEMIZE_BINARY OUT.json
"""

import json
import subprocess
import sys
from pathlib import Path

ARMS = ["current", "native", "anglicized"]


def main(binary, out):
    rows, texts = [], []
    for line in open(Path(__file__).with_name("names.tsv"), encoding="utf-8"):
        if line.startswith("#") or not line.strip():
            continue
        name, language, native, anglicized, note = line.rstrip("\n").split("\t")
        for arm, ipa in zip(ARMS, [None, native, anglicized]):
            texts.append(f"Then {name} arrived." if ipa is None else f"Then [{name}](/{ipa}/) arrived.")
            rows.append({"name": name, "language": language, "note": note, "arm": arm})
    res = subprocess.run([binary, "a"], input="\n".join(json.dumps(t) for t in texts) + "\n",
                         capture_output=True, text=True, check=True)
    for row, line in zip(rows, res.stdout.splitlines(), strict=True):
        d = json.loads(line)
        assert d["error"] is None and len(d["chunks"]) == 1, d
        row.update(phonemes=d["chunks"][0]["phonemes"], ids=d["chunks"][0]["ids"])
    json.dump(rows, open(out, "w"), ensure_ascii=False)


if __name__ == "__main__":
    main(*sys.argv[1:])
