"""Pick names for the listening page and get every arm's ids for "Then NAME arrived." from the real pipeline."""

import json
import subprocess
import sys

ARMS = ["key", "current", "rules", "haiku_misaki", "opus_misaki", "opus_ipa"]
ALWAYS = ["Celebrimbor", "Galadriel", "Sauron", "Aragorn", "Legolas", "Celeborn", "Glorfindel", "Maedhros"]
LIMIT = 30


def main(scores_path, binary, out, *arm_files):
    scores = json.load(open(scores_path))
    phonemes = {"key": {r["name"]: r["key"] for r in scores["current"]["rows"]}}
    for spec in arm_files:
        label, path = spec.split("=", 1)
        phonemes[label] = {r["name"]: r["phonemes"] for r in map(json.loads, open(path))}
    # Names where the LLMs or the current pipeline get stress wrong, most arms wrong first.
    wrong = {}
    for arm in ["current", "haiku_misaki", "opus_misaki", "opus_ipa"]:
        for r in scores[arm]["rows"]:
            wrong[r["name"]] = wrong.get(r["name"], 0) + (r["stress_ok"] is False) + (r["cons_errors"] > 0)
    picked = ALWAYS + [n for n, _ in sorted(wrong.items(), key=lambda x: -x[1]) if n not in ALWAYS]
    picked = picked[:LIMIT]
    texts, rows = [], []
    for name in picked:
        for arm in ARMS:
            p = phonemes[arm][name]
            texts.append(f"Then {name} arrived." if arm == "current" else f"Then [{name}](/{p}/) arrived.")
            rows.append({"name": name, "arm": arm})
    res = subprocess.run([binary, "a"], input="\n".join(json.dumps(t) for t in texts) + "\n",
                         capture_output=True, text=True, check=True)
    for row, line in zip(rows, res.stdout.splitlines(), strict=True):
        d = json.loads(line)
        assert d["error"] is None and len(d["chunks"]) == 1, d
        row.update(phonemes=d["chunks"][0]["phonemes"], ids=d["chunks"][0]["ids"])
    json.dump(rows, open(out, "w"), ensure_ascii=False)
    print(f"{len(picked)} names, {len({tuple(r['ids']) for r in rows})} distinct renderings")


if __name__ == "__main__":
    main(*sys.argv[1:])
