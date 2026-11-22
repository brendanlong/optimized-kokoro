"""Listening test: Tolkien's rules in English symbols vs Kokoro's native symbols, in several voices."""

import json
import subprocess
import sys

from rules import pronounce

# Chosen to exercise each sound English lacks: rolled r, pure e/o, ch/ht as [x], Sindarin y [ü],
# long vowels, diphthongs ae/ai/au/ui, final ë, dh.
NAMES = ["Celebrimbor", "Elrond", "Morgoth", "Noldor", "Carcharoth", "Mahtan", "Emyn Muil", "Eryn Vorn",
         "Anárion", "Thranduil", "Maedhros", "Aulë", "Manwë", "Galadhon", "Lothlórien", "Glorfindel",
         "Fëanor", "Eärendil", "Círdan", "Hyarmentir"]
VOICES = ["af_heart", "am_michael", "bf_emma"]


def main(binary, out):
    texts, rows = [], []
    for name in NAMES:
        for arm in ["current", "english", "native"]:
            text = f"Then {name} arrived." if arm == "current" else f"Then [{name}](/{pronounce(name, arm)}/) arrived."
            texts.append(text)
            rows.append({"name": name, "arm": arm})
    res = subprocess.run([binary, "a"], input="\n".join(json.dumps(t) for t in texts) + "\n",
                         capture_output=True, text=True, check=True)
    plan = []
    for row, line in zip(rows, res.stdout.splitlines(), strict=True):
        d = json.loads(line)
        assert d["error"] is None and len(d["chunks"]) == 1, d
        for voice in VOICES:
            plan.append({**row, "voice": voice, "phonemes": d["chunks"][0]["phonemes"], "ids": d["chunks"][0]["ids"]})
    json.dump(plan, open(out, "w"), ensure_ascii=False)
    print(f"{len(plan)} clips")


if __name__ == "__main__":
    main(*sys.argv[1:])
