"""Run texts through the locally built C espeak-ng 1.52 (scratch/cbuild/ttp).

    python3 oracle.py calls  espeak-dump.jsonl... > calls.jsonl
        unique (voice, text) engine calls from espeak_dump.py output, with C's clauses from ttp,
        checked against the answers phonemizer recorded
    python3 oracle.py texts VOICE < lines.txt > calls.jsonl
        one call per input line
    python3 oracle.py pairs < pairs.jsonl > calls.jsonl
        {"voice", "text"} lines; each voice runs in its own process, in input order, so the
        library state each call sees matches one Espeak instance per voice
    python3 oracle.py one VOICE TEXT
"""

import collections
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TTP = os.environ.get("TTP", os.path.join(HERE, "ttp"))
ROOT = os.path.join(HERE, "..", "..")
DATA = os.environ.get("ESPEAK_DATA", os.path.join(ROOT, "reference", ".cache", "espeak-ng-data"))
OUT = os.environ.get("ORACLE_OUT", os.path.join(ROOT, "scratch", "out"))
VOICES = {
    "en-us": "gmw/en-US", "en-gb": "gmw/en", "es": "roa/es", "fr-fr": "roa/fr", "it": "roa/it",
    "pt-br": "roa/pt-BR", "hi": "inc/hi", "ru": "zle/ru", "el": "grk/el", "ko": "ko", "ar": "sem/ar",
    "he": "sem/he",
}


def run(pairs):
    payload = b"".join(v.encode() + b"\0" + t.encode("utf-8", "surrogateescape") + b"\0" for v, t in pairs)
    proc = subprocess.run([TTP, DATA], input=payload, capture_output=True)
    sys.stderr.write(proc.stderr.decode(errors="replace"))
    if proc.returncode != 0 and len(pairs) > 1:
        print(f"ttp exited with {proc.returncode}", file=sys.stderr)
    out = proc.stdout
    records = out.split(b"\0")
    # the piece after the last terminator is unfinished (empty if the process ended cleanly)
    records = records[:-1][: len(pairs)]
    records += [None] * (len(pairs) - len(records))
    return [None if r is None else r.decode(errors="replace").split("\n")[:-1] for r in records]


def main():
    if sys.argv[1] == "calls":
        seen = {}
        for path in sys.argv[2:]:
            with open(path) as f:
                for line in f:
                    case = json.loads(line)
                    voice = VOICES[case["language"]]
                    for text, ps in case["espeak"]:
                        seen.setdefault((voice, text), ps)
        pairs = list(seen)
        clauses = run(pairs)
        bad = 0
        for (voice, text), cl in zip(pairs, clauses):
            if " ".join(cl) != seen[(voice, text)]:
                bad += 1
                if bad < 10:
                    print("oracle differs:", voice, repr(text), cl, repr(seen[(voice, text)]), file=sys.stderr)
            print(json.dumps({"voice": voice, "text": text, "clauses": cl}, ensure_ascii=False))
        print(f"{len(pairs)} calls, {bad} differ from the recorded answers", file=sys.stderr)
    elif sys.argv[1] == "texts":
        voice = sys.argv[2]
        pairs = [(voice, line.rstrip("\n")) for line in sys.stdin]
        for (v, t), cl in zip(pairs, run(pairs)):
            print(json.dumps({"voice": v, "text": t, "clauses": cl}, ensure_ascii=False))
    elif sys.argv[1] == "pairs":
        by_voice = collections.defaultdict(list)
        order = []
        for line in sys.stdin:
            r = json.loads(line)
            by_voice[r["voice"]].append(r["text"])
            order.append(r["voice"])
        # if C crashes, the call gets a {"reset": true} record and C restarts with fresh state
        results = {}
        for voice, texts in by_voice.items():
            out = []
            while len(out) < len(texts):
                rest = texts[len(out):]
                got = run([(voice, t) for t in rest])
                for cl in got:
                    if cl is None:
                        out.append(None)
                        break
                    out.append(cl)
            results[voice] = iter(out)
            by_voice[voice] = iter(texts)
        for voice in order:
            t = next(by_voice[voice])
            cl = next(results[voice])
            if cl is None:
                print(f"C crashed: {voice} {t!r}", file=sys.stderr)
                print(json.dumps({"voice": voice, "text": t, "reset": True}, ensure_ascii=False))
                continue
            print(json.dumps({"voice": voice, "text": t, "clauses": cl}, ensure_ascii=False))
    elif sys.argv[1] == "fresh":
        # each call in a new process; calls where C crashes are reported and skipped
        for line in sys.stdin:
            r = json.loads(line)
            cl = run([(r["voice"], r["text"])])[0]
            if cl is None:
                print(f"C crashed: {r['voice']} {r['text']!r}", file=sys.stderr)
                continue
            print(json.dumps({"voice": r["voice"], "text": r["text"], "clauses": cl}, ensure_ascii=False))
    elif sys.argv[1] == "one":
        for cl in run([(sys.argv[2], " ".join(sys.argv[3:]))])[0]:
            print(cl)


if __name__ == "__main__":
    main()
