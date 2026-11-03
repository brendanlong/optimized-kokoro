"""Run texts through the locally built C espeak-ng 1.52 (scratch/cbuild/ttp).

    python3 oracle.py calls  espeak-dump.jsonl... > calls.jsonl
        unique (voice, text) engine calls from espeak_dump.py output, with C's clauses from ttp,
        checked against the answers phonemizer recorded
    python3 oracle.py texts VOICE < lines.txt > calls.jsonl
        one call per input line
    python3 oracle.py pairs [--ub] < pairs.jsonl > calls.jsonl
        {"voice", "text"} lines; each voice runs in its own process, in input order, so the
        library state each call sees matches one Espeak instance per voice. With --ub, the same
        sequences also run under ASan (ttp-asan) and valgrind's memcheck (ttp-memcheck; `make
        sanitized`), and calls where C reads out of bounds or reads uninitialised memory get
        "ub": [reports]. ASan stops at its first report, so after one the ASan run restarts with
        fresh state at the next call. Their clauses depend on whatever memory C read, so they are not C's answer.
    python3 oracle.py one VOICE TEXT
"""

import collections
import concurrent.futures
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TTP = os.environ.get("TTP", os.path.join(HERE, "ttp"))
ROOT = os.path.join(HERE, "..", "..")
DATA = os.environ.get("ESPEAK_DATA", os.path.join(ROOT, "reference", ".cache", "espeak-ng-data"))
OUT = os.environ.get("ORACLE_OUT", os.path.join(ROOT, "scratch", "out"))
TTP_ASAN = os.path.join(HERE, "ttp-asan")
TTP_MEMCHECK = os.path.join(HERE, "ttp-memcheck")
VALGRIND = os.environ.get("VALGRIND", "valgrind")
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


def payload(voice, texts):
    return b"".join(voice.encode() + b"\0" + t.encode("utf-8", "surrogateescape") + b"\0" for t in texts)


_symbols = {}


def symbolize(offset):
    """`function (file:line)` for an offset in ttp-asan."""
    if offset not in _symbols:
        out = subprocess.run(["addr2line", "-f", "-i", "-e", TTP_ASAN, offset], capture_output=True, text=True).stdout.split("\n")
        func, where = (out + ["?", "?"])[:2]
        _symbols[offset] = f"{func} ({os.path.basename(where.split(' ')[0])})"
    return _symbols[offset]


def sanitized_run(tool, voice, texts):
    """Reports per text from one sanitized process (None where clean), and how many records it
    finished."""
    env = dict(os.environ, TTP_MARK_RECORDS="1")
    if tool == "asan":
        cmd = [TTP_ASAN, DATA]
        # Stop at the first report: after one, C can loop reporting for a long time. The caller
        # restarts after the reported call. Symbolizing is slow; the first frame in ttp-asan is
        # symbolized once below.
        env["ASAN_OPTIONS"] = "detect_leaks=0:symbolize=0"
    else:
        cmd = [VALGRIND, "--error-limit=no", "--log-fd=2", TTP_MEMCHECK, DATA]
    proc = subprocess.run(cmd, input=payload(voice, texts), capture_output=True, env=env)
    finished = proc.stdout.count(b"\0")
    reports = [None] * len(texts)
    sections = re.split(r"\n@@ttp-record (\d+)\n", proc.stderr.decode(errors="replace"))
    errors_before = 0
    for ix in range(1, len(sections) - 1, 2):
        record, body = int(sections[ix]), sections[ix + 1]
        if record >= len(texts):
            break
        if tool == "asan":
            found = []
            for report in body.split("ERROR: AddressSanitizer: ")[1:]:
                kind = report.split()[0]
                frame = re.search(r"\(\S*ttp-asan\+(0x[0-9a-f]+)\)", report)
                found.append(f"asan: {kind} at {symbolize(frame.group(1)) if frame else '?'}")
            if found:
                reports[record] = sorted(set(found))
        else:
            m = re.search(r"@@ttp-errors (\d+)", body)
            errors = int(m.group(1)) if m else errors_before
            if errors > errors_before or (record >= finished and record < len(texts)):
                sites = re.findall(r"^==\d+== (\S.*)\n==\d+==    at 0x[0-9A-F]+: (\S+ \(\S+\))", body, re.M)
                reports[record] = sorted({f"memcheck: {what} at {where}" for what, where in sites}) or [
                    f"memcheck: {errors - errors_before} errors (repeated)"
                ]
            errors_before = errors
    if finished < len(texts) and not reports[finished]:
        reports[finished] = [f"{tool}: crashed"]
    return reports, finished


def sanitized(tool, voice, texts):
    reports = []
    while len(reports) < len(texts):
        got, finished = sanitized_run(tool, voice, texts[len(reports):])
        reports += got[: finished + 1]
    return reports[: len(texts)]


def ub_reports(voice, texts, results):
    """Runs each crash-free stretch of a voice's calls under both sanitizers."""
    out = [None] * len(texts)
    start = 0
    for end in [i for i, r in enumerate(results) if r is None] + [len(texts)]:
        if end > start:
            for tool in ("asan", "memcheck"):
                for i, rep in enumerate(sanitized(tool, voice, texts[start:end])):
                    if rep:
                        out[start + i] = (out[start + i] or []) + rep
        start = end + 1
    return out


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
        check_ub = "--ub" in sys.argv[2:]
        by_voice = collections.defaultdict(list)
        order = []
        for line in sys.stdin:
            r = json.loads(line)
            by_voice[r["voice"]].append(r["text"])
            order.append(r["voice"])
        # if C crashes, the call gets a {"reset": true} record and C restarts with fresh state
        def one_voice(voice, texts):
            out = []
            while len(out) < len(texts):
                rest = texts[len(out):]
                got = run([(voice, t) for t in rest])
                for cl in got:
                    if cl is None:
                        out.append(None)
                        break
                    out.append(cl)
            return out, (ub_reports(voice, texts, out) if check_ub else [None] * len(texts))

        with concurrent.futures.ThreadPoolExecutor(os.cpu_count()) as pool:
            futures = {voice: pool.submit(one_voice, voice, texts) for voice, texts in by_voice.items()}
            results = {}
            for voice, f in futures.items():
                out, ub = f.result()
                results[voice] = iter(zip(out, ub))
                by_voice[voice] = iter(by_voice[voice])
        for voice in order:
            t = next(by_voice[voice])
            cl, ub = next(results[voice])
            if cl is None:
                print(f"C crashed: {voice} {t!r}", file=sys.stderr)
                print(json.dumps({"voice": voice, "text": t, "reset": True}, ensure_ascii=False))
                continue
            rec = {"voice": voice, "text": t, "clauses": cl}
            if ub:
                rec["ub"] = ub
            print(json.dumps(rec, ensure_ascii=False))
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
