# Optimized Kokoro: feasibility and plan (draft)

Goal (from Brendan): a Kokoro TTS server we can host without worrying about C exploits or slow Python. It should do no duplicate work, run optimized graphs, batch whatever is already queued without waiting, avoid C and risky dependencies, and stream audio as fast as possible.

This is a proposal, not decided. Numbers are from the measurements in the wiki (`kokoro-tts-local-vs-gpu`, `self-hosted-kokoro-exposure-options`, journals `2026-10-31/11-01--kokoro-fastapi-latency-profile*`, `2026-11-01--kokoro-architecture-retrain-ideas`), all on the RTX 3060 Ti.

## Short answer

It's feasible. Two of the goals need restating, though:

1. **"No C at all" can't hold on a GPU.** The NVIDIA driver, `libcuda`, and cuBLAS/cuDNN or NVRTC are closed C/C++ whatever framework we pick. The goal we can meet: **no C or Python touches attacker-controlled bytes.** Text, HTTP and G2P run in memory-safe Rust. The GPU process only ever receives arrays of small integers that Rust has already range-checked.
2. **Batching helps, but it needs length buckets and masking.** A whole-model test of equal-length batches on the 3060 Ti (3c3e review, 2026-11-02) measured these speedups:
   - 50-token sentences: 127× real time at batch size 1, 296× at batch size 8;
   - 150-token sentences: 201× at batch size 1, 328× at batch size 8.

   The card tops out around 330–350× real time. Typical sentences are short (median 81 phonemes), and short ones gain the most. An earlier decoder-only test at 470 frames found no gain, which fits: long inputs already fill the GPU. Mixed lengths are the catch. InstanceNorm/AdaIN and the bidirectional LSTMs read the padding, and padding changed the audio (spectrogram correlation 0.946–0.975 vs unpadded, noise floor ~0.99, in journal `2026-11-01--kokoro-fastapi-latency-profile-cont`). So batching needs short length buckets plus masked norms and LSTMs. That's work, not a blocker.

What's large: **G2P (turning text into phonemes) in Rust, at parity with misaki.** Everything else is small or already done.

## Proposed architecture

```
client ──► [Fly relay, Rust]  ◄── outbound connection ── [desktop: front, Rust] ──unix socket──► [model worker]
           (public; auth;                                  text normalize, G2P,     fixed binary     gVisor + nvproxy,
            OpenAI-ish route)                              chunking, scheduler,     frames           GPU, no network
                                                           PCM streaming
```

- **Front (Rust, sandboxed, no GPU).** Owns every byte from the network: request parsing, text normalization, sentence chunking, G2P, the voice table, the scheduler and the PCM stream back. It sends the worker `{request_id, voice_index, speed, phoneme_ids: [u8; ≤510]}` and gets back `{request_id, seq, pcm: [i16]}` frames.
- **Model worker.** Only reads length-prefixed integer frames, and **re-validates them itself**. It doesn't trust the front: every phoneme id must be in the vocab, the voice index in range, the length ≤ 510, and speed finite and within bounds. Weights load from safetensors (no pickle). It runs in gVisor `runsc --nvproxy`, which forwards only the NVIDIA ioctls on its list for that driver version. That list is probably the "whitelist of allowed GPU commands" you remembered. The arguments to those ioctls are still forwarded, though, so the sandbox doesn't make the worker safe to compromise. It has no network and a read-only filesystem. nvproxy supports exact driver versions only, so pin the host driver and runsc together.
- **Desktop listens on nothing.** This matches the relay design already chosen in `self-hosted-kokoro-exposure-options`. The relay and agent authenticate each other (mutual TLS or a pinned key). A stolen agent credential would receive every listener's text, so store it outside the `claude` user.

### Protocol question (OpenAI-compatible HTTP vs MessagePack-RPC)

Most of the risk was never in the HTTP format. It was in the C and Python parsers behind it, and G2P (espeak-ng is C, spaCy is Cython, MeCab is C++). With Rust on both ends:

- **Between the relay and the desktop:** a fixed binary frame over one outbound TLS stream. Use length-prefixed `postcard`/hand-rolled structs. Check each length prefix against a hard cap *before* allocating. MessagePack adds nothing over that. The agent's hello frame carries the voice list.
- **Relay API:** two routes, `POST /v1/audio/speech` and `GET /v1/audio/voices`, which Lion Reader's `selfhosted.ts` also calls. They go through `hyper`/`axum` with a body-size cap. `/v1/audio/speech` returns `audio/pcm` (24 kHz s16le), with no encoding. The API stays on Fly's private network, so only the agent's connection endpoint is public, and auth is checked before reading a body or accepting an upgrade. That's a small, well-fuzzed surface, and Lion Reader and clawed-abode keep working. Whether to reject unknown fields (`deny_unknown_fields`) depends on which clients we support. Generic OpenAI clients send extras like `model` and `response_format`, so the choice is either to accept and ignore a fixed list of them, or to support only our own clients.

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
- **espeak covers six languages** (ru/el/ko/ar/he/hi) as well as unknown English words. So a distilled character model would only replace the English fallback; **espeak-ng-rs** is needed regardless. Pin the reference espeak version, since misaki's `E2M` rewrites depend on it. Port our own post-processing too: strip Hebrew vowel points, map `ħ→h` and `ʕ→ʔ`, and treat Greek as Greek only from 3 letters. Licensing doesn't change: the current deploy already ships GPL-3 espeak-ng.
- **Chinese.** The v1.0 model uses misaki's **legacy** path (jieba + pypinyin `lazy_pinyin` + `transcription.py`), not `ZHFrontend`. Polyphones (行, 长) depend on pypinyin's phrase dictionary, and segmentation depends on jieba's dictionary version. So the Rust side has to use the same dictionary data, not whatever a pinyin crate ships. Also test which characters have no reading (they fall back to English), and the Mandarin-accent rules (kokoro#6).
- **Japanese** is cutlet: MeCab with full UniDic, then romanization rules, then misaki's kana tables. `lindera` only replaces the MeCab step, so this is about as much work as zh.
- **Segmentation** (`foreign.py`, ~140 lines): port `tests/test_foreign.py` one-to-one and use it as the spec.

**Parity harness.** Diff per-chunk phonemes from our `KPipeline` (`en_g2p_tokens`/`g2p_phonemes`, pinned to a commit), not from misaki alone, because segmentation, chunking and post-processing live outside misaki. Corpus: real Lion Reader articles plus the mixed-script sets from kokoro#2. Weight the diffs, since many are inaudible (primary vs secondary stress, `ᵊ`). For the rest, use blind transcription by two audio models as a cheap second check.

**Order:** harness and corpus → English tokenizer and lexicon (measure the match rate) → tagger → segmentation + espeak-ng-rs (one dependency covers six languages plus the English fallback) → zh (legacy path) → ja last (largest dependency, least traffic).

Until the Rust G2P reaches parity, the stepping stone is to run Python G2P in its **own** no-GPU gVisor sandbox. It must not run in the GPU worker, so the worker only ever receives integers. That sandbox answers `text → phoneme ids` over the same kind of frame protocol. It's no worse than today, and it already removes the Python/C HTTP stack.

### Scheduler / "no waiting"

The worker loop: when the GPU is free, take the highest-priority queued chunks, up to N (about 8 on the 3060 Ti), grouped into length buckets. Never sleep to fill a batch. Priority: chunks a listener is blocked on first, then prefetch, then FIFO. A batch finishes when its longest item does, so a blocked-on chunk goes in a short bucket or runs alone. Stream each chunk's PCM out as soon as it's done. Per-sentence chunking with phonemizing once (KFastAPI#7) and overlap stitching (#8) port over unchanged. Streaming *inside* a sentence is blocked by InstanceNorm, BERT attention and the bidirectional LSTMs, which all see the whole sentence. So it needs a retrained, causal or windowed model, and it isn't in scope.

## Phases (rough)

1. **Worker + protocol (small, about a week).** Day one: check that torch.compile/Triton runs under gVisor+nvproxy on this host's 580.x driver, with a tmpfs or prebuilt kernel cache. Then freeze the current compiled PyTorch model behind a unix-socket integer protocol in gVisor+nvproxy. Python G2P runs in a separate no-GPU gVisor sandbox for now. Benchmark against Kokoro-FastAPI.
2. **Rust front and relay (small–medium).** HTTP/relay, chunking, scheduler with priorities and length-bucketed batching (masked norms/LSTMs in the worker), PCM streaming. Replace Kokoro-FastAPI in production.
3. **Rust G2P (large, open-ended).** In the order above. Move G2P out of the worker sandbox language by language as each one reaches parity.
4. **Smaller model (research).** Finish LSTM→conv distillation and maybe a Vocos head. Per-timestep norms make mixed-length batching exact.
5. **Rust runtime (optional).** Port the simplified model to burn or candle, and keep it only if it matches torch.compile speed.

Phases 1–2 give the security win quickly. Phase 3 is where the time goes. Phases 4–5 are optional performance work, and their value depends on measurements.

## Open questions

- Which GPU in production: the 3060 Ti only, or cloud? This sets the batch size and the bucket boundaries.
- Do we support generic OpenAI clients (accept and ignore their extra fields), or only Lion Reader and clawed-abode (strict schema)?
