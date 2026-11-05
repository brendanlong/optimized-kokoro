# GPU worker

Loads Kokoro-82M, listens on a Unix socket, speaks [the worker protocol](../docs/worker-protocol.md), and batches whatever is queued. It runs in gVisor (`runsc`, GPU through nvproxy) with no network and a read-only root. It never sees text, only phoneme ids, and re-validates every frame; any violation closes the connection.

```sh
uv sync
uv run python -m kokoro_worker.convert models/          # once: .pth/.pt -> safetensors (outside the sandbox)
podman build -t localhost/kokoro-worker -f Containerfile .
./run.sh serve /run/user/$UID/kokoro models/           # socket at /run/user/$UID/kokoro/worker.sock
uv run python -m kokoro_worker.client /run/user/$UID/kokoro/worker.sock \
    --config models/config.json --phonemes "həlˈO wˈɜɹld." --out hello.wav
uv run pytest                                             # protocol and connection tests, no GPU
./run.sh probe /tmp/probe models/                         # what the sandbox can reach (tools/sandbox_probe.py)
```

Startup compiles the model (torch.compile, Triton) into a tmpfs; the worker listens only once it's warm, so the front gets its Hello at once. In gVisor that takes 4-6 minutes at `--max-batch 1` and about 10 at `--max-batch 8`, including compiling and autotuning the Triton kernels. To restart faster, seed the caches: `./run.sh export-cache DIR models/ [same flags]` warms up, writes the caches to DIR and exits without ever listening; then `KOKORO_CACHE_SEED=DIR ./run.sh serve ...` mounts them read-only and copies them in (about 1 minute at batch 1). The cache holds pickles and shared objects the worker loads, which is why only a run that never accepts a connection may write it. A seed is only valid for the same image, GPU and driver.

## Sandbox

`run.sh` runs rootless podman with `/usr/bin/runsc`, and needs no root and no podman or docker config:
- one GPU (`KOKORO_GPU`, an index or UUID from `nvidia-smi -L`, default 0), through podman's CDI spec;
- no network, a read-only root, no capabilities, `no-new-privileges`; writable only the socket directory and a 2 GB tmpfs at `/tmp` (mounted `exec`, for Triton);
- runs as the calling user's uid (`--userns=keep-id --user $(id -u):$(id -g)`), so the socket belongs to that user whatever the uid. Checked with uid 2000 inside: the venv, models and socket all work;
- bounded RAM, CPU and host threads through a `systemd-run --user --scope` around podman (`KOKORO_MEMORY_MAX` default 12G, `KOKORO_CPU_QUOTA` 400%, `KOKORO_TASKS_MAX` 1024; in gVisor a batch-8 worker peaked at 5.2 GB of host RAM, and a batch-1 worker under the hostile front at 6.4 GB). runsc runs with `ignore-cgroups` (it can't manage cgroups rootless) and podman refuses runsc with `--cgroups=disabled`, so `--cgroups=split` keeps podman, conmon and the sandbox inside the scope. Checked: a 1 GB allocation under `MemoryMax=300M` kills the sandbox, and 4 busy processes do 4x less work at `CPUQuota=100%` than at 400%. `TasksMax` bounds host threads; gVisor runs guest threads on a few of those, so it doesn't cap guest thread count. GPU memory isn't covered by any of these. `KOKORO_NO_LIMITS=1` skips the scope.

`--max-chunk-seconds` (default 60) is the most audio one request may produce, at any speed, and the most frames in one packed decoder row. It is announced in Hello (protocol v4; under v3 it was scaled by 1/speed). It is the knob for the worker's GPU memory: set it per host (`KOKORO_MAX_CHUNK_SECONDS` in `deploy/settings.env`). A request whose predicted audio is longer gets `Failed` code 3 (too long) before the decoder runs, and the server maps that to an error for the whole request (HTTP 500 `processing_error`, "a chunk is outside the model worker's limits"). To keep that rare, the server splits a chunk whose audio could exceed the cap into overlapping pieces before sending it: it allows 4.7 frames per phoneme id at speed 1 (the most measured on the corpus for 64 ids or more, `bench/frames_per_id.py`, was 4.59), so a request gets at most `cap frames × speed / 4.7` ids, and at most 510 (`crates/server/src/stitch.rs`). At the default 60 s and speed 1 that is 510, so nothing is split that Kokoro-FastAPI wouldn't split; at 30 s it is 255 ids at speed 1 and 63 at speed 0.25. Real 510-phoneme chunks reach about 44 s at speed 1 (`bench/max_duration.py`).

Peak memory grows by about 0.35 MiB per frame (40 frames are one second of audio) on top of about 210 MB, and the CUDA context adds about 175 MB on the card. A packed row of several sentences costs about 15% more than one request of the same total frames. Measured peaks for one request with `bench/chunk_memory.py` on the 3060 Ti (fp16, compiled):

| frames (seconds of audio) | 200 (5) | 800 (20) | 1,200 (30) | 1,600 (40) | 2,400 (60) | 4,800 (120) | 9,600 (240) | 14,400 (360) |
|---|---|---|---|---|---|---|---|---|
| peak GPU memory reserved | 300 MB | 558 MB | 806 MB | 1.0 GB | 1.4 GB | 2.1 GB | 3.9 GB | out of memory |

(The 4,800- and 9,600-frame runs are older, from before packing; the 14,400-frame run had about 5 GB free next to the desktop and another worker.) Under v3 the default 60 s allowed 9,600 frames at speed 0.25. The default stays 60 s, which needs about 1.4 GB; 30 s keeps the worker near 1 GB on the card. MAXFRAMES_MEASURED

## Files

- `kokoro_worker/protocol.py`: framing and validation, no torch.
- `kokoro_worker/scheduler.py`: the queue, most urgent first (priority, higher first; then waiting before prefetch; then arrival), with its slots and cancellation.
- `kokoro_worker/server.py`: the socket, a reader thread per connection, and the GPU loop.
- `kokoro_worker/model.py`: loads the weights and runs a batch. The modules are the kokoro fork's (`brendanlong/kokoro@66c8ed5`, imported without its G2P); the forward pass is ours.
- `kokoro_worker/masked.py`: the decoder's forward pass with padding masked out.
- `kokoro_worker/kernels.py`: the convolutions and STFT, without per-shape plans (see [Kernels](#kernels)).
- `kokoro_worker/convert.py`, `kokoro_worker/client.py`: one-time converter and a test client.
- `bench/`: equivalence checks, load generator, listening pairs, and the gpuc jobs that ran them; `e2e.sh` runs the Rust server against the sandboxed worker.
- `tools/`: `hostile.py`, a hostile front (malformed, extreme and stalled frames, slot misuse, 300 mutations), and `sandbox_probe.py`, both from the PR #11 review; `fuzz_front.py`, coverage-guided fuzzing of the frame decoder and slot table with atheris (`uv run --with atheris python tools/fuzz_front.py CORPUS -max_total_time=600`).


## Batching

`--max-batch` defaults to 1. On the RTX 3060 Ti with real sentences in gVisor, `--max-batch 8` gives 173x at 16 concurrent requests against 161x at batch 1, and about the same single-request latency (36 vs 34 ms median at 1 stream). Single requests used to be slower at batch 8 (44 vs 37 ms); that was first-time shapes (see [Kernels](#kernels)). Use 8 when many listeners share the worker.

When the GPU is free the worker takes up to `--max-batch` queued requests, most urgent first, and never waits for more. The text side (ALBERT, text encoder, duration predictor) runs on the whole batch with attention masks, zeroed padding and packed LSTMs, so durations match a batch of one (exactly in fp32; in fp16 about one sentence in 16 gets one duration a frame off). Then the decoder runs on the batch's sentences packed end to end into one row, with two frames of zeros between them (`masked.py`), in rows of at most the frame budget, most urgent first; each row's audio goes out as soon as it's done.

Packing replaces grouping by length, which split a batch of real sentences into 4-6 decoder runs on the H100. The convolutions run once over the row, and none reaches across a gap, which is zeroed again before each one. What isn't local is done per sentence: the instance norm statistics and the style, the harmonic source's phase (summed in float64, restarting at each sentence, and upsampled in the sentence's own positions). At a sentence's edges the STFTs see the gap's zeros instead of a row's reflect padding, which the generator spreads over a few hundred samples of mostly near-silence; the spectrograms were the same with and without per-sentence edge handling (0.99772 mean, 0.99455 worst, against 0.99772 and 0.99457). With the noise off, a packed sentence's spectrogram correlates 0.998 with the sentence alone (0.995 worst, fp32; `bench/check_packing.py`). Samples still differ: the generator magnifies rounding in the harmonic phase, the same rounding batch 1 has. In fp16 any batch differs from batch 1 a little more than seed noise (0.99 vs 0.996), as it did with grouping; that's fp16 numerics depending on batch size, and copies of one sentence show the same.

On the 3060 Ti, with main's worker and this one up at once and load alternating between them (`bench/job_packing_ab.sh`, `--max-batch 8`), packing gave 209-234x at 16 and 64 streams over three rounds against 189-213x for main (232-234x against 210-213x in the last two), and 377 vs 418 ms median latency at 16 streams. (That was before the segment sums moved to float64; with them, one more A/B gave 207-218x against 191-211x, on a GPU busier with other work.) In process, on the same batches, it is 7-10% faster at batches of 8-16, about the same or faster at 32, and 4% slower at 2. Building a row's masks and indexes costs about 4 ms per batch.

Batch 1 and batches above 1 compile into separate graphs, because a graph compiled for a dynamic batch also accepts batch 1 (no measured speed difference either way).

### Large batches (H100)

Measured on an H100 SXM 80 GB (RunPod, no gVisor) with `bench/job_scale.sh`; the in-process numbers are `bench/batch_scale.py`, seconds of audio per wall second:

| Batch | 1 | 8 | 32 | 64 | 128 | 512 |
|---|---|---|---|---|---|---|
| B copies of a 100-phoneme chunk | 206x | 1,010x | 1,667x | 1,878x | 1,891x | 1,884x |
| B corpus chunks, grouped by length (frame budget 75,000) | 205x | 603x | 937x | 1,120x | 1,257x | 1,342x |
| The worker over its socket, `--max-batch` B, 64-128 streams | | 450x | 730-770x | 970x | | |

- **The model's ceiling is about 1,850-1,900x** (measured before packing, when equal-length batches skipped the masks), at least 6x the 3060 Ti's (300x at B=16, the largest tried there). It's reached by batch 64 for 100- and 250-phoneme chunks, and by 128 for 30-phoneme ones (1,834x). 512 copies of a 100-phoneme chunk peak at 43 GiB allocated.
- **Below the ceiling, the GPU waits on the host.** In profiles of corpus batches, GPU busy time was 20% of wall time at batch 1, 50% at 8, 72% at 64 and 90% at 256. Those runs had CPU tracing on, which slows the host, so the shares are low by an unknown amount; the script now measures against unprofiled time. The text side takes 8-28 ms per batch for equal lengths, about the same at any batch size up to 128. For corpus batches it grows: 13 ms at 1, 35 ms at 64, 176 ms at 512.
- **Mixed lengths cost the most in practice**, with the length groups that packing has since replaced (see above; packing is not yet measured on the H100). At 25% padding, a batch of corpus chunks splits into 4-6 decoder groups, which run one after another. That holds batch 64 to 60% of the equal-length rate. (The masked and unmasked runs here used different chunks, so this job doesn't measure what the masks cost; `batch_scale.py` now reuses the chunks.)
- **The worker over its socket gets less again**, 970x at `--max-batch 64`. Not measured, but plausible causes:
  - Closed-loop streams fill batches only partly.
  - The worker's 9,600-frame group budget (2,400 since protocol v4, the chunk cap at any speed) is smaller than the 75,000 used in process.
  - The next batch's text side waits for this batch's decoder.
- **The Triton conv1d (`_conv1d_kernel`) is about half of GPU time at large batches**: 48% at batch 64, 52% at 256.
  - Against cuDNN at batch 64 (`bench/conv_peak.py`, one call of each decoder layer shape), it's 19% slower in total, 25.8 vs 21.7 ms. On the two 1024-channel layers it's 3x slower (111 vs 330-350 TFLOP/s). On the 128-channel k3 and k7 layers at 48,000 steps it's faster, but slower on k11.
  - Even matching cuDNN everywhere would save at most about 8% at saturation.
  - At 1,878x the model does about 100 TFLOP/s: 1,878 × 27.6 GMAC (the per-layer count of multiply-adds per second of audio) × 2 FLOP per multiply-add. That is about a tenth of the card's dense fp16 peak.
- **One call is limited to 2^31 elements**, by the kernel's int32 offsets. That's about 75,000 frames in one group: 128 copies of a 250-phoneme chunk (74,880 frames) ran, 256 failed. The worker's own frame budget is the chunk cap in frames, 2,400 by default.

## Kernels

cuDNN builds an execution plan the first time it sees each convolution shape (about 90 convolutions per request, ~200 µs each to plan), and cuFFT a plan per STFT size. Since a request's frame count depends on its predicted durations, most requests were the first of their size: 71 ms median to the result over the socket against 33 ms for the same request repeated. On a busy host, plan building, which is CPU work, made it worse. `kernels.py` replaces them:

- Conv1d is a Triton implicit-GEMM kernel, compiled once per layer configuration with the length as a runtime argument. The tile is autotuned per layer, not per length, during warmup, and the timings go to the Triton cache. Lengths, the strides that scale with them and the input's alignment are passed unspecialized, because Triton otherwise compiles a variant per integer divisible by 16 or pointer alignment, a 2 s stall. After warmup, 300 random requests (batch 1 and 8, any length and speed) built no new variants (`bench/recompile.py`).
- ConvTranspose1d is one cuBLAS matmul plus overlap-add.
- The STFT and iSTFT are matmuls with a DFT matrix.

Layer by layer, the Triton conv1d matches cuDNN's repeat time (4.1 ms summed over 12 decoder layers) and costs the same on a fresh length (cuDNN: 31 ms). Nothing in the model builds per-shape plans now: the profile shows no cuDNN convolution or FFT calls, and cuBLAS and the cuDNN LSTMs showed no first-time cost. Torch's own Triton convolution templates can't do this in torch 2.8: they have no conv1d, no dilation and no transposed convolutions. Doing the convolutions as cuBLAS GEMMs (im2col or shift-and-add) was 2-2.6x slower.

Measured in gVisor (`bench/job_kernels.sh`), against main's worker in the same job:
- Latency, 200 fresh real chunks each sent twice at batch 1: the first send went from 71 to 30 ms median, the repeat from 33 to 30 ms. Through the HTTP server, time to first byte went from 47 to 29 ms on first sends and from 34 to 29 ms on repeats.
- Throughput at batch 1, with both workers up and load alternating between them over 3 rounds: 159-169x against 122-134x at 1 stream, and 156-161x against 122-129x at 16.
- Audio over 650 sentences: durations identical. Spectrogram correlation with cuDNN is 0.993 mean, 0.985 min, against a seed-to-seed floor of 0.996 and 0.993. Whisper small.en on 200 sentences: 2.10% WER against 2.13%. Batched vs alone is as before (0.99, one sentence in 16 with a duration a frame off).
- Memory: GPU reserved 590-594 MiB at batch 1 (cuDNN 468-552), about 765 MiB on the card with the CUDA context; RSS 6.2 GB (6.5-6.8 GB). Warmup peak is the same as steady state.
- Startup: compiling and autotuning the Triton kernels adds to warmup. Cold, at batch 1, it takes 4-6 minutes against 3; at batch 8, 10 against 7. From a seeded cache it takes 63 s against 51 s, with the seed at 154 MB against 57 MB. The autotune timings are cached in the seed too.

`bench/reference.py` swaps cuDNN and cuFFT back in, for comparisons only. `bench/kernels_correct.py` checks the conv1d against `F.conv1d` for every layer and tile configuration at edge lengths, with guard bands; `tests/test_kernels.py` checks the transposed convolution and STFT on CPU.

