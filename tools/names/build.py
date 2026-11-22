"""Build crates/g2p-en/data/names.json, the names the English G2P reads in their own language.

    python3 tools/names/build.py [--cache DIR]

Each source module (SOURCES) has names(cache, english) -> {spelling: (phonemes, countable)}: the spellings
the source attests, where english is the set of words misaki's dictionaries have (as written and
lowercase) and countable says whether the name takes a plural (Silmaril and Balrog do; Celebrimbor and
Gondor don't). Each spelling with accents also gets an accent-free twin ("Earendil" for "Eärendil"), and each
countable name its plural ("Silmarils"). A spelling is left out when:
- English already has it: misaki's dictionaries know it in any case ("Region", "Luna", "Turin");
- it's on exclude.tsv: an English reader could meet it outside the source's world (reviewed by hand from
  the flags flag.py collects in review/);
- two names that differ only in accents are read differently, for their shared accent-free twin.
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
    return words | {w.lower() for w in words}


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
    """{spelling: (phonemes, countable)}: the source's spellings and their accent-free twins."""
    twins, clashes = {}, set()
    for spelling, entry in own.items():
        plain = strip_accents(spelling)
        if plain != spelling and plain not in own:
            # Two names that only differ in accents may be read differently: then the twin is ambiguous.
            if twins.setdefault(plain, entry)[0] != entry[0]:
                clashes.add(plain)
    return {**own, **{s: e for s, e in twins.items() if s not in clashes}}


def plural(spelling, phonemes):
    """ "Silmarils", "Balrogs", with misaki's ending; none for names ending in a hissing sound (English writes "-es")."""
    hissing = spelling[-1] in "sxz" or spelling.endswith(("sh", "h")) and not spelling.endswith(("th", "dh", "ch"))
    if " " in spelling or hissing or phonemes[-1] in "sz":
        return None
    return spelling + "s", phonemes + ("s" if phonemes[-1] in "ptkfθxç" else "z")  # as lexicon.rs's s()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=Path, default=Path.home() / ".cache/kokoro-names")
    args = ap.parse_args()
    vocab = {chr(int(h, 16)) for h in re.findall(r"\('\\u\{([0-9a-f]+)\}', \d+\)", VOCAB_RS.read_text())}
    english, exclude = english_words(), excluded()
    names, unreviewed = {}, []
    for source, module in SOURCES.items():
        seen = reviewed(source)

        def keep(spelling, phonemes):
            single = " " not in spelling
            if single and (spelling in english or spelling.lower() in english) or spelling in exclude:
                return False
            if single and spelling not in seen:
                unreviewed.append(spelling)
                return False
            bad = set(phonemes) - vocab
            if bad:
                raise SystemExit(f"{spelling}: {phonemes} has symbols Kokoro doesn't know: {bad}")
            if names.get(spelling, phonemes) != phonemes:
                raise SystemExit(f"{spelling}: two sources read it differently")
            return True

        entries = candidates(module.names(args.cache, english))
        kept = {s: p for s, (p, _) in entries.items() if keep(s, p)}
        # Plurals of the countable names that are kept (Silmarils, not Celebrimbors), checked the same way.
        plurals = dict(filter(None, (plural(s, p) for s, p in kept.items() if entries[s][1])))
        kept.update({s: p for s, p in plurals.items() if s not in kept and keep(s, p)})
        names.update(kept)
        if unreviewed:
            json.dump(sorted(unreviewed), open(HERE / "review" / f"{source}.todo.json", "w"), ensure_ascii=False, indent=0)
            raise SystemExit(f"{len(unreviewed)} {source} words haven't been checked for other meanings: "
                             f"run flag.py {source}, then review its flags into exclude.tsv")
    OUT.write_text(json.dumps(dict(sorted(names.items())), ensure_ascii=False, indent=0) + "\n", encoding="utf-8")
    print(f"{len(names)} entries ({sum(' ' in n for n in names)} of several words) -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
