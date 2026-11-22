"""Every Tolkien Gateway article with an {{IPA|...}} pronunciation, as JSON lines of {name, ipa, page}."""

import json
import re
import sys
import urllib.parse
import urllib.request

API = "https://tolkiengateway.net/w/api.php"
UA = {"User-Agent": "kokoro-names-experiment/0.1 (pronunciation research)"}


def get(params):
    url = API + "?" + urllib.parse.urlencode({**params, "format": "json", "formatversion": 2})
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA)) as r:
        return json.load(r)


def pages_with_ipa():
    params = {"action": "query", "list": "embeddedin", "eititle": "Template:IPA", "eilimit": 500, "einamespace": 0}
    while True:
        d = get(params)
        yield from (p["title"] for p in d["query"]["embeddedin"])
        if "continue" not in d:
            return
        params.update(d["continue"])


def wikitexts(titles):
    for i in range(0, len(titles), 50):
        d = get({"action": "query", "prop": "revisions", "rvprop": "content", "rvslots": "main",
                 "titles": "|".join(titles[i:i + 50])})
        for p in d["query"]["pages"]:
            if "revisions" in p:
                yield p["title"], p["revisions"][0]["slots"]["main"]["content"]


PRONUN = re.compile(r"\|\s*pronun\s*=\s*(.*)")
IPA = re.compile(r"\{\{IPA\|\[?([^\]}]+?)\]?\}\}")
NAME = re.compile(r"\|\s*name\s*=\s*(.*)")


def main():
    titles = sorted(set(pages_with_ipa()))
    seen = set()
    for title, text in wikitexts(titles):
        if "(disambiguation)" in title:
            continue
        m = PRONUN.search(text)
        ipas = IPA.findall(m.group(1)) if m else IPA.findall(text[:3000])
        if not ipas:
            continue
        name = re.sub(r"\s*\(.*\)$", "", title)
        if name in seen:
            continue
        seen.add(name)
        print(json.dumps({"name": name, "ipa": ipas[0].strip(), "page": title}, ensure_ascii=False))


if __name__ == "__main__":
    sys.exit(main())
