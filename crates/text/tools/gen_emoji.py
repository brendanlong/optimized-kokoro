"""Writes crates/text/data/emoji.tsv: the spoken name of each emoji, from Unicode CLDR's English
text-to-speech annotations (`tts`), for the symbol pass (src/symbols.rs).

Takes CLDR 49.0.0's `annotations/en` and `annotationsDerived/en` (cldr-json) and Unicode 17's
`emoji-data.txt`, downloading them unless paths are given:

    python3 crates/text/tools/gen_emoji.py > crates/text/data/emoji.tsv
    python3 crates/text/tools/gen_emoji.py annotations.json derived.json emoji-data.txt > crates/text/data/emoji.tsv

Each line is a sequence (with the emoji variation selectors U+FE0E/U+FE0F taken out, as the
symbol pass takes them out of text), a tab, and its name. Kept: every pictographic character
(`Extended_Pictographic`) and sequence in the base annotations, but not the other symbols CLDR
names (arrows, shapes), which the symbol pass names itself or leaves silent; and the derived
sequences without skin tones (flags, ZWJ sequences, subdivision flags), since skin tones are read
as the emoji without them. Names are made
speakable: "flag: Japan" becomes "Japan flag", parentheses, quotes, colons and commas go, "&" is
"and", "St." is "Saint", and the few digits are words.

CLDR is under the Unicode License v3 (data/LICENSE-unicode).
"""

import json
import re
import sys
import urllib.request

BASE = "https://raw.githubusercontent.com/unicode-org/cldr-json/49.0.0/cldr-json"
URLS = [
    f"{BASE}/cldr-annotations-full/annotations/en/annotations.json",
    f"{BASE}/cldr-annotations-derived-full/annotationsDerived/en/annotations.json",
    "https://www.unicode.org/Public/17.0.0/ucd/emoji/emoji-data.txt",
]
DIGIT_WORDS = {"1st": "first", "2nd": "second", "3rd": "third", "8": "eight", "10": "ten"}


def read(source):
    if source.startswith("https://"):
        with urllib.request.urlopen(source) as f:
            return f.read().decode("utf-8")
    with open(source, encoding="utf-8") as f:
        return f.read()


def pictographic(source):
    """The code points with `Extended_Pictographic` in emoji-data.txt."""
    out = set()
    for line in read(source).splitlines():
        fields = line.split("#")[0].split(";")
        if len(fields) == 2 and fields[1].strip() == "Extended_Pictographic":
            lo, _, hi = fields[0].strip().partition("..")
            out.update(range(int(lo, 16), int(hi or lo, 16) + 1))
    return out


def load(source):
    data = json.loads(read(source))
    root = data.get("annotations") or data["annotationsDerived"]
    return root["annotations"]


def skin_tone(seq):
    return any(0x1F3FB <= ord(c) <= 0x1F3FF for c in seq)


def speakable(name):
    name = name.replace("’", "'")
    if name.startswith("flag: "):
        name = name[len("flag: "):] + " flag"
    name = re.sub(r"\([^)]*\)", " ", name)
    name = name.replace("&", " and ").replace("St. ", "Saint ")
    name = re.sub(r"[“”\"!.:,]", " ", name)
    name = re.sub(r"\b(1st|2nd|3rd|8|10)\b", lambda m: DIGIT_WORDS[m.group(1)], name)
    name = " ".join(name.split())
    assert not re.search(r"[0-9#*]", name), name
    return name


def main():
    sources = sys.argv[1:] or URLS
    base, derived = (load(s) for s in sources[:2])
    emoji = pictographic(sources[2])
    out = {}
    for seq, entry in base.items():
        if "tts" not in entry or skin_tone(seq):
            continue
        if len(seq) == 1 and ord(seq) not in emoji:
            continue
        if "\u20e3" in seq:
            continue  # keycaps: the number pass reads the digit
        out[seq] = entry["tts"][0]
    for seq, entry in derived.items():
        if "tts" in entry and not skin_tone(seq) and "\u20e3" not in seq:
            out[seq] = entry["tts"][0]
    lines = {}
    for seq, name in out.items():
        key = seq.replace("️", "").replace("︎", "")
        lines[key] = speakable(name)
    for key in sorted(lines):
        print(f"{key}\t{lines[key]}")


if __name__ == "__main__":
    main()
