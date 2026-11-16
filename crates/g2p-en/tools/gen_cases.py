"""Writes tests/data/misaki_cases.jsonl: misaki's English G2P run directly on texts that exercise its rules,
for American and British. There are no numbers, symbols or number flags: the text normalizer reads those
before G2P, and tests/parity.rs skips calls with them.

Records use reference/dump.py's format, one call per record, so crates/reference reads them.
Run with the reference venv from the repository root:
    reference/.cache/kfa/.venv/bin/python -I crates/g2p-en/tools/gen_cases.py [espeak-ng-data dir]
"""

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ESPEAK_DATA = os.path.join(HERE, "..", "..", "..", "reference", ".cache", "espeak-ng-data")

TEXTS = [
    # Phoneme links
    "[read](/ɹˈɛd/) the [Kokoro](/kˈOkəɹO/) [two words](/tˈu wˈɜɹdz/) [x](/ /) [y](//)",
    # Capitalization and stress
    "THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG. AM I IN? TO BE OR NOT TO BE.",
    "NASA's FBI's CEOs ABCs GPUs URLs PhDs DVDs iPhones eBay McDonald's MacBook YouTube JavaScript",
    "Running RUNNING Runs RUNS Walked WALKED Cities CITIES Studies STUDYING Tried TRIED",
    "ELIZABETH Elizabeth elizabeth, LONDON London, OK Ok ok, TV tv, USA usa Usa",
    # Function words and context
    "to the end, to an end, to go, to eat, going to, TO BE, To me. In it, in the, IN IT, Come in!",
    "the apple, the pear, the hour, the one, the eulogy, THE END, The end.",
    "I used to go. I used it. used cars, He is used to it. USED TO. They used two.",
    "Stand by. He went by. By and by. BY the way, close by, passed by quickly.",
    "England vs. France, cats vs dogs, A VS B, Ali vs. Frazier.",
    # Symbols, punctuation, quotes and dashes
    "\"Hello,\" she said. 'Goodbye,' he replied — then left… ‘Quoted’ “double” «guillemets»",
    "Hmm... well, uh, I don't know?! Really?!? Yes!!! No... Okay.",
    "rock 'n' roll, ’tis, 'twas, o'clock, y'all, ma'am, fo'c'sle, dogs' bones, James' book, Chris's",
    "can't won't shouldn't they're we'd you'll I've it's its let's ain't gonna wanna",
    "e.g. i.e. U.S. U.S.A. U.K. a.m. p.m. Ph.D. etc. vs. Dr. Mr. Mrs. St. Mt. No. Jan. Feb.",
    "C:\\Users\\me, /usr/bin/env, ~/notes.txt, file_name.rs, snake_case, kebab-case, __init__",
    # Hyphens, compounds and casing inside words
    "Twenty-five well-known self-driving state-of-the-art mother-in-law re-enter co-op X-ray T-shirt",
    # Accents and whitespace
    "The café served crème brûlée and jalapeño poppers at the naïve façade of the Zürich hôtel.",
    "Tabs\tand  double  spaces,\nnewlines\n\nand trailing spaces   ",
    # Words for the lexicon's stemming and fallback
    "Zelenskyy met Nguyen and Tchaikovsky's descendant in Llanfairpwllgwyngyll near Kubernetes.",
    "The blorfing snarkle glimbed over the frumious bandersnatch's quuxes and zorbled.",
    "unfriendliness antidisestablishmentarianism floccinaucinihilipilification supercalifragilistic",
    "hopping hoping running ruining kicking picking bidding biding writing written wrote typed",
    "realised realized colour color theatre theater grey gray programme program organisation",
    # Short and odd inputs
    "Wait.", "No!", "Why?", "A.", "I", "a", "OK", "...", "—", "'", "\"", "-",
    "!?", "( )", "a-", "-a", "_", "__", "'s", "s'",
]


def main(dst, espeak_data):
    from phonemizer.backend.espeak.wrapper import EspeakWrapper
    import misaki.espeak  # noqa: F401 (points espeak at espeakng-loader's library, as deployed)

    EspeakWrapper.set_data_path(espeak_data)
    from misaki import en

    with open(dst, "w") as out:
        for lang in "ab":
            british = lang == "b"
            g2p = en.G2P(trf=False, british=british, fallback=misaki.espeak.EspeakFallback(british=british), unk="")
            nlp, fallback = g2p.nlp, g2p.fallback
            for n, text in enumerate(TEXTS):
                call = {"input": text, "segments": [[text, None]], "spacy": [], "fallback": [], "foreign": []}

                def recording_nlp(t, call=call):
                    doc = nlp(t)
                    call["spacy"].append([[tk.text, tk.tag_, tk.whitespace_] for tk in doc])
                    return doc

                def recording_fallback(tk, call=call):
                    ps, rating = fallback(tk)
                    call["fallback"].append([tk.text, ps])
                    return ps, rating

                g2p.nlp, g2p.fallback = recording_nlp, recording_fallback
                stripped = text.lstrip()
                try:
                    tokens = g2p(stripped)[1] if stripped else []
                except Exception as e:  # noqa: BLE001
                    print(f"{lang} {text!r}: {type(e).__name__}: {e}", file=sys.stderr)
                    continue
                for tk in tokens:
                    tk.phonemes = tk.phonemes or ""
                call["tokens"] = [[t.text, t.tag, t.whitespace, t.phonemes] for t in tokens]
                call["phonemes"] = "".join(t.phonemes + (" " if t.whitespace else "") for t in tokens).strip()
                record = {"id": f"misaki-{lang}-{n:03}", "lang": lang, "text": text, "normalized": text,
                          "sentences": [text], "chunks": [], "calls": [call]}
                out.write(json.dumps(record, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main(os.path.join(HERE, "..", "tests", "data", "misaki_cases.jsonl"), sys.argv[1] if len(sys.argv) > 1 else ESPEAK_DATA)
