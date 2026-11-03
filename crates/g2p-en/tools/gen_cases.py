"""Writes tests/data/misaki_cases.jsonl: misaki's English G2P run directly (no Kokoro-FastAPI normalizer,
which rewrites most numbers and symbols first) on texts that exercise its rules, for American and British.

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
    # Integers, separators, signs and suffixes
    "0 1 7 10 11 19 20 21 99 100 101 110 999 1000 1001 1010 1100 9999 10000 12345 100000 1000000",
    "1,000 10,000 1,234,567 1,000,000,000 12,34 1,2,3 123456789012 98765432109876543210",
    "-1 -42 -1,000 -3.5 -0.25 - 5 --5 +5 +1 800 555 0199",
    "1st 2nd 3rd 4th 11th 12th 13th 21st 22nd 23rd 100th 101st 111th 1,000th 1000000th 0th",
    "the 1990s, the 90s, the '90s, 1800s, 5s and 6s, 1990's, 7's, 10ed, 3ing, 4'd",
    "In 1066, 1492, 1776, 1900, 1901, 1906, 1999, 2000, 2001, 2005, 2010, 2019, 2024, 2100, 0999 and 0042.",
    "Years: 1000 1100 1200 1010 1099 3000 3001 9010 9100 2900 1066AD 1066 BC",
    # Decimals, versions and addresses
    "3.14159 0.5 .5 .25 1.0 1.10 2.50 10.05 100.001 0.000001 123456.789 3.0 5. 7.000",
    "1.2.3 v2.0.1 192.168.0.1 Section 4.2.1 says 10.1.0 replaced 9.9.",
    "pi is 3.14159265358979323846 and e is 2.718281828459045.",
    # Currency
    "$5 $5.50 $0.99 $1 $1.00 $1.01 $.50 $100.123 $1,000,000 $5 million $3.2 billion $0 $0.01 $10.5",
    "£3.50 £1 £0.50 £100 £2.5 million €1.05 €1 €20 €0.01 $ 5 5$ USD 5",
    "It cost $1,234.56, or about £900, or €1.05 million; the $20 bill and $1 coin.",
    "He paid $12 for 3 items, then $7.25 each for the other 10.",
    # Numbers inside words and next to letters
    "x2y peer2peer B2B 4x4 3D 2nd-hand 24/7 9-to-5 1-800-FLOWERS COVID-19 MP3 A4 F-16 iPhone15",
    "Call (555) 123-4567 or 555.123.4567, ext. 42; room 4B; flight AA100; gate B12.",
    "At 3:45 p.m., 9:30am, 10 PM and 12:00 noon; the score was 21-14 and the ratio 3:2.",
    "3/4 1/2 cup, 98.6°F, 45%, 100 %, 50/50, 2+2=4, 3x5, 10KB, 5GB, 100Mbps, 5km, 60mph",
    # Number flags and other link features
    "[1984](#a#) [150](#a#) [1001](#&#) [1,001](#n#) [2,500](#an#) [365](#&n#) [100](#a&#)",
    "[read](/ɹˈɛd/) the [Kokoro](/kˈOkəɹO/) [two words](/tˈu wˈɜɹdz/) [x](/ /) [y](//)",
    "[record](-1) a [record](1) [record](2) [present](0.5) [present](-0.5) [present](+0.5) [the](-2) [ok](3)",
    "[New York](-1) [hello world](2) [it's](0) [don't](-1) [9](1) [1999](0) [$5](-1)",
    "An [unclosed](link and [empty]() and [bad](feature) and [nested [x](1)](2).",
    # Capitalization and stress
    "THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG. AM I IN? TO BE OR NOT TO BE.",
    "NASA's FBI's CEOs ABCs GPUs URLs PhDs DVDs iPhones eBay McDonald's MacBook YouTube JavaScript",
    "Running RUNNING Runs RUNS Walked WALKED Cities CITIES Studies STUDYING Tried TRIED",
    "ELIZABETH Elizabeth elizabeth, LONDON London, OK Ok ok, TV tv, USA usa Usa",
    # Function words and context
    "I am. Am I? I'm am. The AM radio at 9 AM. An apple, an hour, an AN-94, a A, A a.",
    "to the end, to an end, to go, to eat, going to, TO BE, To me. In it, in the, IN IT, Come in!",
    "the apple, the pear, the hour, the one, the eulogy, THE END, The end.",
    "I used to go. I used it. used cars, He is used to it. USED TO. They used two.",
    "Stand by. He went by. By and by. BY the way, close by, passed by quickly.",
    "England vs. France, cats vs dogs, A VS B, Ali vs. Frazier.",
    # Symbols, punctuation, quotes and dashes
    "Tom & Jerry, 5 + 3, @home, 50% off, #1, C++, AT&T, R&D, Q&A, x@y.com",
    "\"Hello,\" she said. 'Goodbye,' he replied — then left… ‘Quoted’ “double” «guillemets»",
    "Wait - what? A–B, pages 10–20, well—maybe; (parenthetical) [brackets] {braces} <angles>",
    "Hmm... well, uh, I don't know?! Really?!? Yes!!! No... Okay.",
    "rock 'n' roll, ’tis, 'twas, o'clock, y'all, ma'am, fo'c'sle, dogs' bones, James' book, Chris's",
    "can't won't shouldn't they're we'd you'll I've it's its let's ain't gonna wanna",
    "e.g. i.e. U.S. U.S.A. U.K. a.m. p.m. Ph.D. etc. vs. Dr. Mr. Mrs. St. Mt. No. Jan. Feb.",
    "Visit https://www.example.org/path?q=1 or www.lionreader.com or mail jane.doe@example.com.",
    "C:\\Users\\me, /usr/bin/env, ~/notes.txt, file_name.rs, snake_case, kebab-case, __init__",
    # Hyphens, compounds and casing inside words
    "Twenty-five well-known self-driving state-of-the-art mother-in-law re-enter co-op X-ray T-shirt",
    "CamelCase camelCase HTMLParser XMLHttpRequest PostgreSQL TensorFlow OpenAI GPT-4o iOS macOS",
    # Unicode: accents, digits, compatibility forms
    "The café served crème brûlée and jalapeño poppers at the naïve façade of the Zürich hôtel.",
    "Arabic-Indic ٣ and ٤٥, superscript x² and 10³, fullwidth １２３ and ＡＢＣ, ligature ﬁne and ﬂow.",
    "½ cup, ¾ mile, Ⅻ chapters, ① first, ㈠ one, ß straße, İstanbul, ǅemal, Σίσυφος",
    "Tabs\tand  double  spaces,\nnewlines\n\nand trailing spaces   ",
    # Words for the lexicon's stemming and fallback
    "Zelenskyy met Nguyen and Tchaikovsky's descendant in Llanfairpwllgwyngyll near Kubernetes.",
    "The blorfing snarkle glimbed over the frumious bandersnatch's quuxes and zorbled.",
    "unfriendliness antidisestablishmentarianism floccinaucinihilipilification supercalifragilistic",
    "hopping hoping running ruining kicking picking bidding biding writing written wrote typed",
    "realised realized colour color theatre theater grey gray programme program organisation",
    # Short and odd inputs
    "Wait.", "No!", "Why?", "A.", "I", "a", "OK", "...", "—", "'", "\"", "-", "5", "1st", "$", "$5", "%",
    "!?", "( )", "a-", "-a", "_", "__", "'s", "s'", "2", "x2", "2x", "0", "00", "007", "0.0", "-0",
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
