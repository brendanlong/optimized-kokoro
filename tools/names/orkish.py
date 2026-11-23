"""Orkish and Black Speech names and words, listed by hand. Their sounds are Appendix E's, except kh, a spirant
as in "bach" (as Elvish ch), gh its voiced fellow, and sh as in English (Ardalambion, "Orkish and the Black
Speech", https://ardalambion.net/o2.htm). Nobody knows where they put their stress, so they take the Elvish
rule (NAZ-gûl), leaving the endings -hai ("folk") and -ishi ("in") unstressed (U-ruk-hai).
"""

from pathlib import Path

import tolkien

WORLD = tolkien.WORLD

ENDINGS = tolkien.PARTICLES | {"hai", "ishi"}
UNITS = sorted({**tolkien.DIPHTHONGS, "kh": "x", "gh": "ɣ", "sh": "ʃ", "th": "θ"}.items(), key=lambda kv: -len(kv[0]))

# Names, and the words of the Ring-inscription other than ash and agh, which English has.
NAMES = [
    "Nazgûl", "Lugbúrz", "Uruk-hai", "Olog-hai", "Uglúk", "Grishnákh", "Shagrat", "Gorbag", "Lugdush", "Mauhúr",
    "Muzgash", "Lagduf", "Radbug", "Ufthak", "Ghâsh", "Snaga", "Sharkû", "Azog", "Bolg", "Golfimbul", "Golug",
    "nazg", "durbatulûk", "gimbatul", "thrakatulûk", "burzum-ishi", "krimpatul",
]


def names(cache: Path, english):
    """{spelling: (phonemes, countable)}; none of these takes a plural."""
    return {n: (tolkien.pronounce(n, units=UNITS, particles=ENDINGS), False) for n in NAMES}
