"""Writes crates/text/data/given_names.txt: the given names the Roman-numeral reader (src/roman.rs)
takes as a person's name, so that "Henry VIII" is read "Henry the eighth" and "Saturn V" "Saturn
five".

Takes the US Social Security Administration's national baby-name data (`names.zip`, from
https://www.ssa.gov/oact/babynames/names.zip; public domain, a work of the US government):

    python3 crates/text/tools/gen_given_names.py names.zip > crates/text/data/given_names.txt

The zip holds one file per year of birth (`yob1880.txt` ...), each line `Name,Sex,Count`, for
every name given to at least 5 babies of one sex that year. A name is kept if it was given to at
least MIN_BABIES babies in total, over all years and both sexes. One name per line, sorted by
byte (the reader binary-searches it).

Why 500: the list's job is to tell a person's name from the other capitalised words a numeral
follows ("Part", "Bowl", "Fantasy", "Type"). Below a few hundred babies over 146 years, the names
are mostly one-off spellings and words used as names ("Apple", "Galaxy", "Holiday", "Chapel",
"Man", all between 200 and 500), which would read "Apple II" as "Apple the second". 500 keeps
18,344 names (130 KB) and the names of most monarchs and popes that are in the data at all
("Louis", "Benedict", "Umberto", "Wilhelm"); it loses rarer historical ones ("Pius" 476,
"Xerxes" 304, "Ramesses" 214), which are read as cardinals.
"""

import collections
import io
import sys
import zipfile

MIN_BABIES = 500


def main(path):
    totals = collections.Counter()
    with zipfile.ZipFile(path) as z:
        for name in z.namelist():
            if not (name.startswith("yob") and name.endswith(".txt")):
                continue
            with z.open(name) as f:
                for line in io.TextIOWrapper(f, encoding="ascii"):
                    given, _sex, count = line.strip().split(",")
                    totals[given] += int(count)
    names = sorted(n for n, c in totals.items() if c >= MIN_BABIES)
    sys.stdout.write("".join(f"{n}\n" for n in names))


if __name__ == "__main__":
    main(sys.argv[1])
