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

## C espeak-ng oracle

`espeak-ng-oracle/` builds C espeak-ng 1.52.0 statically into small drivers, to test `crates/espeak-ng` (the Rust port of its text-to-phoneme engine) on any text, not just the calls the Python pipeline makes. It reads the same data as the venv. Nothing here ships.

```sh
cd espeak-ng-oracle && make                       # downloads the 1.52.0 source; builds ttp and bench
python3 oracle.py one gmw/en-US "Hello world."      # C's clauses for one text
python3 gen_adversarial.py 1 3000 gmw/en-US roa/fr > /tmp/pairs.jsonl
python3 oracle.py pairs < /tmp/pairs.jsonl > /tmp/calls.jsonl   # one C process per voice, in order
cargo run --release -p kokoro-espeak-ng --example compare -- /tmp/calls.jsonl .cache/espeak-ng-data
python3 adv.py gen_dict.py 1 4 3000 gmw/en-US zle/ru  # generate, run C, compare, for seeds 1-4
python3 bench.py /tmp/calls.jsonl 3                 # C (-O3) vs Rust time per call and per word
python3 make_fixture.py ../../crates/espeak-ng/tests/c_calls.jsonl
```

Call files are JSON lines `{"voice", "text", "clauses"}`, with `{"voice", "reset": true}` where C crashed and restarted (espeak-ng 1.52 reads out of bounds in `CountVowelPosition` and `isAfterStress` on some inputs). The engine's parity test replays them with one engine per voice, since C carries state between calls. `ESPEAK_NG_CALLS` (paths separated by `:`) and `ESPEAK_NG_DATA` point it at more call files and the full data.
