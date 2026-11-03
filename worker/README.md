# GPU worker

Loads Kokoro-82M, listens on a Unix socket, speaks [the worker protocol](../docs/worker-protocol.md), and batches whatever is queued. It runs in gVisor (`runsc`, GPU through nvproxy) with no network and a read-only root. It never sees text, only phoneme ids, and re-validates every frame; any violation closes the connection.

```sh
uv sync
uv run python -m kokoro_worker.convert models/          # once: .pth/.pt -> safetensors (outside the sandbox)
podman build -t localhost/kokoro-worker -f Containerfile .
./run.sh /run/user/$UID/kokoro models/                 # socket at /run/user/$UID/kokoro/worker.sock
uv run python -m kokoro_worker.client /run/user/$UID/kokoro/worker.sock \
    --config models/config.json --phonemes "həlˈO wˈɜɹld." --out hello.wav
uv run pytest                                             # protocol and connection tests, no GPU
```

Startup compiles the model (torch.compile, Triton) into a tmpfs; the worker listens only once it's warm, so the front gets its Hello at once. In gVisor that takes about 2 minutes at `--max-batch 1` and 8 at `--max-batch 8`. To restart faster, seed the caches: run once with `KOKORO_CACHE_EXPORT=DIR ./run.sh ...` (it writes the caches after warmup, before accepting a connection), then start with `KOKORO_CACHE_SEED=DIR`, which mounts them read-only and copies them in (8 minutes down to 1.5 with batching). A seed is only valid for the same image, GPU and driver.

`--max-chunk-seconds` (default 120) is the most audio one request may produce at speed 1, scaled by 1/speed and announced in Hello; longer requests get `Failed` code 3 before the decoder runs. Real 510-phoneme chunks reach about 44 s (`bench/max_duration.py`).

## Files

- `kokoro_worker/protocol.py`: framing and validation, no torch.
- `kokoro_worker/scheduler.py`: the queue, by (priority, arrival), with cancellation and limits.
- `kokoro_worker/server.py`: the socket, a reader thread per connection, and the GPU loop.
- `kokoro_worker/model.py`: loads the weights and runs a batch. The modules are the kokoro fork's (`brendanlong/kokoro@66c8ed5`, imported without its G2P); the forward pass is ours.
- `kokoro_worker/masked.py`: the decoder's forward pass with padding masked out.
- `kokoro_worker/convert.py`, `kokoro_worker/client.py`: one-time converter and a test client.
- `bench/`: equivalence checks, load generator, listening pairs, and the gpuc job that runs them.

## Batching

`--max-batch` defaults to 1. On the RTX 3060 Ti with real sentences, `--max-batch 8` gives about +39% throughput at 16 concurrent requests (181x vs 130x), the same at 4, and slower single requests (60 vs 34 ms median); the last is a regression in the serving loop whose cause isn't found yet (the model path alone is as fast at batch 1). Use 8 when many listeners share the worker.

When the GPU is free the worker takes up to `--max-batch` queued requests, most urgent first, and never waits for more. The text side (ALBERT, text encoder, duration predictor) runs on the whole batch with attention masks, zeroed padding and packed LSTMs, so durations match a batch of one (exactly in fp32; in fp16 about one sentence in 16 gets one duration a frame off). Then the decoder runs in groups of similar output length (`--pad-slack`, default 25% padding at most), most urgent group first, and each group's audio goes out as soon as it's done.

Padding used to change the audio (spectrogram correlation about 0.95 against a 0.996 noise floor): the instance norms took statistics over the padding, and padded frames leaked into real ones through the convolutions. `masked.py` takes the norm statistics over each row's real frames and zeroes padded frames before every convolution. Padded audio then correlates with unpadded at the noise floor (fp32). In fp16, any batch, padded or not, differs from batch 1 a little more than seed noise (0.99 vs 0.996), and about one sentence in 16 gets one duration a frame off; that's fp16 numerics depending on batch size. `--no-exact` turns the masks off, which is about 10% faster and changes the audio (heavily padded rows come out about twice as loud).

Batch 1 and batches above 1 compile into separate graphs, because a graph compiled for a dynamic batch also accepts batch 1 (no measured speed difference either way).
