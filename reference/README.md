# Reference pipeline

The Rust port has to produce the same phonemes as the deployed Python server, Kokoro-FastAPI at a pinned commit (`setup.sh`), using the kokoro fork and misaki versions it locks. This directory runs that pipeline over text and records every stage, so each Rust component can be tested against the stage it replaces.

```sh
./setup.sh                      # Kokoro-FastAPI venv in .cache/ (CPU torch, UniDic for Japanese)
python3 fetch_corpus.py         # optional: books and Wikipedia paragraphs into corpus/ (not committed)
.cache/kfa/.venv/bin/python dump.py fixtures/hard_cases.jsonl fixtures/hard_cases.out.jsonl
.cache/kfa/.venv/bin/python dump.py corpus/gutenberg.jsonl out/gutenberg.jsonl
```

`dump.py` documents the output format; `crates/reference` reads it. `fixtures/hard_cases.out.jsonl` is committed so tests run without the Python setup. Dumps of the downloaded corpus go in `out/`, and the parity tests pick them up when present.

The text functions before G2P (normalizer options, tags, SSML, chunking) have their own fixture, since books and Wikipedia rarely reach most of them. `crates/text` also needs Python's character classes, generated from this interpreter:

```sh
python3 text_cases.py fixtures/text_cases.jsonl   # hand-written cases; dump_text.py adds seeded random ones
.cache/kfa/.venv/bin/python -I dump_text.py fixtures/text_cases.jsonl fixtures/text_cases.out.jsonl
.cache/kfa/.venv/bin/python -I gen_text_tables.py ../crates/text/src/tables.rs
```

Differences from the deployed image: espeak reads the same espeak-ng 1.52 data (bundled with espeakng-loader), from a copy at a shorter path (see `setup.sh`).
