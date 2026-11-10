"""Writes crates/text/data/common_words.txt: the common English words, for the rules that ask
whether a capitalised word is a name or just a word that starts a sentence (src/words.rs).

    python3 -I crates/text/tools/gen_common_words.py crates/g2p-en/data/us_gold.json crates/text/data/names.txt \
        > crates/text/data/common_words.txt

A word is common when misaki's American gold lexicon (crates/g2p-en/data, Apache-2.0) has it in
lowercase and doesn't also have it capitalised: "give", "journal", "part"; not "henry" or "louis",
which the lexicon also lists as names. ASCII letters only, two letters or more, one per line,
sorted.
"""

import json
import sys

lexicon = json.load(open(sys.argv[1]))
words = sorted(
    w for w in lexicon if len(w) >= 2 and w.isascii() and w.isalpha() and w.islower() and w.capitalize() not in lexicon
)
sys.stdout.write("".join(f"{w}\n" for w in words))
# With a second argument, also the names: the capitalised words, which are never stemmed to a
# common word ("James" isn't "jam" + "es").
if len(sys.argv) > 2:
    names = sorted(w for w in lexicon if len(w) >= 2 and w.isascii() and w.isalpha() and w[0].isupper() and w[1:].islower())
    open(sys.argv[2], "w").write("".join(f"{w}\n" for w in names))
