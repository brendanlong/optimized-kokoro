"""Runs misaki's two espeak wrappers over test cases and records what C espeak-ng saw and said.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I espeak_dump.py cases.jsonl out.jsonl
    .cache/kfa/.venv/bin/python -I espeak_dump.py --build dump-or-corpus.jsonl... > cases.jsonl

Input lines: {"wrapper": "fallback"|"g2p", "language": espeak language, "text": str}
    fallback   misaki.espeak.EspeakFallback (language en-us or en-gb), given one English token
    g2p        misaki.espeak.EspeakG2P(language), given a whole segment
Output lines add:
    espeak     [[text, phonemes]]: each string phonemizer passed to espeak_TextToPhonemes (clauses
               joined with a space, ties as U+0361) and what came back, before phonemizer's
               post-processing
    phonemes   the wrapper's output (null when EspeakFallback returns None)

--build turns existing cases, reference dumps (dump.py output) and raw corpus files into cases: every fallback word
and foreign segment the dumps recorded, the native segments of es/fr/hi/it/pt-br calls, sentences and
words from the raw corpus, and runs of each foreign script found in it.
"""

import collections
import json
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".cache", "kfa"))

from phonemizer.backend.espeak.wrapper import EspeakWrapper  # noqa: E402
from misaki import espeak  # noqa: E402

EspeakWrapper.set_data_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".cache", "espeak-ng-data"))

from loguru import logger  # noqa: E402

logger.remove()

NATIVE = {"e": "es", "f": "fr-fr", "h": "hi", "i": "it", "p": "pt-br"}


def recording(wrapper):
    calls = []
    inner = wrapper.backend._espeak
    original = inner.text_to_phonemes

    def text_to_phonemes(text, tie=False):
        out = original(text, tie)
        calls.append([text, out])
        return out

    inner.text_to_phonemes = text_to_phonemes
    return calls


class Wrappers:
    def __init__(self):
        self.cache = {}

    def get(self, wrapper, language):
        key = (wrapper, language)
        if key not in self.cache:
            if wrapper == "fallback":
                assert language in ("en-us", "en-gb"), language
                w = espeak.EspeakFallback(british=language == "en-gb")
            else:
                w = espeak.EspeakG2P(language=language)
            self.cache[key] = (w, recording(w))
        return self.cache[key]

    def run(self, case):
        w, calls = self.get(case["wrapper"], case["language"])
        calls.clear()
        if case["wrapper"] == "fallback":
            token = collections.namedtuple("Token", "text")(case["text"])
            phonemes = w(token)[0]
        else:
            phonemes = w(case["text"])[0]
        return dict(case, espeak=list(calls), phonemes=phonemes)


def dump(src, dst):
    wrappers = Wrappers()
    with open(src) as fin, open(dst, "w") as fout:
        for line in fin:
            if line.strip():
                fout.write(json.dumps(wrappers.run(json.loads(line)), ensure_ascii=False) + "\n")


def build(paths):
    from kokoro.foreign import RUN_REGEX, SCRIPT_LANGUAGES, HEBREW_POINTS

    seen = set()

    def emit(wrapper, language, text, explicit=False):
        if (text or explicit) and (wrapper, language, text) not in seen:
            seen.add((wrapper, language, text))
            print(json.dumps({"wrapper": wrapper, "language": language, "text": text}, ensure_ascii=False))

    for path in paths:
        with open(path) as f:
            for line in f:
                record = json.loads(line)
                if "wrapper" in record:
                    emit(record["wrapper"], record["language"], record["text"], explicit=True)
                    continue
                if "calls" in record:
                    for call in record["calls"]:
                        for word, _ in call.get("fallback", []):
                            for english in ("en-us", "en-gb"):
                                emit("fallback", english, word)
                        for segment, language, _ in call.get("foreign", []):
                            if language in SCRIPT_LANGUAGES.values() and language != "en":
                                if language == "he":
                                    segment = HEBREW_POINTS.sub("", segment)
                                emit("g2p", language, segment)
                        if record["lang"] in NATIVE:
                            for segment, language in call["segments"]:
                                if language is None:
                                    emit("g2p", NATIVE[record["lang"]], segment)
                    continue
                text = record["text"]
                if record["lang"] in NATIVE:
                    emit("g2p", NATIVE[record["lang"]], text)
                    for sentence in re.split(r"(?<=[.!?;:])\s+", text):
                        emit("g2p", NATIVE[record["lang"]], sentence)
                elif record["lang"] in "ab":
                    for word in text.split():
                        for english in ("en-us", "en-gb"):
                            emit("fallback", english, word)
                            emit("fallback", english, word.strip(".,;:!?\"'()[]“”‘’"))
                for m in RUN_REGEX.finditer(text):
                    language = SCRIPT_LANGUAGES.get(m.lastgroup)
                    if language not in (None, "en"):
                        segment = m.group()
                        if language == "he":
                            segment = HEBREW_POINTS.sub("", segment)
                        emit("g2p", language, segment)


if __name__ == "__main__":
    if sys.argv[1] == "--build":
        build(sys.argv[2:])
    else:
        dump(sys.argv[1], sys.argv[2])
