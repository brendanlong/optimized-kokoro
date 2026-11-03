"""Downloads Japanese text for the parity tests (not committed: Wikipedia is CC BY-SA).

    python3 tools/fetch_corpus.py wiki PAGES OUT.jsonl     # paragraphs of random ja.wikipedia articles
    python3 tools/fetch_corpus.py aozora OUT.jsonl         # paragraphs of public-domain Aozora Bunko works

Aozora texts are Shift_JIS with ruby (《reading》), ruby markers (｜) and editor's notes (［＃…］),
which are removed; old kana usage, iteration marks and rare kanji stay.
"""

import io
import json
import re
import sys
import time
import urllib.parse
import urllib.request
import zipfile

USER_AGENT = "optimized-kokoro-corpus/0.1 (https://github.com/brendanlong/optimized-kokoro)"
AOZORA = {
    "kokoro": "https://www.aozora.gr.jp/cards/000148/files/773_ruby_5968.zip",
    "wagahai": "https://www.aozora.gr.jp/cards/000148/files/789_ruby_5639.zip",
    "rashomon": "https://www.aozora.gr.jp/cards/000879/files/127_ruby_150.zip",
    "ginga": "https://www.aozora.gr.jp/cards/000081/files/456_ruby_145.zip",
    "takekurabe": "https://www.aozora.gr.jp/cards/000064/files/389_ruby_15296.zip",
    "maihime": "https://www.aozora.gr.jp/cards/000129/files/2078_ruby_15898.zip",
    "dogura": "https://www.aozora.gr.jp/cards/000096/files/2093_ruby_28087.zip",
}


def get(url):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read()


def wiki(pages, out):
    api = "https://ja.wikipedia.org/w/api.php?"
    seen = set()
    with open(out, "w") as f:
        fetched = 0
        while fetched < pages:
            query = urllib.parse.urlencode({
                "action": "query", "format": "json", "generator": "random", "grnnamespace": 0,
                "grnlimit": 1, "prop": "extracts", "explaintext": 1,
            })
            result = json.loads(get(api + query))
            for page in result.get("query", {}).get("pages", {}).values():
                if page["pageid"] in seen:
                    continue
                seen.add(page["pageid"])
                fetched += 1
                for i, p in enumerate(page.get("extract", "").split("\n")):
                    p = p.strip()
                    if len(p) >= 10 and not p.startswith("=="):
                        f.write(json.dumps({"id": f"jawiki-{page['pageid']}-{i:03d}", "text": p}, ensure_ascii=False) + "\n")
            time.sleep(0.2)


def aozora(out):
    with open(out, "w") as f:
        for name, url in AOZORA.items():
            try:
                data = get(url)
            except Exception as e:
                print(f"{name}: {e}", file=sys.stderr)
                continue
            z = zipfile.ZipFile(io.BytesIO(data))
            txt = next(n for n in z.namelist() if n.endswith(".txt"))
            text = z.read(txt).decode("cp932", errors="replace")
            text = re.split(r"\n-{10,}\n", text, maxsplit=2)[-1]
            text = re.split(r"\n底本：", text)[0]
            text = re.sub(r"《[^》]*》|｜|［＃[^］]*］", "", text)
            for i, p in enumerate(text.splitlines()):
                p = p.strip()
                if len(p) >= 2:
                    f.write(json.dumps({"id": f"aozora-{name}-{i:05d}", "text": p}, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    if sys.argv[1] == "wiki":
        wiki(int(sys.argv[2]), sys.argv[3])
    else:
        aozora(sys.argv[2])
