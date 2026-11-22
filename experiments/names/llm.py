"""Ask a Claude model for each name's pronunciation, in misaki phonemes or in IPA, through the Claude Code CLI."""

import json
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor

import yaml

from phon import clean_misaki, ipa_to_misaki

SYSTEM = "You are an expert in phonetics and in J.R.R. Tolkien's languages. Answer with JSON only."

TASK = """These are names from Tolkien's legendarium. For each, give the pronunciation Tolkien intended \
(following his rules in Appendix E of The Lord of the Rings), as an American English speaker would say it \
when doing it properly: no rolled r, and no sounds English lacks except where an English approximation is impossible."""

MISAKI = """Write each pronunciation in this phoneme alphabet (it's what our text-to-speech model reads):
vowels: ɑ (father) æ (cat) ɛ (bed) ɪ (sit) i (see) ʊ (book) u (food) ʌ (cup) ɔ (law) ə (about) ɜɹ (bird) ᵊ (the reduced vowel in "button" bʌtᵊn)
diphthongs, each one character: A (day) I (my) O (go) W (now) Y (boy)
consonants: b d f h j (yes) k l m n ŋ (sing) p ɹ (red) s ʃ (she) t θ (thin) ð (this) v w z ʒ (vision) ʤ (jam) ʧ (church) ɡ (go) T (the flap in "butter")
stress: put ˈ (primary) or ˌ (secondary) immediately before the stressed vowel, e.g. "Gondor" ɡˈɑndɔɹ, "Arwen" ˈɑɹwɛn.
Use only these symbols and spaces."""

IPA = "Write each pronunciation in IPA, with ˈ and ˌ stress marks, without slashes or brackets."

FORMAT = 'Reply with one JSON object mapping each name exactly as given to its pronunciation string.\n\nNames:\n'


def ask(model, fmt, names):
    prompt = TASK + "\n\n" + (MISAKI if fmt == "misaki" else IPA) + "\n\n" + FORMAT + "\n".join(names)
    with tempfile.TemporaryDirectory() as cwd:
        res = subprocess.run(
            [os.environ.get("CLAUDE_BIN", "claude"), "-p", "--model", model, "--tools", "", "--system-prompt", SYSTEM],
            input=prompt, capture_output=True, text=True, cwd=cwd, timeout=600, check=True)
    text = res.stdout
    return json.loads(text[text.index("{"):text.rindex("}") + 1]), text


def main(key, config, model_name, fmt, out):
    cfg = yaml.safe_load(open(config))
    model = cfg["models"][model_name]
    names = [json.loads(l)["name"] for l in open(key)]
    n = cfg["llm_batch"]
    batches = [names[i:i + n] for i in range(0, len(names), n)]
    with ThreadPoolExecutor(len(batches)) as pool:
        replies = list(pool.map(lambda b: ask(model, fmt, b), batches))
    answers = {k.strip(): v for reply, _ in replies for k, v in reply.items()}
    with open(out, "w") as f:
        for name in names:
            raw = answers.get(name)
            if raw is None:
                row = {"name": name, "phonemes": None, "raw": None, "invalid": []}
            else:
                phonemes, invalid = clean_misaki(raw if fmt == "misaki" else ipa_to_misaki(raw))
                row = {"name": name, "phonemes": phonemes, "raw": raw, "invalid": invalid}
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
    with open(out + ".log", "w") as f:
        f.write("\n\n".join(text for _, text in replies))


if __name__ == "__main__":
    main(*sys.argv[1:])
