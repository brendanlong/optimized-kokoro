# GPU worker

Loads Kokoro-82M, listens on a Unix socket, speaks [the worker protocol](../docs/worker-protocol.md), and batches whatever is queued. It runs in gVisor (`runsc`, GPU through nvproxy) with no network and a read-only root. It never sees text, only phoneme ids, and re-validates every frame; any violation closes the connection.

```sh
uv sync
uv run python -m kokoro_worker.convert models/          # once: .pth/.pt -> safetensors (outside the sandbox)
podman build -t localhost/kokoro-worker -f Containerfile .
./run.sh /run/user/$UID/kokoro models/ --max-batch 8   # socket at /run/user/$UID/kokoro/worker.sock
uv run python -m kokoro_worker.client /run/user/$UID/kokoro/worker.sock \
    --config models/config.json --phonemes "həlˈO wˈɜɹld." --out hello.wav
uv run pytest                                             # protocol and connection tests, no GPU
```

Startup compiles the model (torch.compile, Triton) into a tmpfs, which takes about a minute; the worker listens only once it's warm.

## Files

- `kokoro_worker/protocol.py`: framing and validation, no torch.
- `kokoro_worker/scheduler.py`: the queue, by (priority, arrival), with cancellation and limits.
- `kokoro_worker/server.py`: the socket, a reader thread per connection, and the GPU loop.
- `kokoro_worker/model.py`: loads the weights and runs a batch. The modules are the kokoro fork's (`brendanlong/kokoro@66c8ed5`, imported without its G2P); the forward pass is ours.
- `kokoro_worker/masked.py`: the decoder's forward pass with padding masked out.
- `kokoro_worker/convert.py`, `kokoro_worker/client.py`: one-time converter and a test client.
- `bench/`: equivalence checks, load generator, listening pairs, and the gpuc job that runs them.

## Batching

When the GPU is free the worker takes up to `--max-batch` queued requests, most urgent first, and never waits for more. The text side (ALBERT, text encoder, duration predictor) runs on the whole batch with attention masks, zeroed padding and packed LSTMs, so durations match a batch of one exactly. Then the decoder runs in groups of similar output length (`--pad-slack`, default 25% padding at most), most urgent group first, and each group's audio goes out as soon as it's done.

Padding used to change the audio (spectrogram correlation about 0.95 against a 0.996 noise floor): the instance norms took statistics over the padding, and padded frames leaked into real ones through the convolutions. `masked.py` takes the norm statistics over each row's real frames and zeroes padded frames before every convolution. Padded audio then correlates with unpadded at the noise floor. `--no-exact` turns the masks off.
