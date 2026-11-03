# Reference pipeline

The Rust port has to produce the same phonemes as the deployed Python server, Kokoro-FastAPI at a pinned commit (`setup.sh`), using the kokoro fork and misaki versions it locks. This directory runs that pipeline over text and records every stage, so each Rust component can be tested against the stage it replaces.

```sh
./setup.sh                      # Kokoro-FastAPI venv in .cache/ (CPU torch, UniDic for Japanese)
python3 fetch_corpus.py         # optional: books and Wikipedia paragraphs into corpus/ (not committed)
.cache/kfa/.venv/bin/python -I dump.py fixtures/hard_cases.jsonl fixtures/hard_cases.out.jsonl
.cache/kfa/.venv/bin/python -I dump.py corpus/gutenberg.jsonl out/gutenberg.jsonl
```

`espeak_dump.py` does the same for misaki's two espeak wrappers alone (`EspeakFallback`, `EspeakG2P`), recording each call phonemizer makes to C espeak-ng. `espeak_cases.jsonl` holds hand-written cases; `--build` adds every fallback word and foreign run in the dumps, plus words, sentences and script runs from the raw corpus. `crates/espeak/tests/espeak.jsonl` is its output for the committed cases and the Gutenberg dump. `espeak_bench.py` times the same calls.

```sh
.cache/kfa/.venv/bin/python -I espeak_dump.py --build espeak_cases.jsonl out/gutenberg.jsonl corpus/wiki-*.jsonl > /tmp/cases.jsonl
.cache/kfa/.venv/bin/python -I espeak_dump.py /tmp/cases.jsonl out/espeak-corpus.jsonl
```

`dump.py` documents the output format; `crates/reference` reads it. `fixtures/hard_cases.out.jsonl` is committed so tests run without the Python setup. Dumps of the downloaded corpus go in `out/`, and the parity tests pick them up when present.

Differences from the deployed image: espeak reads the same espeak-ng 1.52 data (bundled with espeakng-loader), from a copy at a shorter path (see `setup.sh`).
