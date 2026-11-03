"""Runs the reference text pipeline over a corpus and records every stage's output.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I dump.py corpus.jsonl out.jsonl

Input lines: {"id": str, "lang": "a"|"b"|"e"|"f"|"h"|"i"|"j"|"p"|"z", "text": str}
Output lines add:
    normalized   text after join_lines and the language normalizer, before sentence splitting
    sentences    the sentences smart_split phonemizes
    chunks       [{"text", "phonemes"}] exactly as smart_split yields them
    calls        one entry per G2P call (phonemize(text, lang)):
        input        text given to G2P
        segments     [[segment, language or null]] from ForeignScripts.split
        phonemes     the call's output
        tokens       English only: [[text, tag, whitespace, phonemes]] after misaki
        spacy        English only: one list per misaki call, [[text, tag, whitespace]] from spaCy
        fallback     English only: [[word, phonemes]] answered by the espeak fallback
        foreign      [[segment, language, phonemes]] read by another language's G2P
        foreign_spacy, foreign_fallback
                     the same as spacy and fallback, for Latin runs another pipeline reads with English G2P
"""

import asyncio
import json
import os
import sys

# Server settings come from the environment and ./.env. Run with neither, so the dump records the defaults
# the deployed image runs with (it sets none that affect text).
HERE = os.path.dirname(os.path.abspath(__file__))
if os.environ.get("KOKORO_REFERENCE_CLEAN") != "1":
    env = {k: os.environ[k] for k in ("PATH", "HOME", "TMPDIR") if k in os.environ}
    args = [os.path.abspath(a) for a in sys.argv]
    os.chdir(HERE)
    os.execve(sys.executable, [sys.executable, "-I", *args], {**env, "KOKORO_REFERENCE_CLEAN": "1"})

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".cache", "kfa"))

from phonemizer.backend.espeak.wrapper import EspeakWrapper  # noqa: E402

# misaki points espeak at the data bundled with espeakng-loader when imported (as in the deployed image).
# espeak-ng keeps that path in a fixed-size buffer this venv's path overflows; setup.sh copies the data
# somewhere shorter.
import misaki.espeak  # noqa: E402,F401

EspeakWrapper.set_data_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".cache", "espeak-ng-data"))

from loguru import logger  # noqa: E402

logger.remove()

from misaki import en  # noqa: E402

from api.src.inference import g2p  # noqa: E402
from api.src.services.text_processing import text_processor as tp  # noqa: E402
from api.src.structures.schemas import NormalizationOptions  # noqa: E402

current = None


def record(key, value):
    if current is not None:
        current.setdefault(key, []).append(value)


def instrument(pipeline):
    if getattr(pipeline, "_instrumented", False):
        return
    pipeline._instrumented = True
    foreign = pipeline.foreign
    original_split, original_phonemize = foreign.split, foreign.phonemize

    def split(text):
        segments = original_split(text)
        record("segments", [[s, lang] for s, lang in segments])
        return segments

    def phonemize(segment, language):
        ps = original_phonemize(segment, language)
        record("foreign", [segment, language, ps])
        return ps

    original_get_g2p = foreign.get_g2p

    def get_g2p(language):
        g = original_get_g2p(language)
        if language == "en" and isinstance(g, en.G2P):
            instrument_english(g, "foreign_")
        return g

    foreign.split, foreign.phonemize, foreign.get_g2p = split, phonemize, get_g2p
    if pipeline.lang_code in "ab":
        instrument_english(pipeline.g2p, "")


def instrument_english(g, prefix):
    if getattr(g, "_instrumented", False):
        return
    g._instrumented = True
    nlp = g.nlp

    def recording_nlp(text):
        doc = nlp(text)
        record(prefix + "spacy", [[t.text, t.tag_, t.whitespace_] for t in doc])
        return doc

    g.nlp = recording_nlp
    fallback = g.fallback
    if fallback is not None:
        def recording_fallback(token):
            ps, rating = fallback(token)
            record(prefix + "fallback", [token.text, ps])
            return ps, rating

        g.fallback = recording_fallback


original_phonemize = g2p.phonemize


def phonemize(text, lang_code):
    global current
    instrument(g2p.pipeline(lang_code))
    outer, current = current, {"input": text}
    try:
        result = original_phonemize(text, lang_code)
        current["segments"] = current.get("segments", [[]])[0]
        current["phonemes"] = result.phonemes
        if result.tokens is not None:
            current["tokens"] = [[t.text, t.tag, t.whitespace, t.phonemes] for t in result.tokens]
        call = current
    finally:
        current = outer
    calls.append(call)
    return result


tp.phonemize = phonemize
calls = []


def normalized(text, lang_code, options):
    processed = tp.join_lines(text)
    normalizer = tp.get_normalizer(lang_code)
    if tp.settings.advanced_text_normalization and options.normalize and normalizer is not None:
        parts = tp.CUSTOM_PHONEMES.split(processed)
        for index in range(0, len(parts), 2):
            parts[index] = normalizer.normalize(parts[index], options)
        processed = "".join(parts).strip()
    return processed


async def dump(record_in):
    calls.clear()
    options = NormalizationOptions()
    text, lang = record_in["text"], record_in["lang"]
    # smart_split normalizes each part between pause tags separately; normalized/sentences below don't.
    if tp.PAUSE_TAG_PATTERN.search(text):
        raise ValueError(f"{record_in['id']}: pause tags aren't supported in the corpus")
    out = dict(record_in)
    out["normalized"] = normalized(text, lang, options)
    out["sentences"] = [s.strip() for s in tp.sentences(out["normalized"]) if s.strip()]
    out["chunks"] = []
    async for chunk_text, phonemes, _pause in tp.smart_split(text, lang_code=lang, normalization_options=options):
        if phonemes is not None:
            out["chunks"].append({"text": chunk_text, "phonemes": phonemes.phonemes})
    out["calls"] = list(calls)
    return out


async def main(src, dst):
    os.makedirs(os.path.dirname(os.path.abspath(dst)), exist_ok=True)
    with open(src) as fin, open(dst, "w") as fout:
        for line in fin:
            if line.strip():
                fout.write(json.dumps(await dump(json.loads(line)), ensure_ascii=False) + "\n")


if __name__ == "__main__":
    asyncio.run(main(sys.argv[1], sys.argv[2]))
