# Provenance

These files are the dictionaries misaki 0.9.4's legacy Chinese G2P reads (`ZHG2P(version=None)`, which Kokoro-82M v1.0 uses), converted by `reference/gen_zh_data.py` from the reference venv (`reference/setup.sh`, the versions Kokoro-FastAPI locks). The script checks its output against the libraries themselves: every character's IPA equals `ZHG2P.word2ipa` on it, and the characters with a reading are exactly those `lazy_pinyin(c)[0] != c` holds for, over every code point.

| File | From | License |
|---|---|---|
| `jieba.txt` | jieba 0.42.1 `dict.txt` (words and frequencies, without part-of-speech tags; only words of U+4E00-U+9FD5, the only characters jieba's DAG sees in misaki's input; `total` over all of `dict.txt`) | MIT, © 2013 Sun Junyi (`../LICENSE-jieba`) |
| `hmm.txt` | jieba 0.42.1 `finalseg/prob_emit.py` (`repr` of each float). `prob_start.py` and `prob_trans.py` are constants in `src/jieba.rs` | MIT, as above |
| `chars.txt` | pypinyin 0.55.0 `pinyin_dict.json`: each character's first reading | MIT, © 2016 mozillazg, 闲耘 (`../LICENSE-pypinyin`) |
| `phrases.txt` | pypinyin 0.55.0 `phrases_dict.json`: each phrase's first reading per character | MIT, as above |
| `syllables.txt` | Each reading above through pypinyin's TONE3 style (neutral tone as 5) and misaki's `pinyin_to_ipa` and `retone` (`misaki/transcription.py`, adapted by misaki from pinyin-to-ipa, MIT) | misaki: Apache-2.0, © hexgrad |

cn2an 0.5.23 (MIT, © 2017 Ailln, `../LICENSE-cn2an`) has no data files; its number tables are in `src/cn2an.rs`.

SHA-256 of the sources:

| Source | SHA-256 |
|---|---|
| `jieba/dict.txt` | `7197c3211ddd98962b036cdf40324d1ea2bfaa12bd028e68faa70111a88e12a8` |
| `jieba/finalseg/prob_emit.py` | `27d46b1c9efe4dd148fde8be042a21be40e3562d0c7f1273f9de7abae12ebb8d` |
| `jieba/finalseg/prob_start.py` | `14c5706ced5cd3b42eb4873d4b88f7f52a7bdf80fbd767bc4423d361e20c5330` |
| `jieba/finalseg/prob_trans.py` | `54dfbc252ed71480d4f0cdfdf516ecfbe44efd0f6c3c64b158e7039f2906c91b` |
| `pypinyin/pinyin_dict.json` | `5f294c01e6c6c0a1c8e329c79335a3f8e0b27d06bf1de7a99244b765892d1e5b` |
| `pypinyin/phrases_dict.json` | `a45ff140a6b631ca9c82127b280a2f414e0aba6bb2824a0e9d1e77fff359c665` |

And of the files here:

| File | SHA-256 |
|---|---|
| `chars.txt` | `9cd5528a74fb159d980b486bdda80ab67fa576481ea0889628605ab9f90adaf6` |
| `hmm.txt` | `e9b7080bfc9598bff76c33cec38d32c637000ef5725aafbdc3927f34720ecc10` |
| `jieba.txt` | `cc98c7720d5b309d758446d9ba1bd3ffdcd7a9a5ceddc68fcd818e26fb856bc8` |
| `phrases.txt` | `6d4e7921a2e2fed60493ef1253c4c8bc3da62b9f82d8e8159075256d1448ebb1` |
| `syllables.txt` | `adb5a1e859a67e5b713064a683c72ca32761e8a749ebac1ea49cd5a2cccfdc04` |

They are compiled in with `include_str!` and parsed when a `ZhG2p` is made: about 80–95 ms in a release build, and about 45 MB resident for the Chinese G2P alone. The pipeline loads one per process, lazily, on the first Han character or Chinese request (Brendan's decision, 2026-11-03), so an English-only server never pays for it.

## Upstream sources of pypinyin's data

pypinyin builds `pinyin_dict` from its author's [pinyin-data](https://github.com/mozillazg/pinyin-data) (MIT), which merges the readings of the Unicode Han Database (Unihan: `kTGHZ2013`, `kHanyuPinyin`, `kXHC1983`, `kHanyuPinlu`, `kMandarin`) with hand corrections. Unihan is © Unicode, Inc. under the Unicode License v3, whose notice is in `../COPYING.UCD`; pypinyin itself ships no Unicode notice. `phrases_dict` comes from [phrase-pinyin-data](https://github.com/mozillazg/phrase-pinyin-data) (MIT) 0.19.0 (commit `cee0ed6`, the submodule pypinyin v0.55.0 pins; its `phrases_dict.json` hashes to the value above).

### CC-CEDICT

phrase-pinyin-data also carries `cc_cedict.txt`, converted from CC-CEDICT (CC BY-SA 4.0), but none of it is in the phrase readings here:

- pypinyin's `make gen_phrases_dict` reads only phrase-pinyin-data's `pinyin.txt`, which its `make merge` builds from `pinyin.txt` and `overwrite.txt`. `cc_cedict.txt` (with zdic.net's `zdic_cibs.txt`/`zdic_cybs.txt`) only goes into `large_pinyin.txt`, which pypinyin doesn't use. The 47,111 phrases here are exactly the phrases of `pinyin.txt` at `cee0ed6`.
- 41,126 of those phrases were imported in March 2017 from hotoo/pinyin's and pypinyin's earlier phrase tables, two years before CC-CEDICT was added (April 2019). Only 49% of them are CC-CEDICT words at all, so the base isn't derived from it.
- Of the later edits to `pinyin.txt`/`overwrite.txt` that survive into the shipped data, none cites CC-CEDICT, and none of the bulk ones looks copied from it. The 3,993 words of phrase-pinyin-data PR #35 include 29 CC-CEDICT words. Commit `30980d7`'s 645 corrections include 161 CC-CEDICT words, and only 88 of those have CC-CEDICT's reading. Commit `916efc6` corrects 764 readings using a scraped web dictionary posted in phrase-pinyin-data issue #53. Of those, 319 agree with CC-CEDICT, which is what two dictionaries agreeing on standard readings looks like. In all, 41 added phrases and 389 changed readings match CC-CEDICT and not zdic.net. Each is a single-word correction from a commit with another stated source, not an import.

The check was a one-off script over phrase-pinyin-data's git history, not kept here. Redo it if pypinyin is upgraded.
