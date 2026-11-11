"""Short phrases (1-30 phonemes) and filler sentences for fitting the voice pack's short rows.

    python3 train/make_phrases.py GUTENBERG_JSONL PHONEMIZE_BIN OUT_JSON

GUTENBERG_JSONL is reference/fetch_corpus.py's corpus/gutenberg.jsonl (public-domain books);
PHONEMIZE_BIN is the pipeline's example (cargo build --release -p kokoro-pipeline --example
phonemize). Every phrase goes through the real pipeline, and is kept only if it is one chunk of
1..30 phonemes. Sources:

  sentence   a whole short sentence from the books
  start      a sentence's beginning cut at a comma or semicolon, with that punctuation
  prefix     a sentence's first few words, with the sentence's own final punctuation
  interj     interjections and one-word answers
  number     numbers, as digits and words
  heading    headings ("Chapter 12", "Part II.", "Introduction")

The output holds {"train": [...], "test": [...], "fillers": {"train": [...], "test": [...]}},
each phrase {"text", "phonemes", "ids", "kind"}; ids are the model's, without the boundary 0s.
Test phrases are a stratified 12% by length, plus the phrases short_words.py measured; the fillers
(60-100 phonemes) are split too, so the in-context reference at test time uses fillers that the
fit never saw.
"""

import collections
import json
import random
import re
import subprocess
import sys

MAX_LEN = 30
PER_LENGTH = 90  # cap on phrases per length, so the long end doesn't dominate
SEED = 0
MEASURED = ["One.", "Chapter one.", "Twelve.", "Yes.", "No.", "Wait.", "he said.", "Silent?"]

INTERJ = """Yes. No. Oh! Oh. Ah. Ah! Wait. Wait! Hmm. Hm? Well. Well? Okay. OK. Sure. Right. Fine.
Thanks. Thanks! Thank you. Please. Sorry. Hello. Hello! Hi. Hey! Goodbye. Bye. Yeah. Nope. Yep.
Really? What? Why? How? Who? When? Where? Huh? Indeed. Exactly. Absolutely. Never. Always.
Maybe. Perhaps. Of course. Of course! Not yet. Not now. Come on! Stop! Look! Listen. Quiet!
Go. Run! Help! Wow! Oops. Alas. Bravo! Cheers. Damn. Good. Great. Great! Nice. Done. Ready?
Again. Enough! Here. There. Now. Later. Soon. Tomorrow. Today. Yesterday. Agreed. True. False.
Correct. Wrong. Certainly. Definitely. Probably. Seriously? Honestly. Anyway. So? And? But.
Then what? I see. I know. I don't know. Me too. Good morning. Good night. See you. Excuse me.
Hold on. No way! Of course not. Not at all. All right. Very well. Go on. Come here. Who's there?"""
INTERJ = re.split(r"(?<=[.!?])\s+", INTERJ.strip())

# the shortest inputs there are (1-4 phonemes)
TINY = """A I O Oh Ah Eh Ow Uh Aw Hm Um Er O! Eh? Ah? Uh. Ow! Aw. I? I! Ay. Oi! Hey Ooh Oh? Ah, Oh, A,
I, Um, Er, So, Yes, No, Two Ha! Huh Huh? Hey? Hm. Mm. Shh! Oh no. Ah yes. Uh oh. Yo! Me? Us? Go!""".split()
# a prefix shouldn't stop on one of these ("In the.")
FUNCTION_WORDS = set("""the a an and or but of to in on at by for with his her my your their its our
he she we they it was is were are be that which as from mr mrs miss dr st not so if when than then""".split())
ONES = "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen nineteen".split()
TENS = "twenty thirty forty fifty sixty seventy eighty ninety".split()
ROMAN = "I II III IV V VI VII VIII IX X XI XII XIII XIV XV".split()
HEADINGS = """Introduction|Conclusion|Contents|Table of Contents|Preface|Epilogue|Prologue|Appendix|
Acknowledgements|Abstract|Summary|Notes|References|Index|Glossary|Background|Methods|Results|
Discussion|Overview|Getting Started|Installation|Usage|Examples|Frequently Asked Questions|
The End|Afterword|Foreword|Dedication|Bibliography|Further Reading|Next Steps|Related Work""".replace(
    "\n", "").split("|")


def number_words(n):
    if n < 20:
        return ONES[n]
    if n < 100:
        return TENS[n // 10 - 2] + ("" if n % 10 == 0 else "-" + ONES[n % 10])
    return ONES[n // 100] + " hundred" + ("" if n % 100 == 0 else " " + number_words(n % 100))


def sentences(path):
    """Sentences from the books' paragraphs, split at . ! ? (and closing quotes) before a capital."""
    for line in open(path, encoding="utf-8"):
        text = re.sub(r"\s+", " ", json.loads(line)["text"]).strip()
        for s in re.split(r"(?:(?<=[.!?])|(?<=[.!?][\"”’)]))\s+(?=[\"“‘(]?[A-Z])", text):
            s = s.strip()
            if 1 <= len(s.split()) <= 40 and not re.search(r"[0-9_*\[\]{}<>|/\\]", s):
                yield s


def candidates(path, rng):
    out = []
    for w in INTERJ + TINY:
        out.append((w, "interj"))
    for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ":
        out += [(c, "letter"), (c + ".", "letter"), (c + "?", "letter")]
    for n in list(range(0, 21)) + rng.sample(range(21, 1000), 60) + [1984, 2024, 1066, 1776]:
        out.append((f"{n}.", "number"))
        if n < 1000:
            out.append((number_words(n).capitalize() + ".", "number"))
    for n in rng.sample(range(1, 40), 20):
        out.append((f"Chapter {n}", "heading"))
        out.append((f"Chapter {number_words(n)}.", "heading"))
        out.append((f"Step {n}.", "heading"))
    for r in ROMAN:
        out.append((f"Part {r}.", "heading"))
        out.append((f"Chapter {r}", "heading"))
    for h in HEADINGS:
        out.append((h, "heading"))
        out.append((h + ".", "heading"))
    for s in sentences(path):
        words = s.split()
        if len(words) <= 7:
            out.append((s, "sentence"))
            continue
        end = s[-1] if s[-1] in ".!?" else "."
        for m in re.finditer(r"[,;]", s):
            head = s[: m.end()]
            if len(head.split()) <= 6:
                out.append((head, "start"))
        if rng.random() < 0.3:
            k = rng.randint(2, 5)
            head = " ".join(words[:k]).rstrip(",;:—-\"“”'’")
            if head and head[-1].isalpha() and head.split()[-1].lower() not in FUNCTION_WORDS:
                out.append((head + end, "prefix"))
    return out


def phonemize(binary, texts):
    proc = subprocess.run([binary], input="\n".join(json.dumps(t) for t in texts) + "\n",
                          capture_output=True, text=True, check=True)
    return [json.loads(line) for line in proc.stdout.splitlines()]


def main():
    path, binary, out_path = sys.argv[1:4]
    rng = random.Random(SEED)
    cands = list(dict.fromkeys(candidates(path, rng)))
    seen, phrases = set(), []
    for (text, kind), rec in zip(cands, phonemize(binary, [t for t, _ in cands])):
        if rec["error"] or len(rec["chunks"]) != 1:
            continue
        c = rec["chunks"][0]
        if not 1 <= len(c["ids"]) <= MAX_LEN or c["phonemes"] in seen:
            continue
        seen.add(c["phonemes"])
        phrases.append({"text": text, "phonemes": c["phonemes"], "ids": c["ids"], "kind": kind})

    by_len = collections.defaultdict(list)
    for p in phrases:
        by_len[len(p["ids"])].append(p)
    train, test = [], []
    for n in sorted(by_len):
        items = by_len[n]
        rng.shuffle(items)
        # hand-made kinds first, so the cap keeps them
        items.sort(key=lambda p: p["kind"] in ("sentence", "start", "prefix", "letter"))
        items = items[:PER_LENGTH]
        for i, p in enumerate(items):
            held = p["text"] in MEASURED or (len(items) >= 4 and i % 8 == 3)
            (test if held else train).append(p)

    fillers = []
    for s in sentences(path):
        if s[0].isupper() and s[-1] == "." and not re.search(r"[\"“”‘’()]", s) and 12 <= len(s.split()) <= 22:
            fillers.append(s)
    rng.shuffle(fillers)
    chosen = []
    for rec in phonemize(binary, fillers[:200]):
        if not rec["error"] and len(rec["chunks"]) == 1 and 60 <= len(rec["chunks"][0]["ids"]) <= 100:
            c = rec["chunks"][0]
            chosen.append({"text": rec["text"], "phonemes": c["phonemes"], "ids": c["ids"]})
        if len(chosen) == 8:
            break
    doc = {"train": train, "test": test, "fillers": {"train": chosen[:6], "test": chosen[6:]}}
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(doc, f, ensure_ascii=False, indent=0)
    counts = collections.Counter(len(p["ids"]) for p in train)
    print(f"{len(train)} train, {len(test)} test phrases; per length (train):",
          " ".join(f"{n}:{counts[n]}" for n in range(1, MAX_LEN + 1)))
    print("kinds:", collections.Counter(p["kind"] for p in train + test))
    for f_ in chosen:
        print(len(f_["ids"]), f_["text"])


if __name__ == "__main__":
    main()
