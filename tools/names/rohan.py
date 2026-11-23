"""Names of Rohan, listed by hand. Tolkien wrote the Rohirrim's tongue as Old English (Appendix F), so its names
take Old English's stress, always on the first syllable (ÉO-mer, ER-ken-brand), and otherwise the letters of
Appendix E: é and the other accents are long vowels, y is French u ("Éowyn"), c and g are hard.
"""

from pathlib import Path

import tolkien

WORLD = tolkien.WORLD

NAMES = [
    "Théoden", "Théodred", "Théodwyn", "Thengel", "Éomer", "Éowyn", "Éomund", "Éothain", "Éothéod", "Éored",
    "Eorl", "Eorlingas", "Edoras", "Meduseld", "Erkenbrand", "Elfhelm", "Elfhild", "Grimbold", "Dúnhere", "Háma",
    "Gamling", "Gríma", "Fréaláf", "Fréa", "Fréawine", "Folcwine", "Folca", "Fengel", "Brego", "Baldor",
    "Aldor", "Goldwine", "Déor", "Déorwine", "Léod", "Walda", "Wídfara", "Herefara", "Herubrand", "Fastred",
    "Guthláf", "Gálmód", "Hasufel", "Arod", "Felaróf", "Windfola", "Mearas", "Mundburg", "Aldburg",
    "Halifirien", "Firienfeld", "Eastemnet", "Westemnet", "Folde", "Dernhelm", "Brytta", "Léofa", "Éofor",
    "Elfwine", "Freca", "Herugrim", "Gúthwinë", "Stybba", "Holdwine", "Gléowine",
]


def names(cache: Path, english):
    """{spelling: (phonemes, countable)}; none of these takes a plural."""
    return {n: (tolkien.pronounce(n, first=True), False) for n in NAMES}
