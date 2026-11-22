"""Build crates/g2p-en/data/names.json, the names the English G2P reads in their own language.

    python3 tools/names/build.py [--cache DIR]

Each source module (SOURCES) gives {spelling: phonemes}. Every word of a multi-word name becomes an entry
too, and every spelling gets an accent-free twin ("Earendil" for "Eärendil"). A spelling is left out when:
- English already has it: misaki's dictionaries know it in any case ("Region", "Luna", "Turin");
- it's on exclude.tsv: an English reader could meet it outside the source's world (reviewed by hand from
  the flags flag.py collects in review/);
- two entries spell the same but are pronounced differently, unless one is the source's own spelling.
Single words must have been through flag.py; the build stops if one hasn't.
"""

import argparse
import json
import re
import sys
import unicodedata
from pathlib import Path

import tolkien

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
OUT = ROOT / "crates/g2p-en/data/names.json"
SOURCES = {"tolkien": tolkien}
VOCAB_RS = ROOT / "crates/pipeline/src/vocab.rs"


def strip_accents(s):
    return unicodedata.normalize("NFC", "".join(c for c in unicodedata.normalize("NFD", s) if not unicodedata.combining(c)))


def english_words():
    words = set()
    for f in ("us_gold.json", "us_silver.json", "gb_gold.json", "gb_silver.json"):
        words.update(json.load(open(ROOT / "crates/g2p-en/data" / f)))
    return words


def excluded():
    out = {}
    for line in open(HERE / "exclude.tsv", encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            spelling, _, reason = line.rstrip("\n").partition("\t")
            out[spelling] = reason
    return out


def reviewed(source):
    path = HERE / "review" / f"{source}.json"
    return set(json.load(open(path))["reviewed"]) if path.exists() else set()


def candidates(own):
    """{spelling: (phonemes, is the source's own spelling)} from one source's names."""
    out = {}

    def add(spelling, phonemes, primary):
        old = out.get(spelling)
        if old is None or (primary and not old[1]):
            out[spelling] = (phonemes, primary)
        elif old[0] != phonemes and primary == old[1]:
            out[spelling] = (None, primary)  # two readings: drop it

    for name, phonemes in own.items():
        add(name, phonemes, True)
    for name, phonemes in own.items():
        words, readings = name.split(), phonemes.split()
        if len(words) > 1 and len(words) == len(readings):
            for w, p in zip(words, readings):
                if w[0].isupper():
                    add(w, p, w in own)
    for spelling, (phonemes, primary) in list(out.items()):
        plain = strip_accents(spelling)
        if plain != spelling and phonemes is not None:
            add(plain, phonemes, False)
    return {s: p for s, (p, _) in out.items() if p is not None}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=Path, default=Path.home() / ".cache/kokoro-names")
    args = ap.parse_args()
    vocab = {chr(int(h, 16)) for h in re.findall(r"\('\\u\{([0-9a-f]+)\}', \d+\)", VOCAB_RS.read_text())}
    english, exclude = english_words(), excluded()
    names, unreviewed = {}, []
    for source, module in SOURCES.items():
        seen = reviewed(source)
        for spelling, phonemes in candidates(module.names(args.cache)).items():
            single = " " not in spelling
            if single and (spelling in english or spelling.lower() in english):
                continue
            if spelling in exclude:
                continue
            if single and spelling not in seen:
                unreviewed.append(spelling)
                continue
            bad = set(phonemes) - vocab
            if bad:
                raise SystemExit(f"{spelling}: {phonemes} has symbols Kokoro doesn't know: {bad}")
            if names.get(spelling, phonemes) != phonemes:
                raise SystemExit(f"{spelling}: two sources read it differently")
            names[spelling] = phonemes
        if unreviewed:
            json.dump(sorted(unreviewed), open(HERE / "review" / f"{source}.todo.json", "w"), ensure_ascii=False, indent=0)
            raise SystemExit(f"{len(unreviewed)} {source} words haven't been checked for other meanings: "
                             f"run flag.py {source}, then review its flags into exclude.tsv")
    OUT.write_text(json.dumps(dict(sorted(names.items())), ensure_ascii=False, indent=0) + "\n", encoding="utf-8")
    print(f"{len(names)} entries ({sum(' ' in n for n in names)} of several words) -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
