# Provenance

The four lexicons here are unmodified copies of `misaki/data/` from misaki 0.9.4 (the version Kokoro-FastAPI locks; installed by `reference/setup.sh`). misaki is © hexgrad and licensed Apache-2.0: https://github.com/hexgrad/misaki

| File | SHA-256 |
|---|---|
| `us_gold.json` | `dc414872a49a28ae6c141463d502fd945f3b2fde040484fdc47d00cc4612686f` |
| `us_silver.json` | `de8f67be911bb6c659187b4a65fd966b6a30e56350e0f790d763210b053ac475` |
| `gb_gold.json` | `29e62f4b60261c88f7f3c2c7811ca3825978948090b72d2b27d565b729282f71` |
| `gb_silver.json` | `48131e2d92ccc41655f4543e87e0f938e71463eb5a54be7f0693bb712ebb6bce` |

They are compiled into the crate with `include_str!` and parsed when a `G2p` is created (about 40–50 ms per accent in a release build). Keys and phonemes borrow from the embedded text; only the case variants `Lexicon.grow_dictionary` adds are allocated.

The files are unchanged; `lexicon.rs` drops the keys "dr" and "mr" after loading them (`GUESSES`, `docs/divergences.md`, "misaki English: lowercase abbreviations").
