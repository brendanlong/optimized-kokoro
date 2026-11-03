"""Record spaCy's tokens and tags over texts, for the crate's parity test.

    reference/.cache/kfa/.venv/bin/python -I crates/spacy-en/reference.py OUT.jsonl [INPUT.jsonl ...]

Each input line is a JSON object with a "text" field (the corpus format, reference/corpus/*.jsonl).
Each paragraph is recorded whole and line by line. With no inputs, writes the synthetic cases in
tests/data/synthetic.jsonl: every tokenizer special case in a few contexts, plus edge cases.
Output lines: {"text": ..., "tokens": [[text, tag, whitespace], ...]}.
"""

import json
import sys

import spacy

EDGE_CASES = [
    "",
    " ",
    "  ",
    "\n",
    " leading space",
    "trailing space ",
    "two  spaces   three\tand\ttabs\n\nnewlines \n mixed  nbsp em　ideographic",
    "Visit https://example.com/path?q=1&r=2#frag, or www.example.org. Email me at a.b@c.co.uk!",
    "http://localhost:8080/ and 192.168.0.1 and 10.0.0.1:80/x and ftp://user:pw@host.com/f.txt.",
    "It costs $5.99, €10, £3.50, US$7, C$8, 5$, 100km, 3.5kg, 20°C. 15% off; 1,000,000 people.",
    "He said, “I can’t”—then left… Didn't he? 'Tis y'all's fault. Gimme gonna wanna.",
    "U.S.A. vs. U.K., e.g. etc. i.e. a.m. p.m. Mr. Mrs. Dr. St. Jan. Feb. No. 5 Ph.D. A.D. 1066.",
    "well-known state-of-the-art 3-4 1/2 x+y=z a<b>c a:b 10:30 2023-01-01 rock'n'roll",
    "(parenthetical) [brackets] {braces} <angles> \"quotes\" 'single' ``latex'' -- --- ...",
    ":) :-) ;) :( :P <3 xD ^_^ (^_^) o.O ¯\\_(ツ)_/¯ 😀👍🏽 #hashtag @mention",
    "ΣΊΣΥΦΟΣ ΟΔΥΣΣΕΥΣ İstanbul ǅemal ﬁne ß ẞ Ⅻ ①②③ ½ ²³ ٣٤٥ ௧௨ १२३",
    "naïve café résumé Zürich São Paulo Москва 東京 서울 עִבְרִית العربية हिन्दी",
    "CAPS LOCK SHOUTING, MiXeD cAsE, camelCase, snake_case, kebab-case, SCREAMING_SNAKE",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa long",
    "1st 2nd 3rd 4th 21st 1990s '90s the '60s 9/11 24/7 50-50 1-800-555-0199 +1 (555) 123-4567",
    "C++ C# F# .NET node.js Python3.12 v1.2.3 x86_64 file.tar.gz ./run.sh ~/home --flag=value -v",
    "Hello world.This is it.Yes!No?Maybe...Okay;fine:good,bad",
    "don't won't can't shan't ain't isn't I'm you're he's she'd we'll they've it's let's o'clock",
    "DON'T WON'T CAN'T I'M YOU'RE Dont Wont Cant Im youre",
    "‘Single curly’ “double curly” «guillemets» „low“ ‹single›",
]

LONG_CASES = [
    "a" * 300,
    "a." * 300,
    "a-" * 300,
    "1+" * 300,
    "http://" + "a" * 300 + ".com",
    "www." + "a." * 300 + "com",
    "x@" + "a." * 300 + "com",
    "(" * 300 + "word" + ")" * 300,
    "." * 300,
    "'" * 300,
    "a:" * 300,
    "-" * 300,
    "\U0001f600" * 300,
    "a.b-c/d" * 100,
    "U.S." * 100,
]

CONTEXTS = ["{}", "I said {} today.", "({})", "“{}”", "{},"]


def synthetic(nlp):
    texts = EDGE_CASES + LONG_CASES
    for chunk in sorted(nlp.tokenizer.rules):
        if not chunk.strip():
            continue
        texts.extend(c.format(chunk) for c in CONTEXTS)
    return texts


def corpus(paths):
    for path in paths:
        with open(path, encoding="utf-8") as f:
            for line in f:
                if not line.strip():
                    continue
                text = json.loads(line)["text"]
                yield text
                lines = text.splitlines()
                if len(lines) > 1:
                    yield from (l for l in lines if l.strip())


def main(out, inputs):
    nlp = spacy.load("en_core_web_sm", enable=["tok2vec", "tagger"])
    texts = corpus(inputs) if inputs else synthetic(nlp)
    seen = set()
    with open(out, "w", encoding="utf-8") as f:
        for text in texts:
            if text in seen:
                continue
            seen.add(text)
            tokens = [[t.text, t.tag_, t.whitespace_] for t in nlp(text)]
            f.write(json.dumps({"text": text, "tokens": tokens}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
