"""Writes crates/pipeline/tests/case_sentences.txt: 400 sentences picked at random (seed 69) from four
public-domain Project Gutenberg books, for the case test (crates/pipeline/tests/case.rs).

    python3 reference/case_sentences.py

The books come from the gutenberg.pglaf.org mirror (www.gutenberg.org refuses scripts). A sentence is
kept if it has 6 to 35 words, is ASCII without underscores, brackets or asterisks, starts with a
capital, ends with ".", "!" or "?", and isn't already in capitals. Sentences end at ".", "!" or "?" and
whitespace, but not after "Mr.", "Mrs.", "Dr." or "St.".
"""

import random
import re
import urllib.request
from pathlib import Path

BOOKS = {  # Gutenberg number: path on the mirror
    35: "3/35/35-0.txt",  # The Time Machine
    84: "8/84/84-0.txt",  # Frankenstein
    1342: "1/3/4/1342/1342-0.txt",  # Pride and Prejudice
    1661: "1/6/6/1661/1661-0.txt",  # The Adventures of Sherlock Holmes
}
OUT = Path(__file__).resolve().parents[1] / "crates/pipeline/tests/case_sentences.txt"


def sentences(text):
    start = re.search(r"\*\*\* START OF .*?\*\*\*", text, re.S)
    end = re.search(r"\*\*\* END OF .*?\*\*\*", text, re.S)
    for paragraph in re.split(r"\r?\n\s*\r?\n", text[start.end() : end.start()]):
        ends = r'(?<=[.!?])(?<!\bMr\.)(?<!\bMrs\.)(?<!\bDr\.)(?<!\bSt\.)\s+(?=[A-Z"“])'
        for sentence in re.split(ends, " ".join(paragraph.split())):
            if (
                6 <= len(sentence.split()) <= 35
                and sentence.isascii()
                and not re.search(r"[_\[\]*]", sentence)
                and sentence[0].isupper()
                and sentence[-1] in ".!?"
                and sentence.upper() != sentence
            ):
                yield sentence


def main():
    found = []
    # In the order of the books' file names, as they were first picked.
    for number, path in sorted(BOOKS.items(), key=lambda b: f"{b[0]}-0.txt"):
        with urllib.request.urlopen(f"https://gutenberg.pglaf.org/{path}", timeout=60) as response:
            found.extend(sentences(response.read().decode("utf-8-sig")))
    random.seed(69)
    OUT.write_text("\n".join(random.sample(found, 400)) + "\n")


if __name__ == "__main__":
    main()
