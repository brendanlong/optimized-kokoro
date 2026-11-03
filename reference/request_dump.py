"""The reference server's output for whole requests, as crates/pipeline returns it: smart_split's
chunks and pauses, each chunk's model ids (KModel's vocabulary filter) and its words as
`[word, first id, end id]` (`_word_spans` with `_render_stitched`'s filter). A request that
raises records `{"error"}` after the chunks it yielded.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I request_dump.py < cases.jsonl > ../crates/pipeline/tests/requests.jsonl
"""

import asyncio
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".cache", "kfa"))

from phonemizer.backend.espeak.wrapper import EspeakWrapper  # noqa: E402

import misaki.espeak  # noqa: E402,F401

EspeakWrapper.set_data_path(os.path.join(HERE, ".cache", "espeak-ng-data"))

from huggingface_hub import hf_hub_download  # noqa: E402
from loguru import logger  # noqa: E402

logger.remove()

from api.src.inference.kokoro_v1 import _word_spans  # noqa: E402
from api.src.services.text_processing import text_processor as tp  # noqa: E402
from api.src.structures.schemas import NormalizationOptions  # noqa: E402

with open(hf_hub_download("hexgrad/Kokoro-82M", "config.json")) as f:
    VOCAB = json.load(f)["vocab"]


def words(phonemes):
    kept = [i for i, c in enumerate(phonemes.phonemes) if c in VOCAB]
    index = {k: n for n, k in enumerate(kept)}
    out = []
    for word, first, last in _word_spans(phonemes):
        inside = [i for i in range(first, last + 1) if i in index]
        if inside:
            out.append([word, index[inside[0]], index[inside[-1]] + 1])
    return out


async def run(text, lang):
    chunks = []
    try:
        async for chunk_text, phonemes, pause in tp.smart_split(text, lang_code=lang, normalization_options=NormalizationOptions()):
            if phonemes is None:
                if pause is not None:
                    chunks.append({"pause": pause})
                continue
            ids = [VOCAB[c] for c in phonemes.phonemes if c in VOCAB]
            chunks.append({"text": chunk_text, "phonemes": phonemes.phonemes, "ids": ids,
                           "dropped": len(phonemes.phonemes) - len(ids), "words": words(phonemes)})
    except Exception as e:
        chunks.append({"error": f"{type(e).__name__}: {e}"})
    return chunks


async def main():
    for line in sys.stdin:
        if line.strip():
            case = json.loads(line)
            case["chunks"] = await run(case["text"], case["lang"])
            print(json.dumps(case, ensure_ascii=False), flush=True)


asyncio.run(main())
