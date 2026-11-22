"""Ask a model which name spellings an English reader could meet outside the source's world, so they stay out of the dictionary."""

import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

PROMPT = """Below are spellings of names from {world}. Our text-to-speech reader will say these words with their \
{world} pronunciation whenever they appear in any English text: news, novels, web pages. That's only safe if a reader \
would almost never meet the spelling outside {world}.

Flag every spelling that is also any of these in English text: an English word (including archaic, technical, or \
plural forms), a real person's first name or surname that's at all common, a real place (city, town, river, region, \
country), a brand or product, a word commonly borrowed from another language, or an abbreviation. When in doubt, flag it.

Reply with JSON only: an object mapping each flagged spelling to {{"reason": short reason, "same": true if that other use is normally pronounced about the same as the {world} name, false if noticeably differently}}. Omit spellings you don't flag.

Spellings:
"""


def ask(model, world, words):
    with tempfile.TemporaryDirectory() as cwd:
        res = subprocess.run([os.environ.get("CLAUDE_BIN", "claude"), "-p", "--model", model, "--tools", "",
                              "--system-prompt", "You are a careful lexicographer. Answer with JSON only."],
                             input=PROMPT.format(world=world) + "\n".join(words), capture_output=True, text=True,
                             cwd=cwd, timeout=900, check=True)
    t = res.stdout
    return json.loads(t[t.index("{"):t.rindex("}") + 1]) if "{" in t else {}


def main(words_path, world, model, out):
    words = json.load(open(words_path))
    batches = [words[i:i + 100] for i in range(0, len(words), 100)]
    with ThreadPoolExecutor(len(batches)) as pool:
        flagged = {k: v for d in pool.map(lambda b: ask(model, world, b), batches) for k, v in d.items() if k in words}
    json.dump(dict(sorted(flagged.items())), open(out, "w"), ensure_ascii=False, indent=1)
    print(f"{len(flagged)} of {len(words)} flagged")


if __name__ == "__main__":
    main(*sys.argv[1:])
