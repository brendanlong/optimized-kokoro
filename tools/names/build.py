"""Build crates/g2p-en/data/names.json, the names the English G2P reads in their own language.

    python3 tools/names/build.py [--cache DIR]

Each source module (SOURCES) has names(cache, english) -> {spelling: (phonemes, countable)}: the spellings
the source attests, where english is the set of words misaki's dictionaries have (as written and
lowercase) and countable says whether the name takes a plural (Silmaril and Balrog do; Celebrimbor and
Gondor don't). A spelling with phonemes None is attested but isn't the name ("Nguyen" beside "Nguyễn", real.py).
Each countable name also gets its plural ("Silmarils"). A spelling is left out when:
- English already has it: misaki's dictionaries know it in any case ("Region", "Luna"), unless override.tsv
  says misaki's entry is only its guess at the name ("Thrain");
- it's on exclude.tsv: an English reader could meet it outside the source's world (reviewed by hand from
  the flags flag.py collects in review/).
The G2P finds a name in any case and without its accents ("EARENDIL" is "Eärendil"), so a spelling without
the accents that is left out for either reason ("Turin" for "Túrin", "Dior" for "Díor"), or that was never
checked, is written with no phonemes: it isn't the name.
A pipeline that reads Latin text in another language also skips the names exclude_languages.tsv lists
for it, common words there ("Este" in Portuguese), written to names_skip.json: a skipped spelling without
accents skips the name written without them, and one with accents only itself ("Estë" is still the name).
Single words must have been through flag.py; the build stops if one hasn't.
"""

import argparse
import json
import re
import sys
import unicodedata
from pathlib import Path

import dwarves
import orkish
import real
import rohan
import tolkien
from spelling import accents_within, fold, marks

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
OUT = ROOT / "crates/g2p-en/data/names.json"
# Names each espeak pipeline that reads Latin text as its own language skips: common words there.
OUT_SKIP = ROOT / "crates/g2p-en/data/names_skip.json"
SOURCES = {"tolkien": tolkien, "dwarves": dwarves, "rohan": rohan, "orkish": orkish, "real": real}
VOCAB_RS = ROOT / "crates/phonemes/src/lib.rs"


def strip_accents(s):
    return unicodedata.normalize("NFC", "".join(c for c in unicodedata.normalize("NFD", s) if not unicodedata.combining(c)))


def english_words():
    words = set()
    for f in ("us_gold.json", "us_silver.json", "gb_gold.json", "gb_silver.json"):
        words.update(json.load(open(ROOT / "crates/g2p-en/data" / f)))
    return words | {w.lower() for w in words}


def listed(file):
    out = {}
    for line in open(HERE / file, encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            spelling, _, reason = line.rstrip("\n").partition("\t")
            out[spelling] = reason
    return out


def excluded_by_language():
    out = {}
    for line in open(HERE / "exclude_languages.tsv", encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            language, spelling, _ = line.rstrip("\n").split("\t", 2)
            out.setdefault(language, set()).add(spelling)
    return out


def flags(source):
    path = HERE / "review" / f"{source}.json"
    return set(json.load(open(path))["flagged"]) if path.exists() else set()


def reviewed(source):
    path = HERE / "review" / f"{source}.json"
    return set(json.load(open(path))["reviewed"]) if path.exists() else set()


def unaccented(own):
    """{spelling: (phonemes, countable)}: the source's spellings without their accents, where the source
    doesn't write them so, and the names that differ only in accents read alike."""
    plain, clashes = {}, set()
    for spelling, entry in own.items():
        bare = strip_accents(spelling)
        if bare != spelling and bare not in own:
            if plain.setdefault(bare, entry)[0] != entry[0]:
                clashes.add(bare)
    return {s: e for s, e in plain.items() if s not in clashes}


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
    # The letters and marks of the vocabulary's IPA; kokoro-phonemes' tests check each name reads whole.
    ipa = re.findall(r'^    \(\d+, \'.*\', "(.*)"\),$', VOCAB_RS.read_text(), re.M)
    unescape = lambda s: re.sub(r"\\u\{([0-9a-f]+)\}|\\(.)", lambda m: chr(int(m[1], 16)) if m[1] else m[2], s)
    vocab = {c for s in ipa for c in unescape(s)}
    override = set(listed("override.tsv"))
    if stale := {s for s in override if s.lower() not in english_words()}:
        raise SystemExit(f"override.tsv: misaki's dictionaries don't have {sorted(stale)}")
    english = english_words() - override - {s.lower() for s in override}
    exclude = listed("exclude.tsv")
    names, unreviewed, skips = {}, [], []
    for source, module in SOURCES.items():
        seen = reviewed(source)
        # A source may leave out every spelling flag.py flags, rather than the ones exclude.tsv picks by hand.
        flagged = flags(source) if getattr(module, "EXCLUDE_FLAGGED", False) else set()

        def keep(spelling, phonemes):
            if phonemes is None:
                return False
            single = " " not in spelling
            if single and (spelling in english or spelling.lower() in english) or spelling in exclude or spelling in flagged:
                return False
            if single and spelling not in seen:
                unreviewed.append(spelling)
                return False
            bad = set(phonemes) - vocab
            if bad:
                raise SystemExit(f"{spelling}: {phonemes} has sounds Kokoro doesn't know: {bad}")
            if names.get(spelling, phonemes) != phonemes:
                raise SystemExit(f"{spelling}: two sources read it differently")
            return True

        entries = {**(own := module.names(args.cache, english)), **unaccented(own)}
        kept = {s: p for s, (p, _) in entries.items() if keep(s, p)}
        # Plurals of the countable names that are kept (Silmarils, not Celebrimbors), checked the same way.
        plurals = dict(filter(None, (plural(s, p) for s, p in kept.items() if entries[s][1])))
        kept.update({s: p for s, p in plurals.items() if s not in kept and keep(s, p)})
        # The G2P finds a name in any case and without its accents: a spelling that would find a kept name
        # but wasn't kept itself (left out above, or never checked, like "Quendes" for "Quendës") is written
        # without phonemes, and where every spelling of a name reads alike, one is enough.
        rejected = (set(entries) | {strip_accents(s) for s in kept}) - kept.keys()
        by_key = {}
        for s, p in kept.items():
            by_key.setdefault(fold(s), {})[s] = p
        for s in rejected:
            if fold(s) in by_key:
                by_key[fold(s)][s] = None
        for spellings in by_key.values():
            if len(set(spellings.values())) == 1:
                # A spelling the G2P finds through another ("Earendil" through "Eärendil") isn't needed.
                covered = lambda s: any(t != s and marks(t) > marks(s) and accents_within(s, t) for t in spellings)
                spellings = {s: p for s, p in spellings.items() if not covered(s)}
            names.update(spellings)
        if unreviewed:
            json.dump(sorted(unreviewed), open(HERE / "review" / f"{source}.todo.json", "w"), ensure_ascii=False, indent=0)
            raise SystemExit(f"{len(unreviewed)} {source} words haven't been checked for other meanings: "
                             f"run flag.py {source}, then review its flags into exclude.tsv")
        if hasattr(module, "skip"):
            skips.append(module.skip({s for s in kept if " " not in s}))
    OUT.write_text(json.dumps(dict(sorted(names.items())), ensure_ascii=False, indent=0) + "\n", encoding="utf-8")
    skip = {}
    for language, spellings in sorted(excluded_by_language().items()):
        missing = {s for s in spellings if fold(s) not in {fold(n) for n in names}}
        if missing:
            raise SystemExit(f"exclude_languages.tsv: {language} {sorted(missing)} aren't in names.json")
        skip[language] = set(spellings)
    for source_skip in skips:
        for language, spellings in source_skip.items():
            skip.setdefault(language, set()).update(spellings)
    skip = {language: sorted(spellings) for language, spellings in sorted(skip.items())}
    OUT_SKIP.write_text(json.dumps(skip, ensure_ascii=False, indent=0) + "\n", encoding="utf-8")
    print(f"{len(names)} entries ({sum(' ' in n for n in names)} of several words) -> {OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    sys.exit(main())
