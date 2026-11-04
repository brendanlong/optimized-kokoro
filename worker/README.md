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

Startup compiles the model (torch.compile, Triton) into a tmpfs; the worker listens only once it's warm, so the front gets its Hello at once. It also runs every size bucket once (see [Size buckets](#size-buckets)). In gVisor that takes about 3 minutes at `--max-batch 1` and 7 at `--max-batch 8`. To restart faster, seed the caches: `./run.sh export-cache DIR models/ [same flags]` warms up, writes the caches to DIR and exits without ever listening; then `KOKORO_CACHE_SEED=DIR ./run.sh serve ...` mounts them read-only and copies them in (about 1 minute at batch 1, with or without buckets). The cache holds pickles and shared objects the worker loads, which is why only a run that never accepts a connection may write it. A seed is only valid for the same image, GPU and driver.

## Sandbox

`run.sh` runs rootless podman with `/usr/bin/runsc`, and needs no root and no podman or docker config:
- one GPU (`KOKORO_GPU`, an index or UUID from `nvidia-smi -L`, default 0), through podman's CDI spec;
- no network, a read-only root, no capabilities, `no-new-privileges`; writable only the socket directory and a 2 GB tmpfs at `/tmp` (mounted `exec`, for Triton);
- runs as the calling user's uid (`--userns=keep-id --user $(id -u):$(id -g)`), so the socket belongs to that user whatever the uid. Checked with uid 2000 inside: the venv, models and socket all work;
- bounded RAM, CPU and host threads through a `systemd-run --user --scope` around podman (`KOKORO_MEMORY_MAX` default 12G, `KOKORO_CPU_QUOTA` 400%, `KOKORO_TASKS_MAX` 1024; in gVisor a batch-8 worker peaked at 5.2 GB of host RAM, and a batch-1 worker under the hostile front at 6.4 GB). runsc runs with `ignore-cgroups` (it can't manage cgroups rootless) and podman refuses runsc with `--cgroups=disabled`, so `--cgroups=split` keeps podman, conmon and the sandbox inside the scope. Checked: a 1 GB allocation under `MemoryMax=300M` kills the sandbox, and 4 busy processes do 4x less work at `CPUQuota=100%` than at 400%. `TasksMax` bounds host threads; gVisor runs guest threads on a few of those, so it doesn't cap guest thread count. GPU memory isn't covered by any of these. `KOKORO_NO_LIMITS=1` skips the scope.

`--max-chunk-seconds` (default 120) is the most audio one request may produce at speed 1, scaled by 1/speed and announced in Hello; longer requests get `Failed` code 3 before the decoder runs. Real 510-phoneme chunks reach about 44 s (`bench/max_duration.py`).

## Files

- `kokoro_worker/protocol.py`: framing and validation, no torch.
- `kokoro_worker/scheduler.py`: the queue, by (priority, arrival), with its slots and cancellation.
- `kokoro_worker/server.py`: the socket, a reader thread per connection, and the GPU loop.
- `kokoro_worker/model.py`: loads the weights and runs a batch. The modules are the kokoro fork's (`brendanlong/kokoro@66c8ed5`, imported without its G2P); the forward pass is ours.
- `kokoro_worker/masked.py`: the decoder's forward pass with padding masked out.
- `kokoro_worker/convert.py`, `kokoro_worker/client.py`: one-time converter and a test client.
- `bench/`: equivalence checks, load generator, listening pairs, and the gpuc jobs that ran them; `e2e.sh` runs the Rust server against the sandboxed worker.
- `tools/`: `hostile.py`, a hostile front (malformed, extreme and stalled frames, slot misuse, 300 mutations), and `sandbox_probe.py`, both from the PR #11 review; `fuzz_front.py`, coverage-guided fuzzing of the frame decoder and slot table with atheris (`uv run --with atheris python tools/fuzz_front.py CORPUS -max_total_time=600`).


## Batching

`--max-batch` defaults to 1. On the RTX 3060 Ti with real sentences, `--max-batch 8` gives about +39% throughput at 16 concurrent requests (181x vs 130x) and the same at 4. Single requests used to be slower at batch 8 (50 vs 38 ms median in gVisor). With size buckets they take the same time (39 vs 42 ms), so the gap was probably first-time shapes. Use 8 when many listeners share the worker.

When the GPU is free the worker takes up to `--max-batch` queued requests, most urgent first, and never waits for more. The text side (ALBERT, text encoder, duration predictor) runs on the whole batch with attention masks, zeroed padding and packed LSTMs, so durations match a batch of one (exactly in fp32; in fp16 about one sentence in 16 gets one duration a frame off). Then the decoder runs in groups of similar output length (`--pad-slack`, default 25% padding at most), most urgent group first, and each group's audio goes out as soon as it's done.

Padding used to change the audio (spectrogram correlation about 0.95 against a 0.996 noise floor): the instance norms took statistics over the padding, and padded frames leaked into real ones through the convolutions. `masked.py` takes the norm statistics over each row's real frames and zeroes padded frames before every convolution. Padded audio then correlates with unpadded at the noise floor (fp32). In fp16, any batch, padded or not, differs from batch 1 a little more than seed noise (0.99 vs 0.996), and about one sentence in 16 gets one duration a frame off; that's fp16 numerics depending on batch size. `--no-exact` turns the masks off, which is about 10% faster and changes the audio (heavily padded rows come out about twice as loud).

Batch 1 and batches above 1 compile into separate graphs, because a graph compiled for a dynamic batch also accepts batch 1 (no measured speed difference either way).

## Size buckets

Every request runs at one of a fixed set of shapes, all run once before the worker listens, so no request is the first of its size. Without this, about every request was: a new frame count cost about 35 ms more than a repeated one (78 vs 42 ms in-process for a typical sentence at batch 1), because cuDNN builds an execution plan the first time it sees each convolution shape (about 90 convolutions per request, ~200 µs each) and cuFFT one per STFT size. No dynamo recompiles, Triton autotuning or allocator growth were involved, and `TORCH_CUDNN_V8_API_DISABLED=1` (the legacy cuDNN API) didn't help. Over 1,000 real requests, exact sizes ran 383 decoder shapes and built 15,353 distinct convolution plans and 766 cuFFT plans; with buckets, 42 decoder shapes, 1,859 convolutions (576 of them the text side's) and 84 cuFFT plans, all from the warmed set (`bench/shape_count.py`).

- Decoder frames pad to buckets `--bucket-ratio` apart (default 0.05: 32, 40, ... in steps of 8, then 5% apart; 3.4% padding on average), always through the masked decoder, so padding doesn't change the audio. Requests that span more frames than any warmed bucket (slower than speed 1 with chunks near `--max-chunk-seconds`) still run, at a bucket that's cold the first time.
- A batch of one keeps its text side at its exact length: in fp16, padding the text side changed some durations (26 of 650 real sentences had one off by a frame). All 510 lengths are warmed instead.
- Batches of more pad their ids to multiples of 16 and their size to a power of two (with copies of the last request, whose results are dropped); decoder groups split into power-of-two sizes and hold at most `--max-chunk-seconds` of padded frames, so every group shape is warmed.

Measured on the RTX 3060 Ti in gVisor (`bench/job_buckets.sh`; the PR has the full tables):
- Audio, padded vs exact sizes at batch 1 over 650 real sentences: durations and lengths identical for all. Spectrogram correlation is 0.998 mean and 0.994 min, against a seed-to-seed noise floor of 0.996 mean and 0.993 min (fp16 compiled; fp32 on 200 is at the floor too). At batch 8, 14 of 57,731 durations moved a frame, the fp16 batch numerics described above.
- Latency, 200 real sentences each sent twice over the socket at batch 1: the first send went from 70 to 37 ms median, and the repeat stayed at 34–37 ms. Bucketed, the first and second sends take the same time.
- Throughput rose from 118–131x to 137–153x at 1 stream, and from 123–129x to 134–153x at 16 (interleaved A/B, 3 rounds).
- Startup: warming takes about 47 s of GPU time at batch 1 (510 text lengths, 79 decoder buckets). From a seeded cache, startup was 54–78 s before and 59 s after. Unseeded, it went from 2.3 to 3 minutes at batch 1, and from 7.9 to 7.1 at batch 8 (the old warmup compiled graphs that buckets never use).
- GPU memory: warming the largest bucket (4,856 frames, about 2 minutes of audio) peaks at 2.9 GB reserved. The worker returns that memory before it listens and then holds 630–730 MiB, against 470–510 before. Any request that long reached the same peak before too.
