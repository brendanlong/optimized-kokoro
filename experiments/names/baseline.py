"""The current pipeline's phonemes for each name, through the phonemize example (JSON lines in, JSON lines out)."""

import json
import subprocess
import sys


def main(key, binary, out):
    names = [json.loads(l)["name"] for l in open(key)]
    res = subprocess.run([binary, "a"], input="\n".join(json.dumps(n) for n in names) + "\n",
                         capture_output=True, text=True, check=True)
    with open(out, "w") as f:
        for name, line in zip(names, res.stdout.splitlines(), strict=True):
            phonemes = " ".join(c["phonemes"] for c in json.loads(line)["chunks"])
            f.write(json.dumps({"name": name, "phonemes": phonemes}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:])
