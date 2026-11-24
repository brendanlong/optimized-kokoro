"""Write real_names.tsv: each Latin word of the names of the people on Wikidata, with the language its
bearers' countries settle on.

    python3 tools/names/wikidata.py [--cache DIR] [--refetch]

Wikidata is CC0; QLever answers whole-graph queries in seconds. Each person with a citizenship (P27) bears the
words of their name as their country writes it: its label in the language of a country of one language
(countries.tsv), else the label for every language (mul) or in English. So a Pole bears "Brzeziński", from the
Polish label, and an American "Brzezinski". A word's language is the one most of its bearers in countries of one
language speak, if at least BEARERS of them do and SHARE or more of all its bearers are citizens of countries that
speak it, alone or with others (Czechoslovakia, for a Czech name), read with the voice most of them speak. One
bearer is enough for a spelling with a letter only that language's names use (DISTINCTIVE: Kieślowski).
Otherwise it has no language: a name of Belgians could be Flemish or Walloon, and a name of few people has too
little evidence. Nor is a name with more than ENGLISH_SHARE of its bearers in English-speaking countries (Kubiak)
read as another language's. So "Nguyen", "Dvorak" and "Brzezinski", borne mostly by Americans and French people,
aren't "Nguyễn", "Dvořák" and "Brzeziński", borne by Vietnamese, Czechs and Poles.

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
# People's labels: in English or for every language, with their citizenships; and in a language, of the citizens
# of its countries.
# People's labels in English or for every language, with their citizenship: in batches of countries, as the whole is
# more than QLever streams in its time limit.
COUNTRIES = "SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p wdt:P27 ?c ; wdt:P31 wd:Q5 } GROUP BY ?c"
PEOPLE = ("SELECT ?p ?c ?l WHERE {{ VALUES ?c {{ {countries} }} ?p wdt:P27 ?c ; wdt:P31 wd:Q5 ; rdfs:label ?l . "
          "FILTER(LANG(?l) IN (\"en\", \"mul\")) }}")
BATCH = 800_000
CITIZENS = ("SELECT ?p ?l WHERE {{ VALUES ?c {{ {countries} }} ?p wdt:P27 ?c ; wdt:P31 wd:Q5 ; rdfs:label ?l . "
            "FILTER(LANG(?l) = \"{language}\") }}")
SHARE = 0.75
BEARERS = 3
# A name with more of its bearers than this in English-speaking countries is often an English speaker's, said
# their way (Gary Kubiak), which is the one English text most likely means.
ENGLISH_SHARE = 0.1
# Letters only one language's names use: a spelling with one is that language's on the word of a single bearer
# (Kieślowski, Błaszczykowski), where another needs BEARERS.
DISTINCTIVE = {"pl": set("ąęłńśźż"), "cs": set("řůě"), "hu": set("őű"), "tr": set("ğı"), "is": set("þð")}


def run(query):
    # QLever answers urllib with HTTP 429; curl is fine.
    out = subprocess.run(["curl", "-sf", "-m", "1200", ENDPOINT, "--data-urlencode", "query=" + PREFIXES + query,
                          "-H", "Accept: text/tab-separated-values"], capture_output=True, check=True).stdout
    if not out.startswith(b"?") or b"An error has occurred while exporting" in out[-2000:]:
        raise SystemExit(f"QLever: {out[:300]!r} ... {out[-300:]!r}")
    return out


def value(v):
    """A QLever TSV term as a Q-id or a string; "" for none."""
    if m := re.fullmatch(r"<http://www\.wikidata\.org/entity/(Q\d+)>", v):
        return m[1]
    if m := re.fullmatch(r'"(.*)"(?:@[\w-]+|\^\^<[^>]*>)?', v):
        return re.sub(r"\\(.)", lambda e: {"t": "\t", "n": "\n"}.get(e[1], e[1]), m[1])
    return v if not v.startswith("<") else ""


def rows(cache, name, query, refetch, raw=False):
    path = cache / "wikidata" / f"{name}.tsv"
    if refetch or not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(run(query))
    with open(path, encoding="utf-8", newline="") as f:
        reader = csv.reader(f, delimiter="\t", quoting=csv.QUOTE_NONE)
        next(reader)
        for row in reader:
            yield [value(v) for v in row[:-1]] + [row[-1] if raw else value(row[-1])]


def countries():
    out = {}
    for line in open(HERE / "countries.tsv", encoding="utf-8"):
        if line.strip() and not line.startswith("#"):
            qid, _, languages = line.rstrip("\n").split("\t")
            out[qid] = [tuple(x.split("/")) for x in languages.split(",") if x]
    return out


def latin(spelling):
    return all(unicodedata.name(c, "").startswith("LATIN") or c in " -'’" for c in spelling)


def words(name):
    """The words of a person's name that could be names: Latin, capitalized, without digits or initials' dots."""
    for w in name.split():
        w = w.strip(",;:()[]\"“”«»")
        if len(w) > 1 and w[0].isupper() and latin(w) and not any(c.isdigit() or c == "." for c in w):
            yield unicodedata.normalize("NFC", w)


def bearers(cache, refetch, where):
    """{word: {country: people}}: each person counted once per word and citizenship."""
    labels = collections.defaultdict(dict)
    citizenships = collections.defaultdict(set)
    batches, size = [[]], 0
    for country, n in sorted(rows(cache, "countries", COUNTRIES, refetch), key=lambda r: -int(r[1])):
        if not country.startswith("Q"):
            continue
        if size + int(n) > BATCH and batches[-1]:
            batches.append([])
            size = 0
        batches[-1].append(country)
        size += int(n)
    for k, batch in enumerate(batches):
        query = PEOPLE.format(countries=" ".join(f"wd:{q}" for q in batch))
        for q, country, label in rows(cache, f"people-{k}", query, refetch, raw=True):
            labels[q]["en" if label.endswith("@en") else "mul"] = value(label)
            citizenships[q].add(country)
    by_language = collections.defaultdict(list)
    for qid, spoken in where.items():
        if len(spoken) == 1 and spoken[0][0] != "en":
            by_language[spoken[0][0]].append(qid)
    for language, qids in sorted(by_language.items()):
        query = CITIZENS.format(countries=" ".join(f"wd:{q}" for q in qids), language=language)
        for q, label in rows(cache, f"citizens-{language}", query, refetch):
            labels[q][language] = label
    out = collections.defaultdict(collections.Counter)
    for q, countries in citizenships.items():
        for country in countries:
            spoken = where.get(country, [])
            own = spoken[0][0] if len(spoken) == 1 else None
            label = labels[q].get(own) or labels[q].get("mul") or labels[q].get("en")
            for w in set(words(label or "")):
                out[w][country] += 1
    return out


# The voice for a language's names unless SHARE of their bearers speak another (es: Spain's θ only for names
# mostly of Spaniards, as most Spanish speakers, and English text's Martínezes, say s).
DEFAULT_VOICE = {"es": "es-419", "pt": "pt-br", "fr": "fr-fr"}


def voice_of(language, voices):
    theirs = {v: n for (lang, v), n in voices.items() if lang == language}
    best = max(theirs, key=theirs.__getitem__)
    default = DEFAULT_VOICE.get(language)
    return best if default is None or theirs[best] >= SHARE * sum(theirs.values()) else default


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", type=Path, default=Path.home() / ".cache/kokoro-names")
    ap.add_argument("--refetch", action="store_true")
    args = ap.parse_args()
    where = countries()
    rows = []
    for spelling, people in sorted(bearers(args.cache, args.refetch, where).items()):
        people = {c: n for c, n in people.items() if c}
        known = sum(people.values())
        # The language is the one most bearers of single-language countries are citizens of; the share counts
        # citizens of every country that speaks it, of one language or several.
        single, voices, speaking = collections.Counter(), collections.Counter(), collections.Counter()
        for country, n in people.items():
            spoken = where.get(country, [])
            for language, voice in spoken:
                speaking[language] += n
                if len(spoken) == 1:
                    single[language] += n
                    voices[language, voice] += n
        language = single.most_common(1)[0][0] if single else ""
        share = speaking[language] / known if known else 0
        english = single["en"] / known if known else 0
        needed = 1 if DISTINCTIVE.get(language, set()) & set(spelling.lower()) else BEARERS
        if language not in ("", "en") and single[language] >= needed and share >= SHARE and english <= ENGLISH_SHARE \
                or language == "en" and single[language] >= BEARERS and share >= SHARE:
            rows.append((spelling, language, voice_of(language, voices), known, share))
        else:
            rows.append((spelling, "", "", known, share))
    # English names are left to English, and an unsettled spelling matters only where the G2P would take it
    # for another language's name (it finds names without their accents).
    named = {fold(r[0]) for r in rows if r[1] not in ("", "en")}
    lines = [f"{s}\t{lang}\t{voice}\t{known}\t{share:.2f}" for s, lang, voice, known, share in rows
             if lang not in ("", "en") or fold(s) in named]
    OUT.write_text(f"# Generated by wikidata.py from Wikidata (QLever, {date.today().isoformat()}); don't edit.\n"
                   "# spelling\tlanguage\tvoice\tbearers with a citizenship\tshare of them in the most common "
                   "language's countries\n" + "\n".join(lines) + "\n", encoding="utf-8")
    print(f"{len(lines)} spellings, {sum(bool(l.split(chr(9))[1]) for l in lines)} with a language -> {OUT.name}")


if __name__ == "__main__":
    main()
