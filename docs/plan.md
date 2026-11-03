# Optimized Kokoro: feasibility and plan (draft)

Goal (from Brendan): a Kokoro TTS server we can host without worrying about C exploits or slow Python. It should do no duplicate work, run optimized graphs, batch whatever is already queued without waiting, avoid C and risky dependencies, and stream audio as fast as possible.

Items marked **Decided** are settled with Brendan; the rest is proposal. Numbers are from the measurements in the wiki (`kokoro-tts-local-vs-gpu`, `self-hosted-kokoro-exposure-options`, journals `2026-10-31/11-01--kokoro-fastapi-latency-profile*`, `2026-11-01--kokoro-architecture-retrain-ideas`), all on the RTX 3060 Ti.

## Short answer

It's feasible. Two of the goals need restating, though:

1. **"No C at all" can't hold on a GPU.** The NVIDIA driver, `libcuda`, and cuBLAS/cuDNN or NVRTC are closed C/C++ whatever framework we pick. The goal we can meet: **no C or Python touches attacker-controlled bytes.** Text, HTTP and G2P run in memory-safe Rust. The GPU process only ever receives arrays of small integers that Rust has already range-checked.
2. **Batching helps, but it needs length buckets and masking.** A whole-model test of equal-length batches on the 3060 Ti (3c3e review, 2026-11-02) measured these speedups:
   - 50-token sentences: 127× real time at batch size 1, 296× at batch size 8;
   - 150-token sentences: 201× at batch size 1, 328× at batch size 8.

   The card tops out around 330–350× real time. Typical sentences are short (median 81 phonemes), and short ones gain the most. An earlier decoder-only test at 470 frames found no gain, which fits: long inputs already fill the GPU. Mixed lengths are the catch. InstanceNorm/AdaIN and the bidirectional LSTMs read the padding, and padding changed the audio (spectrogram correlation 0.946–0.975 vs unpadded, noise floor ~0.99, in journal `2026-11-01--kokoro-fastapi-latency-profile-cont`). With or without a masked norm, padded audio measured 0.964 against a 0.995 floor, and the cause wasn't found. But Brendan listened and **couldn't hear the difference** (same journal). So padded, length-bucketed batching looks fine in practice. Do a listening check on real batched output before shipping, and treat finding the cause as optional cleanup.

What's large: **G2P (turning text into phonemes) in Rust, at parity with misaki.** Everything else is small or already done.

## Proposed architecture

```
client ──► [Fly relay, Rust]  ◄── outbound connection ── [desktop: front, Rust] ──unix socket──► [model worker]
           (public; auth;                                  text normalize, G2P,     fixed binary     gVisor + nvproxy,
            OpenAI-ish route)                              chunking, scheduler,     frames           GPU, no network
                                                           PCM streaming
```

- **Front (Rust, sandboxed, no GPU).** Owns every byte from the network: request parsing, text normalization, sentence chunking, G2P, the voice table, the scheduler and the PCM stream back. It sends the worker `{slot, voice_index, speed, phoneme_ids: [u8; ≤510]}` and gets back `{slot, durations, pcm: [i16]}` frames (`worker-protocol.md`).
- **Model worker.** Only reads length-prefixed integer frames, and **re-validates them itself**. It doesn't trust the front: every phoneme id must be in the vocab, the voice index in range, the length ≤ 510, and speed (an integer, in hundredths) within bounds. Weights load from safetensors (no pickle). It runs in gVisor `runsc --nvproxy`, which forwards only the NVIDIA ioctls on its list for that driver version. That list is probably the "whitelist of allowed GPU commands" you remembered. The arguments to those ioctls are still forwarded, though, so the sandbox doesn't make the worker safe to compromise. It has no network and a read-only filesystem. nvproxy supports exact driver versions only, so pin the host driver and runsc together.
- **Desktop listens on nothing.** This matches the relay design already chosen in `self-hosted-kokoro-exposure-options`. The relay and agent authenticate each other (mutual TLS or a pinned key). A stolen agent credential would receive every listener's text, so store it outside the `claude` user.

### Protocol question (OpenAI-compatible HTTP vs MessagePack-RPC)

Most of the risk was never in the HTTP format. It was in the C and Python parsers behind it, and G2P (espeak-ng is C, spaCy is Cython, MeCab is C++). With Rust on both ends:

- **Between the relay and the desktop:** a fixed binary frame over one outbound TLS stream. Use length-prefixed `postcard`/hand-rolled structs. Check each length prefix against a hard cap *before* allocating. MessagePack adds nothing over that. The agent's hello frame carries the voice list.
- **Relay API:** two routes, `POST /v1/audio/speech` and `GET /v1/audio/voices`, which Lion Reader's `selfhosted.ts` also calls. They go through `hyper` (routing is one `match`, no framework) with a body-size cap. `/v1/audio/speech` returns `audio/pcm` (24 kHz s16le), with no encoding. The API stays on Fly's private network, so only the agent's connection endpoint is public, and auth is checked before reading a body or accepting an upgrade. That's a small, well-fuzzed surface, and Lion Reader and clawed-abode keep working. **Decided (Brendan, 2026-11-03):** support only our own clients. The API takes exactly the fields Lion Reader and clawed-abode send (`model`, ignored; `input`, `voice`, `speed`, `response_format: "pcm"`, `stream`) and refuses any other with 400 (`divergences.md`).

### Audio

Return raw PCM. No mature pure-Rust Opus or AAC encoder exists (the `opus` crate wraps libopus, which is C). So encoding to fMP4/Opus/AAC happens wherever the client side already does it, either in the browser with WebCodecs or in Lion Reader. 24 kHz mono s16 is 384 kbit/s, which is fine.

### Model runtime: PyTorch first, Rust later, ideally after a smaller model

| Option | Speed we know | Risk |
|---|---|---|
| **PyTorch + torch.compile in the worker** | Model alone at batch size 1: 150–200× real time. The deployed server (one sentence per chunk) gets 95× and 48 ms to first audio, or ~135× with two processes. The gap is Python overhead around the model, which the GPU is only ~42% busy through. | Large C++ dependency, but it only ever sees integer arrays inside gVisor. Startup ~50 s for the compile. **Not yet tested in gVisor**: Inductor and Triton need gcc, ptxas and a writable cache. |
| Rust: burn (CubeCL CUDA, fuses ops automatically) | unknown | Speed on Kokoro's conv-heavy decoder is untested, plus LSTMs, ConvTranspose1d, STFT. Still needs the NVIDIA driver and NVRTC. |
| Rust: candle (cudarc + cuBLAS/cuDNN) | unknown; Crane's Kokoro uses ONNX Runtime (C++), not candle | Same gaps. Its LSTM is a slow per-timestep loop. |
| ONNX Runtime (Kokoros, Crane) | Slower than torch on CUDA in our one test (onnxruntime-gpu 1.23.2; some decoder convs used cuDNN fallback, see `onnx-istft-convtranspose-slow-on-cuda`). Not a careful measurement. | C++, so it fails goal 4 anyway. |

I recommend **PyTorch in the sandboxed worker first**, behind the integer-only protocol. It's the fast path we've already measured, and it moves C/Python off the untrusted path. Taking G2P, chunking, encoding and the event loop out of the Python process should also close most of the 95× → 150–200× gap. That's probably the biggest throughput win available, ahead of any runtime port. A Rust port becomes much easier and more worthwhile **after** the distillation work (PR kokoro#3): replace the LSTMs with convs (already 36 ms vs 50 ms, but the text encoder's LSTM costs WER), then possibly a Vocos-style head. That turns the model into plain convs, norms and one iSTFT, which any runtime does well. The profile also says the remaining headroom is in fusing elementwise ops: ~62% of eager kernel time is elementwise. That's an upper bound of ~3×, and torch.compile already recovers part of it (46 ms compiled vs 75 ms eager for 150 tokens). That's an argument for burn's fusion or hand-written kernels, but only once we've measured one.

### G2P: the big piece

What's deployed isn't "misaki English". It's a pipeline: script segmentation (kokoro#2, `foreign.py`), then per-language G2Ps, then post-processing. Every piece has its own parity traps. (Details from the a189 review on PR #1.)

- **English.** spaCy's **tokenizer** sets the token boundaries for everything else: contractions, hyphens, number and currency spans. Tokenizer drift will likely cause more diffs than tagging will. misaki uses POS tags in about 66 places, not just for heteronyms: function-word reductions (`to` → `tə`/`tu`), stress on function words, straight-quote direction, numbers through `num2words`. So the Rust tagger has to produce full PTB tags (an averaged perceptron trained on PTB/UD is a known, small design), or we list every tag-dependent rule and test each one.
- **espeak covers six languages** (ru/el/ko/ar/he/hi) as well as unknown English words. So a distilled character model would only replace the English fallback; an espeak engine is needed regardless (now our own port, `kokoro-espeak-ng`, see below). Pin the reference espeak version, since misaki's `E2M` rewrites depend on it. Port our own post-processing too: strip Hebrew vowel points, map `ħ→h` and `ʕ→ʔ`, and treat Greek as Greek only from 3 letters. Licensing doesn't change: the current deploy already ships GPL-3 espeak-ng.
- **Chinese.** The v1.0 model uses misaki's **legacy** path (jieba + pypinyin `lazy_pinyin` + `transcription.py`), not `ZHFrontend`. Polyphones (行, 长) depend on pypinyin's phrase dictionary, and segmentation depends on jieba's dictionary version. So the Rust side has to use the same dictionary data, not whatever a pinyin crate ships. Also test which characters have no reading (they fall back to English), and the Mandarin-accent rules (kokoro#6).
- **Japanese** is cutlet: MeCab with full UniDic, then romanization rules, then misaki's kana tables. `lindera` only replaces the MeCab step, so this is about as much work as zh.
- **Segmentation** (`foreign.py`, ~140 lines): port `tests/test_foreign.py` one-to-one and use it as the spec.

**Parity harness.** Diff per-chunk phonemes from our `KPipeline` (`en_g2p_tokens`/`g2p_phonemes`, pinned to a commit), not from misaki alone, because segmentation, chunking and post-processing live outside misaki. Corpus: real Lion Reader articles plus the mixed-script sets from kokoro#2. Weight the diffs, since many are inaudible (primary vs secondary stress, `ᵊ`). For the rest, use blind transcription by two audio models as a cheap second check.

**Order:** harness and corpus → English tokenizer and lexicon (measure the match rate) → tagger → segmentation + our espeak-ng port (covers the English fallback and every other espeak language) → zh (legacy path) → ja last (largest dependency, least traffic).

**Decided (Brendan, 2026-11-02): go straight to the Rust port.** There's no interim Python G2P service. The Rust code loads misaki's own data files where it can: the gold/silver lexicons, pypinyin's phrase and character dictionaries, jieba's dictionary, `ja_words.txt`, and the espeak-ng data. The Python pipeline stays only as the offline reference for the parity harness. Every language the current deploy supports gets ported. The current server isn't part of the end state or the migration.

Rejected alternatives (measured in session decf45d1):
- **One learned G2P model.** It's simpler to serve, but not faster: Python G2P is already ~1.5% of request time. It would also need a teacher and carries long-tail risk.
- **misaki's BART fallback for unknown words.** About 10 ms per word on CPU, one word at a time, and it truncates long words. The deploy uses espeak instead, where 32 unknown words cost about the same as 70 known ones (~4 ms).
- **Treating duplicate symbols as interchangeable.** Kokoro treats `T` and `ɾ`, and `ts` and `ʦ`, as different sounds (wiki `kokoro-duplicate-phoneme-tokens`). So the port must reproduce each pipeline's exact symbol conventions.

### espeak: port the engine, keep its rules

**Decided (Brendan, 2026-11-02):** port espeak-ng 1.52's text-to-phoneme engine to Rust ourselves, function by function, and use espeak's per-language rule and dictionary files unchanged. English is the goal; other languages should work too, without anyone rewriting rules per language.

Why: an early 25-sentence check suggested espeak-ng-rs was usable, but on the 191k-case harness it matched C espeak on only 44–68% of sentences for es/fr/it/pt/hi/ko/ar, and on 84% of the unknown English words misaki sends it (PR #6). Parts of it, like the clause splitter, reinvent the C code instead of porting it, so fixing it would have a long tail. espeak's per-language data is small (Spanish is 12 KB of rules plus exceptions; English 265 KB). The complexity is in one shared engine (dictionary/rule matching, word translation, numbers, per-language options, clause reading, phoneme lists). The synthesis half isn't needed. A faithful engine port gets every espeak language from its unchanged data, and every function can be checked against C.

Status: PR #8 (about 12.5k lines of Rust) matches C exactly on every scored call: the corpus, adversarial text, and generated text in all 141 voices. It diverges only where C reads undefined memory or carries state between calls (see `divergences.md`).

How it's checked: exact output against C espeak 1.52 (the 191k-case harness from PR #6, plus direct calls). For languages nobody here speaks, quality questions such as simplifications are judged by synthesizing, transcribing with a multilingual ASR model, and comparing the error rate with C espeak's.

### Matching the reference vs simplifying it

**Decided (Brendan, 2026-11-02):** part of the goal is simplification. Where misaki or Kokoro-FastAPI does something overly complex, or wrong, we can do it a better way, as long as the final model still pronounces the result reasonably. Also decided: no state passes between requests.

Proposed process (mine, not yet confirmed by Brendan):
- **Port exactly first** where it's cheap. An exact port, proven by the reference harness, shows we understand the behaviour, and turns every later change into a measured diff instead of an accident. The spaCy, text and misaki-English ports did this.
- **Every intentional difference is listed** in `docs/divergences.md`, with the reason, how many corpus records it changes, and how it was checked. The parity tests then compare against the reference *plus* the listed differences, so nothing else drifts.
- **"Pronounced reasonably" is checked on audio, not phoneme strings**: synthesize the changed sentences with both versions, compare automatic transcriptions (two ASR models), and have Brendan listen to a sample of what changed.
- **For ports not started yet (zh, ja)**, prefer the simplest implementation that gives the same phonemes, or better-sounding ones, over a line-by-line port. Only invest in exact parity where behaviour is subtle and we have no better idea.

First candidates (my proposals, not decided):
- **One number path.** The normalizer spells out most numbers, money and years before misaki sees them, so misaki's number and currency code (and our num2words port) mostly sees leftovers. Two implementations disagree in places: "$1" → "one dollars", "1900" → "nineteen zero", "5 Ω" drops the number. Keep one number reader, and delete the other.
- **Crashes in the reference**: misaki Japanese asserts on digits in 49 of 1,581 Japanese Wikipedia paragraphs (reference dump `out/wiki-ja.jsonl`, see PR #2). Do whatever reads well instead. The misaki English crashes are already handled; see `divergences.md`.
- **espeak wrapper simplifications** listed in PR #6: phonemizer's punctuation split and restore, misaki's `()`↔`«»` swap, keeping only the first output element, and the `(en)` flags.
- **spaCy's full tagger** is only needed for the tag-dependent rules. If a smaller rule set gives the same audio, it could go, but the port is already exact and fast (0.35 ms per sentence), so this is low priority.

### Licensing

To publish, we need one license for the combined work. Component licenses:

| Component | License |
|---|---|
| misaki code and lexicons, Kokoro weights and voices | Apache-2.0 |
| `kokoro-espeak-ng` (port of espeak-ng), `kokoro-espeak` (wrappers), espeak-ng data | GPL-3.0-or-later |
| num2words (ported, so a derivative) | LGPL-2.1 |
| spaCy `en_core_web_sm` weights | MIT (trained on OntoNotes; Explosion licenses it as MIT) |
| pypinyin data, jieba / jieba-rs, cn2an, cutlet, lindera | MIT |
| UniDic | BSD-3 / GPL / LGPL (pick one) |

All of these are compatible with **GPL-3.0-or-later**, and espeak forces it for anything that links our espeak port. **Decided (Brendan, 2026-11-02):** publish the combined program under GPL-3.0-or-later, and license everything else as permissively as its inputs allow. Concretely:
- Each crate carries its own license.
- Our own code and the crates that don't depend on espeak (the protocol, scheduler, worker, the misaki-English/zh/ja ports, segmentation) are Apache-2.0, matching the repo's current LICENSE.
- The num2words port is LGPL-2.1 (or written from scratch from the spec to stay Apache).
- Only `kokoro-espeak-ng`, `kokoro-espeak`, and what links them (the pipeline and server) are GPL-3.0-or-later.
- `cargo deny` in CI checks that no permissive crate pulls in a GPL or LGPL dependency. The alternative is to drop espeak (losing es/fr/it/pt/hi/ru/el/ko/ar/he and English unknown words) and stay Apache. Not publishing removes the question entirely, since the GPL and LGPL only bind distribution.


### Scheduler / "no waiting"

The worker loop: when the GPU is free, take the highest-priority queued chunks, up to N (about 8 on the 3060 Ti), grouped into length buckets. Never sleep to fill a batch. Priority: chunks a listener is blocked on first, then prefetch, then FIFO. A batch finishes when its longest item does, so a blocked-on chunk goes in a short bucket or runs alone. Stream each chunk's PCM out as soon as it's done. Per-sentence chunking with phonemizing once (KFastAPI#7) and overlap stitching (#8) port over unchanged. Streaming *inside* a sentence is blocked by InstanceNorm, BERT attention and the bidirectional LSTMs, which all see the whole sentence. So it needs a retrained, causal or windowed model, and it isn't in scope.

## Phases (rough)

1. **Worker + protocol (small, about a week).** Day one: check that torch.compile/Triton runs under gVisor+nvproxy on this host's 580.x driver, with a tmpfs or prebuilt kernel cache. Then freeze the current compiled PyTorch model behind a unix-socket integer protocol in gVisor+nvproxy. Benchmark it fed with phonemes from the reference pipeline.
2. **Rust G2P (large; in parallel with 1).** In the order above, starting with the parity harness and English.
3. **Rust front and relay (small–medium).** HTTP/relay, chunking, scheduler with priorities and length-bucketed batching, PCM streaming. Switch production over once every language the current deploy supports, including zh and ja, reaches parity. The new server never hands any language back to Kokoro-FastAPI.
4. **Smaller model (research).** Finish LSTM→conv distillation and maybe a Vocos head. Per-timestep norms make mixed-length batching exact.
5. **Rust runtime (optional).** Port the simplified model to burn or candle, and keep it only if it matches torch.compile speed.

Phase 2 is where the time goes, and the production switch waits for all of it. Phases 4–5 are optional performance work, and their value depends on measurements.

## Open questions

- Which GPU in production: the 3060 Ti only, or cloud? This sets the batch size and the bucket boundaries.
- ~~Do we support generic OpenAI clients, or only Lion Reader and clawed-abode?~~ Only those two, with a strict schema (Brendan, 2026-11-03).

