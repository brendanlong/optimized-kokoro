"""The plan for the listening page on joins between languages: each sentence of sentences.json read
three ways, as ids from the real pipeline.

    python3 plan.py SENTENCES PHONEMIZE_BINARY OUT_PLAN

- joined: as the pipeline reads it, a space (or nothing) where the languages meet;
- comma: a comma at each switch ("、" in Chinese; Japanese reads "、" as nothing, so ","), so the
  model pauses there;
- split: each language's part rendered alone, put together with SILENCE_MS of silence between.
"""

import json
import subprocess
import sys

CONDITIONS = ["joined", "comma", "split"]


def parts(segments):
    """Segments with punctuation-only ones folded into the one before."""
    out = []
    for s in segments:
        if out and not any(c.isalpha() for c in s):
            out[-1] += s
        else:
            out.append(s)
    return out


def with_commas(segments, lang):
    mark = "、" if lang == "z" else ","
    text = ""
    for i, s in enumerate(segments):
        if i and not text.rstrip().endswith((",", "、", ".", "。")) and s.strip()[:1] not in (",", ".", "。"):
            stripped = text.rstrip()
            text = stripped + mark + text[len(stripped):] + ("" if text != stripped or lang in "jz" else " ")
        text += s
    return text


def phonemize(binary, lang, texts):
    res = subprocess.run([binary, lang], input="".join(json.dumps(t) + "\n" for t in texts),
                         capture_output=True, text=True, check=True)
    out = []
    for line in res.stdout.splitlines():
        d = json.loads(line)
        assert d["error"] is None and len(d["chunks"]) == 1, d
        out.append(d["chunks"][0])
    return out


def main(sentences_path, binary, out):
    rows = []
    for n, s in enumerate(json.load(open(sentences_path, encoding="utf-8"))):
        joined = "".join(s["segments"])
        pieces = [p.strip() for p in parts(s["segments"])]
        texts = {"joined": [joined], "comma": [with_commas(s["segments"], s["lang"])], "split": pieces}
        for condition in CONDITIONS:
            chunks = phonemize(binary, s["lang"], texts[condition])
            rows.append({"sentence": n, "text": joined, "lang": s["lang"], "voice": s["voice"], "condition": condition,
                         "texts": texts[condition], "parts": [{"phonemes": c["phonemes"], "ids": c["ids"]} for c in chunks]})
    json.dump(rows, open(out, "w"), ensure_ascii=False, indent=1)
    print(f"{len(rows)} clips from {len(rows) // len(CONDITIONS)} sentences")


if __name__ == "__main__":
    main(*sys.argv[1:])
