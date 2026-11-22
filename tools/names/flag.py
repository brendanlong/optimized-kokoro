"""Ask a model which of a source's unreviewed words an English reader could meet outside the source's world.

    CLAUDE_BIN=claude python3 tools/names/flag.py SOURCE

Reads review/SOURCE.todo.json (written by build.py), adds the words to review/SOURCE.json's "reviewed" list and
the flagged ones with reasons to its "flagged" map. A person then copies the flags that should be left out into
exclude.tsv: flagged words the source's world gave its name to (companies named after LotR, given names taken
from Tolkien) can stay.
"""

import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import build

MODEL = "claude-opus-5-5"
PROMPT = """Below are spellings of names from {world}. Our text-to-speech reader will say these words with their \
{world} pronunciation whenever they appear in any English text: news, novels, web pages. That's only safe if a reader \
would almost never meet the spelling outside {world}.

Flag every spelling that is also any of these in English text: an English word (including archaic, technical, or \
plural forms), a real person's first name or surname that's at all common, a real place (city, town, river, region, \
country), a brand or product, a word commonly borrowed from another language, or an abbreviation. When in doubt, flag it.

Reply with JSON only: an object mapping each flagged spelling to a short reason. Omit spellings you don't flag.

Spellings:
"""


def ask(world, words):
    with tempfile.TemporaryDirectory() as cwd:
        res = subprocess.run([os.environ.get("CLAUDE_BIN", "claude"), "-p", "--model", MODEL, "--tools", "",
                              "--system-prompt", "You are a careful lexicographer. Answer with JSON only."],
                             input=PROMPT.format(world=world) + "\n".join(words), capture_output=True, text=True,
                             cwd=cwd, timeout=900, check=True)
    t = res.stdout
    return json.loads(t[t.index("{"):t.rindex("}") + 1])


def main(source):
    todo_path = build.HERE / "review" / f"{source}.todo.json"
    review_path = build.HERE / "review" / f"{source}.json"
    words = json.load(open(todo_path))
    review = json.load(open(review_path)) if review_path.exists() else {"reviewed": [], "flagged": {}}
    world = build.SOURCES[source].WORLD
    batches = [words[i:i + 100] for i in range(0, len(words), 100)]
    with ThreadPoolExecutor(len(batches)) as pool:
        for flags in pool.map(lambda b: ask(world, b), batches):
            review["flagged"].update({k: v for k, v in flags.items() if k in words})
    review["reviewed"] = sorted(set(review["reviewed"]) | set(words))
    review["flagged"] = dict(sorted(review["flagged"].items()))
    json.dump(review, open(review_path, "w"), ensure_ascii=False, indent=1)
    todo_path.unlink()
    print(f"{len(words)} checked; {len(review['flagged'])} flagged in all, in {review_path.relative_to(build.ROOT)}")


if __name__ == "__main__":
    main(*sys.argv[1:])
