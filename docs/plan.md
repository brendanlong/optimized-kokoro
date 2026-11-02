# Optimized Kokoro: feasibility and plan (draft)

Goal (from Brendan): a Kokoro TTS server we can host without worrying about C exploits or slow Python. It should do no duplicate work, run optimized graphs, batch whatever is already queued without waiting, avoid C and risky dependencies, and stream audio as fast as possible.

This is a proposal, not decided. Numbers are from the measurements in the wiki (`kokoro-tts-local-vs-gpu`, `self-hosted-kokoro-exposure-options`, journals `2026-10-31/11-01--kokoro-fastapi-latency-profile*`, `2026-11-01--kokoro-architecture-retrain-ideas`), all on the RTX 3060 Ti.

## Short answer

It's feasible. Two of the goals need restating, though:

1. **"No C at all" can't hold on a GPU.** The NVIDIA driver, `libcuda`, and cuBLAS/cuDNN or NVRTC are closed C/C++ whatever framework we pick. The goal we can meet: **no C or Python touches attacker-controlled bytes.** Text, HTTP and G2P run in memory-safe Rust. The GPU process only ever receives arrays of small integers that Rust has already range-checked.
2. **Batching probably won't help on this GPU.** Batching the decoder gave no per-item speedup on the 3060 Ti, because one sentence already keeps the GPU busy. Kokoro's InstanceNorm also normalizes over the whole utterance, so padding a batch changes the audio (correlation 0.964, versus a 0.995 noise floor). A scheduler that "takes up to N queued requests" is cheap to build. But **queue order** (do the chunk a listener is waiting on before prefetches) will matter more for latency than batch size, and the size should come from measurements on whatever GPU we deploy to.

What's large: **G2P (turning text into phonemes) in Rust, at parity with misaki.** Everything else is small or already done.

## Proposed architecture

```
client ──► [Fly relay, Rust]  ◄── outbound connection ── [desktop: front, Rust] ──unix socket──► [model worker]
           (public; auth;                                  text normalize, G2P,     fixed binary     gVisor + nvproxy,
            OpenAI-ish route)                              chunking, scheduler,     frames           GPU, no network
                                                           PCM streaming
```

- **Front (Rust, sandboxed, no GPU).** Owns every byte from the network: request parsing, text normalization, sentence chunking, G2P, the voice table, the scheduler and the PCM stream back. It sends the worker `{request_id, voice_index, speed, phoneme_ids: [u8; ≤510]}` and gets back `{request_id, seq, pcm: [i16]}` frames.
- **Model worker.** Only reads length-prefixed integer frames. Weights load from safetensors (no pickle). It runs in gVisor `runsc --nvproxy`, which forwards only the NVIDIA ioctls on its list for that driver version. That list is probably the "whitelist of allowed GPU commands" you remembered. It has no network and a read-only filesystem.
- **Desktop listens on nothing.** This matches the relay design already chosen in `self-hosted-kokoro-exposure-options`.

### Protocol question (OpenAI-compatible HTTP vs MessagePack-RPC)

Most of the risk was never in the HTTP format. It was in the C and Python parsers behind it, and G2P (espeak-ng is C, spaCy is Cython, MeCab is C++). With Rust on both ends:

- **Between the relay and the desktop:** a fixed binary frame over one outbound TLS stream. Use length-prefixed `bincode`/hand-rolled structs with hard size caps. MessagePack adds nothing over that.
- **Public edge (relay):** one route, `POST /v1/audio/speech`, through `hyper`/`axum`, with a body-size cap and `serde(deny_unknown_fields)`. It returns `audio/pcm` (24 kHz s16le). That's a small, well-fuzzed surface, and existing clients (Lion Reader, clawed-abode) keep working. I don't think something narrower is worth the cost of breaking compatibility.

### Audio

Return raw PCM. No mature pure-Rust Opus or AAC encoder exists (the `opus` crate wraps libopus, which is C). So encoding to fMP4/Opus/AAC happens wherever the client side already does it, either in the browser with WebCodecs or in Lion Reader. 24 kHz mono s16 is 384 kbit/s, which is fine.

### Model runtime: PyTorch first, Rust later, ideally after a smaller model

| Option | Speed we know | Risk |
|---|---|---|
| **PyTorch + torch.compile in the worker** | 157× real time, 44 ms to first audio, ~500 MB VRAM (deployed today) | Large C++ dependency, but it only ever sees integer arrays inside gVisor. Startup ~50 s for the compile. |
| Rust: burn (CubeCL CUDA, fuses ops automatically) | unknown | Speed on Kokoro's conv-heavy decoder is untested, plus LSTMs, ConvTranspose1d, STFT. Still needs the NVIDIA driver and NVRTC. |
| Rust: candle (cudarc + cuBLAS/cuDNN) | unknown; Crane's Kokoro uses ONNX Runtime (C++), not candle | Same gaps. Its LSTM is a slow per-timestep loop. |
| ONNX Runtime (Kokoros, Crane) | 2–5× *slower* than torch eager on CUDA for us | C++, so it fails goal 4 anyway. |

I recommend **PyTorch in the sandboxed worker first**, behind the integer-only protocol. It's the fast path we've already measured, and it moves C/Python off the untrusted path. A Rust port becomes much easier and more worthwhile **after** the distillation work (PR kokoro#3): replace the LSTMs with convs (already 36 ms vs 50 ms, but the text encoder's LSTM costs WER), then possibly a Vocos-style head. That turns the model into plain convs, norms and one iSTFT, which any runtime does well. The profile also says the remaining headroom is in fusing elementwise ops: 47 of 75 ms is elementwise, roughly 3× headroom. That's an argument for burn's fusion or hand-written kernels, but only once we've measured one.

### G2P: the big piece

misaki English is: lexicon lookup (gold/silver dictionaries), spaCy POS tags for heteronyms ("read", "lead"), number/currency/abbreviation rules, and espeak for unknown words. Plus our script segmentation (kokoro#2) and Mandarin-accent (kokoro#6) work.

Rust plan:
- Lexicons and rules: a direct port.
- POS tagging: a small pure-Rust tagger (averaged perceptron or a tiny CNN) trained on UD English. It only has to decide heteronyms, not do general tagging.
- Unknown words: either **espeak-ng-rs** (pure Rust and claims bit-identical IPA, but it's 7 months old with 27★ and **GPL-3.0**, which is fine for hosting and a problem for distribution), or a small character-level model distilled from misaki+espeak output, as Crane/Moonshine do.
- Chinese: `jieba-rs` plus a pinyin crate plus a port of misaki's pinyin→IPA tables. Japanese: `lindera` (pure Rust, UniDic). Other scripts: espeak-ng-rs.
- **Testing is easy to make objective:** run Python misaki (our fork) and the Rust G2P over a large corpus and diff the phoneme strings. Count mismatches per category and listen only to the ones that differ. This turns "is the port good" into a number.

Until the Rust G2P reaches parity, the stepping stone is to run Python misaki inside the worker's sandbox. That's no worse than today, and it already removes the Python/C HTTP stack.

### Scheduler / "no waiting"

The worker loop: when the GPU is free, take the highest-priority queued chunks, up to N. Never sleep to fill a batch. Priority: chunks a listener is blocked on first, then prefetch, then FIFO. Stream each chunk's PCM out as soon as it's done. Per-sentence chunking with phonemizing once (KFastAPI#7) and overlap stitching (#8) port over unchanged. Streaming *inside* a sentence is blocked by InstanceNorm, so it waits for a retrained model with per-timestep norms.

## Phases (rough)

1. **Worker + protocol (small, about a week).** Freeze the current compiled PyTorch model behind a unix-socket integer protocol in gVisor+nvproxy. Python G2P runs inside the sandbox for now. Benchmark against Kokoro-FastAPI.
2. **Rust front and relay (small–medium).** HTTP/relay, chunking, scheduler with priorities, PCM streaming. Replace Kokoro-FastAPI in production.
3. **Rust G2P (large, open-ended).** English to parity on the diff harness first, then zh/ja, then espeak-ng-rs for the rest. Move G2P out of the worker sandbox.
4. **Smaller model (research).** Finish LSTM→conv distillation and maybe a Vocos head; per-timestep norm enables real batching and streaming within a sentence.
5. **Rust runtime (optional).** Port the simplified model to burn or candle, and keep it only if it matches torch.compile speed.

Phases 1–2 give the security win quickly. Phase 3 is where the time goes. Phases 4–5 are optional performance work, and their value depends on measurements.

## Open questions

- Which GPU in production: the 3060 Ti only, or cloud? This decides whether batching is worth anything.
- Is GPL-3 espeak-ng-rs acceptable, given we only host and don't distribute?
- Does anything need OpenAI-compatible output besides Lion Reader and clawed-abode?
