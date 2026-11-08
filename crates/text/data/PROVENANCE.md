# Provenance

## `given_names.txt`

The given names the Roman-numeral reader (`src/roman.rs`) takes as a person's name, so that "Henry VIII" reads "Henry the eighth" and "Saturn V" "Saturn five". 18,344 names, one per line, sorted by byte (130 KB).

- Source: the US Social Security Administration's national baby-name data, "Beyond the Top 1000 Names", `names.zip` from https://www.ssa.gov/oact/babynames/limits.html (https://www.ssa.gov/oact/babynames/names.zip), downloaded 2026-11-08: years of birth 1880–2025, every name given to at least 5 babies of one sex in a year. SHA-256 of the zip: `cd78e975ed7bb358e018dd62fbe14ced89295e9581c49172ca4eedcb011b3724`.
- License: public domain. It is a work of the US government (17 U.S.C. § 105), published without restrictions.
- Made by `tools/gen_given_names.py names.zip > data/given_names.txt`: the names given to at least 500 babies in total, over all years and both sexes. The script says why 500.

## `emoji.tsv`

Emoji names from Unicode CLDR 49's text-to-speech annotations and Unicode 17's `emoji-data.txt`, made by `tools/gen_emoji.py` (which describes the sources). Unicode License v3: `LICENSE-unicode`.
