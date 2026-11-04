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

`--max-chunk-seconds` (default 60) is the most audio one request may produce at speed 1. It is scaled by 1/speed and announced in Hello. A longer request gets `Failed` code 3 (too long) before the decoder runs. The server maps that to an error for the whole request (HTTP 500 `processing_error`, "a chunk is outside the model worker's limits"); it doesn't split the chunk. Real 510-phoneme chunks reach about 44 s at speed 1 (`bench/max_duration.py`), so the cap only binds on pathological input.

The cap is set by GPU memory, and decoder batches are held to the same number of frames. Peak memory grows by about 0.37 MiB per frame (40 frames are one second of audio). Measured peaks with `bench/chunk_memory.py` on the 3060 Ti:

| frames (seconds of audio) | 2,400 (60) | 4,800 (120) | 9,600 (240) | 12,000 (300) | 14,400 (360) |
|---|---|---|---|---|---|
| peak GPU memory reserved | 1.2 GB | 2.1 GB | 3.9 GB | 4.8 GB | out of memory |

The 14,400-frame run had about 5 GB free next to the desktop and another worker. At 60 s, the worst case is a 240 s chunk at speed 0.25 (9,600 frames), which fits. The old 120 s default allowed 19,200 frames, which did not fit.

## Files

- `kokoro_worker/protocol.py`: framing and validation, no torch.
- `kokoro_worker/scheduler.py`: the queue, by (priority, arrival), with its slots and cancellation.
- `kokoro_worker/server.py`: the socket, a reader thread per connection, and the GPU loop.
- `kokoro_worker/model.py`: loads the weights and runs a batch. The modules are the kokoro fork's (`brendanlong/kokoro@66c8ed5`, imported without its G2P); the forward pass is ours.
- `kokoro_worker/masked.py`: the decoder's forward pass with padding masked out.
- `kokoro_worker/kernels.py`: the convolutions and STFT, without per-shape plans (see [Kernels](#kernels)).
- `kokoro_worker/convert.py`, `kokoro_worker/client.py`: one-time converter and a test client.
- `bench/`: equivalence checks, load generator, listening pairs, and the gpuc jobs that ran them; `e2e.sh` runs the Rust server against the sandboxed worker.
- `tools/`: `hostile.py`, a hostile front (malformed, extreme and stalled frames, slot misuse, 300 mutations), and `sandbox_probe.py`, both from the PR #11 review; `fuzz_front.py`, coverage-guided fuzzing of the frame decoder and slot table with atheris (`uv run --with atheris python tools/fuzz_front.py CORPUS -max_total_time=600`).


## Batching

`--max-batch` defaults to 1. On the RTX 3060 Ti with real sentences in gVisor, `--max-batch 8` gives 173x at 16 concurrent requests against 161x at batch 1, and about the same single-request latency (36 vs 34 ms median at 1 stream). Single requests used to be slower at batch 8 (44 vs 37 ms); that was first-time shapes (see [Kernels](#kernels)). Use 8 when many listeners share the worker.

When the GPU is free the worker takes up to `--max-batch` queued requests, most urgent first, and never waits for more. The text side (ALBERT, text encoder, duration predictor) runs on the whole batch with attention masks, zeroed padding and packed LSTMs, so durations match a batch of one (exactly in fp32; in fp16 about one sentence in 16 gets one duration a frame off). Then the decoder runs in groups of similar output length (`--pad-slack`, default 25% padding at most), most urgent group first, and each group's audio goes out as soon as it's done.

Padding used to change the audio (spectrogram correlation about 0.95 against a 0.996 noise floor): the instance norms took statistics over the padding, and padded frames leaked into real ones through the convolutions. `masked.py` takes the norm statistics over each row's real frames and zeroes padded frames before every convolution. Padded audio then correlates with unpadded at the noise floor (fp32). In fp16, any batch, padded or not, differs from batch 1 a little more than seed noise (0.99 vs 0.996), and about one sentence in 16 gets one duration a frame off; that's fp16 numerics depending on batch size. `--no-exact` turns the masks off, which is about 10% faster and changes the audio (heavily padded rows come out about twice as loud).

Batch 1 and batches above 1 compile into separate graphs, because a graph compiled for a dynamic batch also accepts batch 1 (no measured speed difference either way).

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

