"""Listening plan for the pilot languages: "Then NAME arrived." as main reads it, and as names.json now reads it.

    python3 experiments/real_names/pilot_plan.py PHONEMIZE_BINARY OUT.json [LANGUAGE...]

Picks PICKS (well-known names) and a fixed random sample of each language's names in names.json; names.json
must be built (tools/names/build.py) and the binary built from main, so its reading is the one before.
"""

import json
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/names"))
import real  # noqa: E402

PICKS = ["Wojciech", "Grzegorz", "Przemysław", "Małgorzata", "Szczepański", "Kieślowski", "Błaszczykowski", "Zbigniew",
         "Räikkönen", "Häkkinen", "Virtanen", "Jääskeläinen", "Mäkelä", "Jukka", "Tuomas", "Dvořák", "Janáček", "Jágr",
         "Navrátilová", "Kvitová", "Přemysl", "Jiří", "Bohuslav", "Ludmila", "Joaquín", "Gálvez", "Ibáñez", "Núñez",
         "Xóchitl", "Pierluigi", "Mastroianni", "Gianluigi", "Thibault", "Geneviève", "Jean-Baptiste", "Lefebvre",
         "Erzsébet", "Szőke", "Batthyány", "Gyöngyi", "Kovács", "Horváth", "Ionuț", "Constanța", "Ștefănescu",
         "Štefánik", "Ľubomír", "Janez", "Kranjec", "Jüri", "Õunapuu", "Barış", "Yılmaz", "Çakmak", "Erdoğan",
         "Soekarno", "Tjahjono", "Bambang"]
SAMPLE = 4


def main(binary, out, *languages):
    names = json.load(open(ROOT / "crates/g2p-en/data/names.json", encoding="utf-8"))
    attributed = real.attributed()
    rows, texts = [], []
    rng = random.Random(1)
    for language in sorted(set(languages) or real.LANGUAGES):
        theirs = sorted(s for s, (lang, _) in attributed.items() if lang == language and names.get(s))
        picked = [n for n in PICKS if n in theirs] + rng.sample(theirs, SAMPLE)
        for name in picked:
            for arm, text in [("before", f"Then {name} arrived."), ("after", f"Then [{name}](/{names[name]}/) arrived.")]:
                texts.append(text)
                rows.append({"name": name, "language": real.LANGUAGE_NAMES[language], "note": "", "arm": arm})
    missing = [n for n in PICKS if n not in names or not names[n]]
    res = subprocess.run([binary, "a"], input="\n".join(json.dumps(t) for t in texts) + "\n",
                         capture_output=True, text=True, check=True)
    for row, line in zip(rows, res.stdout.splitlines(), strict=True):
        d = json.loads(line)
        assert d["error"] is None and len(d["chunks"]) == 1, d
        row.update(phonemes=d["chunks"][0]["phonemes"], ids=d["chunks"][0]["ids"])
    json.dump(rows, open(out, "w"), ensure_ascii=False)
    print(f"{len(rows) // 2} names; not in names.json: {missing}")


if __name__ == "__main__":
    main(*sys.argv[1:])
