# Reference pipeline

The Rust port has to produce the same phonemes as the deployed Python server, Kokoro-FastAPI at a pinned commit (`setup.sh`), using the kokoro fork and misaki versions it locks. This directory runs that pipeline over text and records every stage, so each Rust component can be tested against the stage it replaces.

```sh
./setup.sh                      # Kokoro-FastAPI venv in .cache/ (CPU torch, UniDic for Japanese)
python3 fetch_corpus.py         # optional: books and Wikipedia paragraphs into corpus/ (not committed)
.cache/kfa/.venv/bin/python dump.py fixtures/hard_cases.jsonl fixtures/hard_cases.out.jsonl
.cache/kfa/.venv/bin/python dump.py corpus/gutenberg.jsonl out/gutenberg.jsonl
```

`espeak_dump.py` does the same for misaki's two espeak wrappers alone (`EspeakFallback`, `EspeakG2P`), recording each call phonemizer makes to C espeak-ng. `espeak_cases.jsonl` holds hand-written cases; `--build` adds every fallback word and foreign run in the dumps, plus words, sentences and script runs from the raw corpus. `crates/espeak/tests/espeak.jsonl` is its output for the committed cases and the Gutenberg dump. `espeak_bench.py` times the same calls.

```sh
.cache/kfa/.venv/bin/python -I espeak_dump.py --build espeak_cases.jsonl out/gutenberg.jsonl corpus/wiki-*.jsonl > /tmp/cases.jsonl
.cache/kfa/.venv/bin/python -I espeak_dump.py /tmp/cases.jsonl out/espeak-corpus.jsonl
```

`dump.py` documents the output format; `crates/reference` reads it. `fixtures/hard_cases.out.jsonl` is committed so tests run without the Python setup. Dumps of the downloaded corpus go in `out/`, and the parity tests pick them up when present.

The text functions before G2P (normalizer options, tags, chunking) have their own fixture, since books and Wikipedia rarely reach most of them. `crates/text` also needs Python's character classes, generated from this interpreter:

```sh
python3 text_cases.py fixtures/text_cases.jsonl   # hand-written cases; dump_text.py adds seeded random ones
.cache/kfa/.venv/bin/python -I dump_text.py fixtures/text_cases.jsonl fixtures/text_cases.out.jsonl
.cache/kfa/.venv/bin/python -I gen_text_tables.py ../crates/text/src/tables.rs
.cache/kfa/.venv/bin/python -I gen_float_repr.py 10000 fixtures/float_repr.txt   # repr(float), including ties
```

Differences from the deployed image: espeak reads the same espeak-ng 1.52 data (bundled with espeakng-loader), from a copy at a shorter path (see `setup.sh`).

`crates/spacy-en` has its own recorder for spaCy alone, which covers far more text per second than `dump.py`. Its parity test reads `out/spacy-en/*.jsonl`:

```sh
mkdir -p out/spacy-en
.cache/kfa/.venv/bin/python -I ../crates/spacy-en/reference.py out/spacy-en/corpus.jsonl corpus/*.jsonl
.cache/kfa/.venv/bin/python -I ../crates/spacy-en/reference.py --patterns out/spacy-en/patterns-200k.jsonl 200000
```

`crates/pipeline` (the whole English path, text to phoneme chunks) is tested end to end against the dumps above. Its script segmentation (kokoro's `foreign.py`) also has a random-text fixture, its Unicode tables come from the `regex` package here, and `pipeline_bench.py` times the reference's `smart_split` for comparison with `cargo run --release -p kokoro-pipeline --example bench`:

```sh
.cache/kfa/.venv/bin/python -I foreign_cases.py 3000 ../crates/pipeline/tests/foreign_cases.jsonl
.cache/kfa/.venv/bin/python -I gen_pipeline_tables.py ../crates/pipeline/src/tables.rs
.cache/kfa/.venv/bin/python -I pipeline_bench.py --dump out/wiki-en.jsonl   # or FILE [REPEAT] as one request
```

## C espeak-ng oracle

`espeak-ng-oracle/` builds C espeak-ng 1.52.0 statically into small drivers, to test `crates/espeak-ng` (the Rust port of its text-to-phoneme engine) on any text, not just the calls the Python pipeline makes. It reads the same data as the venv. Nothing here ships.

```sh
cd espeak-ng-oracle && make                       # downloads the 1.52.0 source; builds ttp and bench
make sanitized                                     # ttp-asan and ttp-memcheck, for --ub
python3 oracle.py one gmw/en-US "Hello world."      # C's clauses for one text
python3 gen_adversarial.py 1 3000 gmw/en-US roa/fr > /tmp/pairs.jsonl
python3 oracle.py pairs --ub < /tmp/pairs.jsonl > /tmp/calls.jsonl   # each call in a fresh C library
cargo run --release -p kokoro-espeak-ng --example compare -- /tmp/calls.jsonl .cache/espeak-ng-data
python3 adv.py --ub gen_dict.py 1 4 3000 gmw/en-US zle/ru  # generate, run C, compare, for seeds 1-4
python3 bench.py /tmp/calls.jsonl 3                 # C (-O3) vs Rust time per call and per word
python3 make_fixture.py ../../crates/espeak-ng/tests/c_calls.jsonl
```

Call files are JSON lines `{"voice", "text", "clauses"}`, with `{"voice", "reset": true}` where C crashed. Each call's answer comes from a child that `ttp` forks right after `espeak_SetVoiceByName` (`TTP_FORK`), so it starts from the state of a new library with that voice, as every kokoro-espeak-ng call does. C itself keeps embedded commands such as `\x01B` and read-ahead characters from one call to the next; `oracle.py pairs --history` reproduces that (one process per voice, calls in order), which is what phonemizer sees, and `docs/divergences.md` says why the port doesn't. `ESPEAK_NG_CALLS` (paths separated by `:`) and `ESPEAK_NG_DATA` point it at more call files and the full data.

C espeak-ng 1.52 reads out of bounds and reads uninitialised stack memory on some inputs (it walks off the start of phoneme lists, past the end of `$text` replacements, and past the end of a clause when grouping digits), so its answer for those calls depends on whatever was in memory. `make sanitized` builds `ttp-asan` (ASan) and `ttp-memcheck` (for valgrind's memcheck; set `VALGRIND_CFLAGS=-I…/include` if valgrind's headers aren't installed). `oracle.py pairs --ub` (and `adv.py --ub`) runs every call under both as well and adds `"ub": [reports]` to the calls where C did either. Those calls run in the comparisons but aren't scored; `docs/divergences.md` lists what the port does instead. `VALGRIND` and `VALGRIND_LIB` select a valgrind that isn't on `PATH`.
