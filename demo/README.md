# Streaming demo

A page where you type text and hear it as the server renders it. The server streams 24 kHz PCM, one sentence at a time. The page encodes it to AAC-LC or Opus in fragmented MP4 as it arrives (WebCodecs, or a WASM AAC encoder where WebCodecs isn't available, e.g. plain-HTTP pages, which aren't a secure context) and appends it to a Media Source Extensions stream. The encoder is Lion Reader's (`src/lib/narration/audio-encoding.ts`), by way of clawed-abode.

`serve.py` serves the page and proxies `/v1/audio/voices` and `/v1/audio/speech` to a kokoro-server, adding the API key, so the key stays out of the browser and the server needs no CORS.

```sh
pnpm install && pnpm run build
python3 serve.py --listen 0.0.0.0:PORT --upstream http://127.0.0.1:SERVER_PORT --key-file KEY_FILE
```

`worker/bench/serve.sh` runs the worker and server for a live trial (a gpuc job). In production it runs as the `demo` service of `deploy/compose.yaml` (image: `Containerfile`), on 127.0.0.1:25436 with its own API key.
