"""Write real_names.tsv: each Latin spelling of a family or given name on Wikidata, with the language its
bearers' countries settle on.

    python3 tools/names/wikidata.py [--cache DIR] [--refetch]

Wikidata is CC0; QLever answers whole-graph queries in seconds. Wikidata keeps an item per spelling ("Nguyễn"
and "Nguyen" are two family names) and a name's spelling is its native label (P1705), so a spelling's bearers
are the people given that item. Of a spelling's bearers who are citizens of a country countries.tsv gives
languages (at least BEARERS of them), if SHARE or more are citizens of countries of one language, the spelling
is that language's, read with the voice most of them speak; otherwise it has no language. A spelling most of
whose bearers live elsewhere isn't read as its language's: "Nguyen", "Dvorak" and "Brzezinski" belong mostly to
Americans and French people, "Nguyễn", "Dvořák" and "Brzeziński" to Vietnamese, Czechs and Poles.

The table is committed, so the names change only when it's refetched. The raw tables are kept in the cache;
--refetch downloads them again.
"""

import argparse
import collections
import csv
import re
import subprocess
import unicodedata
from datetime import date
from pathlib import Path

from spelling import fold

HERE = Path(__file__).resolve().parent
OUT = HERE / "real_names.tsv"
ENDPOINT = "https://qlever.dev/api/wikidata"
PREFIXES = ("PREFIX wd: <http://www.wikidata.org/entity/> PREFIX wdt: <http://www.wikidata.org/prop/direct/> "
            "PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> ")
# Each kind of name: the property that gives a person that name.
KINDS = {"family": "P734", "given": "P735"}
QUERIES = {
    # (name, country, people): people with the name, by citizenship (none: no P27).
    "bearers": "SELECT ?n ?c (COUNT(DISTINCT ?p) AS ?b) WHERE {{ ?p wdt:{p} ?n ; wdt:P31 wd:Q5 . "
               "OPTIONAL {{ ?p wdt:P27 ?c }} }} GROUP BY ?n ?c",
    # (name, spelling): the name's native labels.
    "spellings": "SELECT ?n ?l WHERE {{ {{ SELECT DISTINCT ?n WHERE {{ ?p wdt:{p} ?n }} }} ?n wdt:P1705 ?l }}",
}
SHARE = 0.75
BEARERS = 3


def run(query):
    # QLever answers urllib with HTTP 429; curl is fine.
    out = subprocess.run(["curl", "-sf", "-m", "1200", "-G", ENDPOINT, "--data-urlencode", "query=" + PREFIXES + query,
                          "-H", "Accept: text/tab-separated-values"], capture_output=True, check=True).stdout
    if not out.startswith(b"?"):
        raise SystemExit(f"QLever: {out[:500]!r}")
    return out


def value(v):
    """A QLever TSV term as a Q-id or a string; "" for none."""
    if m := re.fullmatch(r"<http://www\.wikidata\.org/entity/(Q\d+)>", v):
        return m[1]
    if m := re.fullmatch(r'"(.*)"(?:@[\w-]+|\^\^<[^>]*>)?', v):
        return re.sub(r"\\(.)", lambda e: {"t": "\t", "n": "\n"}.get(e[1], e[1]), m[1])
    return v if not v.startswith("<") else ""


def rows(cache, kind, query, refetch):
    path = cache / "wikidata" / f"{kind}-{query}.tsv"
    if refetch or not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(run(QUERIES[query].format(p=KINDS[kind])))
    with open(path, encoding="utf-8", newline="") as f:
        reader = csv.reader(f, delimiter="\t", quoting=csv.QUOTE_NONE)
        next(reader)
        for row in reader:
            yield [value(v) for v in row]


def countries():
    out = {}
    for line in open(HERE / "countries.tsv", encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            qid, _, languages = line.rstrip("\n").split("\t")
            out[qid] = [tuple(x.split("/")) for x in languages.split(",") if x]
    return out


def latin(spelling):
    return all(unicodedata.name(c, "").startswith("LATIN") or c in " -'’" for c in spelling)


def bearers(cache, refetch):
    """{spelling: {country: people}}, over family and given names."""
    out = collections.defaultdict(collections.Counter)
    for kind in KINDS:
        spellings = collections.defaultdict(set)
        for n, label in rows(cache, kind, "spellings", refetch):
            if n and latin(label):
                spellings[n].add(unicodedata.normalize("NFC", label))
        for n, country, people in rows(cache, kind, "bearers", refetch):
            for s in spellings.get(n, ()):
                out[s][country] += int(people)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=Path, default=Path.home() / ".cache/kokoro-names")
    ap.add_argument("--refetch", action="store_true")
    args = ap.parse_args()
    where = countries()
    rows = []
    for spelling, people in sorted(bearers(args.cache, args.refetch).items()):
        languages, voices = collections.Counter(), collections.Counter()
        for country, n in people.items():
            for language, voice in where.get(country, []):
                languages[language] += n
                voices[language, voice] += n
        known = sum(n for country, n in people.items() if where.get(country))
        if known < BEARERS:
            continue
        language, n = languages.most_common(1)[0]
        voice = max((v for v in voices if v[0] == language), key=voices.__getitem__)[1]
        settled = n >= SHARE * known
        rows.append((spelling, language if settled else "", voice if settled else "", known, n / known))
    # English names are left to English, and a spelling of no language matters only where the G2P would take it
    # for another language's name (it finds names without their accents).
    named = {fold(r[0]) for r in rows if r[1] not in ("", "en")}
    lines = [f"{s}\t{lang}\t{voice}\t{known}\t{share:.2f}" for s, lang, voice, known, share in rows
             if lang not in ("", "en") or fold(s) in named]
    OUT.write_text(f"# Generated by wikidata.py from Wikidata (QLever, {date.today().isoformat()}); don't edit.\n"
                   "# spelling\tlanguage\tvoice\tbearers with a known country\tshare of them in the most common "
                   "language's countries\n" + "\n".join(lines) + "\n", encoding="utf-8")
    print(f"{len(lines)} spellings, {sum(bool(l.split(chr(9))[1]) for l in lines)} with a language -> {OUT.name}")


if __name__ == "__main__":
    main()
