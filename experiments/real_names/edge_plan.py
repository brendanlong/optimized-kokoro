"""Listening plan for names English text usually means an English speaker or a brand by, which names.json now reads
in their language: "Then NAME arrived." as main reads it and as this branch does.

    python3 experiments/real_names/edge_plan.py MAIN_PHONEMIZE BRANCH_PHONEMIZE OUT.json
"""

import json
import subprocess
import sys

EDGE = [("Rudy Giuliani", "American politician"), ("Marco Rubio", "American politician"),
        ("Tony Romo", "American football player"), ("Albert Pujols", "Dominican-American baseball player"),
        ("Sonia Sotomayor", "US Supreme Court justice"), ("Tom Lantos", "American congressman, born in Hungary"),
        ("Peugeot", "brand"), ("Pirelli", "brand"), ("Hermès", "brand"), ("Lacoste", "brand"),
        ("Benetton", "brand"), ("Olivetti", "brand")]


def main(main_bin, branch_bin, out):
    texts = [f"Then {name} arrived." for name, _ in EDGE]
    rows = []
    for arm, binary in [("before", main_bin), ("after", branch_bin)]:
        res = subprocess.run([binary, "a"], input="\n".join(json.dumps(t) for t in texts) + "\n",
                             capture_output=True, text=True, check=True)
        for (name, note), line in zip(EDGE, res.stdout.splitlines(), strict=True):
            d = json.loads(line)
            rows.append({"name": name, "language": "English text", "note": note, "arm": arm,
                         "phonemes": d["chunks"][0]["phonemes"], "ids": d["chunks"][0]["ids"]})
    rows.sort(key=lambda r: [n for n, _ in EDGE].index(r["name"]))
    json.dump(rows, open(out, "w"), ensure_ascii=False)


if __name__ == "__main__":
    main(*sys.argv[1:])
