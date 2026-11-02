"""Downloads a test corpus into corpus/ (not committed: Wikipedia text is CC BY-SA).

    python3 fetch_corpus.py [--wiki-pages N]

Writes corpus/gutenberg.jsonl (public-domain English books, one record per paragraph) and
corpus/wiki-<lang>.jsonl (paragraphs from random Wikipedia articles in each pipeline language).
"""

import argparse
import json
import os
import re
import time
import urllib.parse
import urllib.request

GUTENBERG = {
    "time-machine": 35,
    "pride-and-prejudice": 1342,
    "sherlock-holmes": 1661,
    "frankenstein": 84,
    "moby-dick": 2701,
}
# Kokoro pipeline code -> Wikipedia edition
WIKIS = {"a": "en", "e": "es", "f": "fr", "h": "hi", "i": "it", "j": "ja", "p": "pt", "z": "zh"}
USER_AGENT = "optimized-kokoro-corpus/0.1 (https://github.com/brendanlong/optimized-kokoro)"


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode("utf-8")


def gutenberg(out):
    with open(out, "w") as f:
        for name, number in GUTENBERG.items():
            text = get(f"https://www.gutenberg.org/cache/epub/{number}/pg{number}.txt")
            start = re.search(r"\*\*\* START OF .*?\*\*\*", text)
            end = re.search(r"\*\*\* END OF .*?\*\*\*", text)
            body = text[start.end() if start else 0 : end.start() if end else len(text)]
            for index, paragraph in enumerate(re.split(r"\r?\n\s*\r?\n", body)):
                paragraph = " ".join(paragraph.split())
                if 40 <= len(paragraph) <= 2000:
                    record = {"id": f"gutenberg-{name}-{index:05d}", "lang": "a", "text": paragraph}
                    f.write(json.dumps(record, ensure_ascii=False) + "\n")


def wiki(lang_code, edition, pages, out):
    # Whole-article extracts come one page per request.
    api = f"https://{edition}.wikipedia.org/w/api.php?"
    with open(out, "w") as f:
        fetched = 0
        while fetched < pages:
            query = urllib.parse.urlencode({
                "action": "query", "format": "json", "generator": "random", "grnnamespace": 0,
                "grnlimit": 1, "prop": "extracts", "explaintext": 1,
            })
            result = json.loads(get(api + query))
            for page in result.get("query", {}).get("pages", {}).values():
                fetched += 1
                for index, paragraph in enumerate(page.get("extract", "").split("\n")):
                    paragraph = paragraph.strip()
                    if 40 <= len(paragraph) <= 2000 and not paragraph.startswith("=="):
                        record = {"id": f"wiki-{edition}-{page['pageid']}-{index:03d}", "lang": lang_code, "text": paragraph}
                        f.write(json.dumps(record, ensure_ascii=False) + "\n")
            time.sleep(0.5)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--wiki-pages", type=int, default=200)
    args = parser.parse_args()
    os.makedirs("corpus", exist_ok=True)
    gutenberg("corpus/gutenberg.jsonl")
    for lang_code, edition in WIKIS.items():
        wiki(lang_code, edition, args.wiki_pages, f"corpus/wiki-{edition}.jsonl")


if __name__ == "__main__":
    main()
