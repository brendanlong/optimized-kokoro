"""Times the reference server's smart_split (text to phoneme chunks) for comparison with
`cargo run --release -p kokoro-pipeline --example bench`.

Usage (from reference/, after ./setup.sh):
    .cache/kfa/.venv/bin/python -I pipeline_bench.py FILE [REPEAT]      # FILE as one request
    .cache/kfa/.venv/bin/python -I pipeline_bench.py --dump out/wiki-en.jsonl  # each English record

KOKORO_BENCH_LANG=z (or b) times another pipeline, and --dump takes that language's records.
"""

import asyncio
import json
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".cache", "kfa"))

from phonemizer.backend.espeak.wrapper import EspeakWrapper  # noqa: E402

import misaki.espeak  # noqa: E402,F401

EspeakWrapper.set_data_path(os.path.join(HERE, ".cache", "espeak-ng-data"))

from loguru import logger  # noqa: E402

logger.remove()

from api.src.services.text_processing import text_processor as tp  # noqa: E402
from api.src.structures.schemas import NormalizationOptions  # noqa: E402

# The server's import configures its own log handler; logging every chunk isn't G2P time.
logger.remove()

LANG = os.environ.get("KOKORO_BENCH_LANG", "a")


async def run(text):
    chunks = 0
    async for _text, phonemes, _pause in tp.smart_split(text, lang_code=LANG, normalization_options=NormalizationOptions()):
        chunks += phonemes is not None
    return chunks


async def main():
    await run("Warm up the lexicon and espeak data, Mr. Qzxv. 银行行长。")
    if sys.argv[1] == "--dump":
        texts = []
        for path in sys.argv[2:]:
            with open(path) as f:
                texts += [r["text"] for r in map(json.loads, f) if r["lang"] == LANG]
        repeat = 1
    else:
        with open(sys.argv[1]) as f:
            texts = [f.read()]
        repeat = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    chunks = chars = 0
    start = time.perf_counter()
    for _ in range(repeat):
        for text in texts:
            chars += len(text)
            chunks += await run(text)
    elapsed = time.perf_counter() - start
    requests = len(texts) * repeat
    print(
        f"{requests} requests, {chunks} chunks, {chars} characters: {elapsed * 1e6 / requests:.1f} µs per request, "
        f"{elapsed * 1e6 / chunks:.1f} µs per chunk, {elapsed * 1e6 / chars:.2f} µs per character"
    )


asyncio.run(main())
