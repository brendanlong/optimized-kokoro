"""Tolkien's Dwarvish names, listed by hand: words of Khuzdul, the Dwarves' own tongue, and the outer names
in the tongues of Men they went by, which Tolkien took from Old Norse.

Tolkien gave the rules of Appendix E for every name in the book, and the outer names follow them ("Durin" is
DOO-rin, "Balin" BAH-lin). Khuzdul's sounds differ only where its letters do (Ardalambion's "Khuzdul",
https://ardalambion.net/khuzdul.htm, from Appendix F and The Peoples of Middle-earth): kh and th are k and t
with a breath after them, which English gives k and t starting a syllable anyway, not the sounds of "bach" and
"thin"; gh is a voiced spirant; sh is as in English; a circumflex marks a long vowel. Nobody knows where
Khuzdul puts its stress, so it takes the Elvish rule (KHA-zad, kha-zad-DÛM).
"""

from pathlib import Path

import tolkien

WORLD = tolkien.WORLD

KHUZDUL_UNITS = sorted({**tolkien.DIPHTHONGS, "kh": "k", "th": "t", "gh": "ɣ", "sh": "ʃ"}.items(),
                       key=lambda kv: -len(kv[0]))

KHUZDUL = [
    "Khazad-dûm", "Khazâd", "Khazad", "Khuzdul", "Baruk Khazâd", "Khazâd ai-mênu", "Uzbad Khazad-dûmu",
    "Mazarbul", "Kheled-zâram", "Kibil-nâla", "Zirakzigil", "Zirak-zigil", "Barazinbar", "Baraz",
    "Bundushathûr", "Shathûr", "Azanulbizar", "Gabilgathol", "Tumunzahar", "Gundabad", "Tharkûn",
    "Felakgundu", "Nulukkizdîn", "Sigin-tarâg", "Azaghâl", "Gamil Zirak", "Khîm", "Ibun", "Rukhs", "Rakhâs",
    "Fundinul",
]

# Durin's line and the Dwarves of The Hobbit, The Lord of the Rings and The Silmarillion.
OUTER = [
    "Durin", "Thorin", "Thráin", "Thrór", "Dáin", "Náin", "Óin", "Glóin", "Balin", "Dwalin", "Fíli", "Kíli",
    "Bifur", "Bofur", "Bombur", "Dori", "Nori", "Ori", "Gimli", "Fundin", "Frár", "Lóni", "Nár", "Flói",
    "Borin", "Farin", "Grór", "Frerin", "Dís", "Narvi", "Mîm", "Gróin", "Náli", "Frór",
]


def names(cache: Path, english):
    """{spelling: (phonemes, countable)}; none of these takes a plural."""
    out = {n: (tolkien.pronounce(n, units=KHUZDUL_UNITS), False) for n in KHUZDUL}
    out.update({n: (tolkien.pronounce(n), False) for n in OUTER})
    return out
