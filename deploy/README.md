# Deployment

Two systemd user services: `kokoro-worker` (the GPU worker in gVisor, via `worker/run.sh serve`, with the RAM/CPU/thread limits on the unit) and `kokoro-server` (the Rust front). They run outside gpuc: this is a system service, not an ML job.

```sh
deploy/deploy.sh                    # build both, export a compile cache, install, start; listens on 127.0.0.1:8880
journalctl --user -u kokoro-worker -u kokoro-server -f
```

State lives in `~/.local/share/optimized-kokoro` (binary, `run.sh`, models, compile-cache seed, `server.toml`, `server.env` with `KOKORO_API_KEYS`, mode 0600). `deploy.sh` writes `server.env` with a new random key only if it doesn't exist. On brendan-desktop it holds the key clawed-abode already used for Kokoro-FastAPI, so clawed-abode switched over without a config change.

Startup: about 2.5 minutes to export the compile cache during a deploy; restarts then take about 90 s. The server answers `/health` with 503 until the worker is connected.

The old Kokoro-FastAPI quadlet was disabled on 2026-11-03 by renaming `~/.config/containers/systemd/kokoro.container` to `kokoro.container.disabled`. To bring it back, rename it back, run `systemctl --user daemon-reload`, and start `kokoro.service` (stop `kokoro-server` first, since both use port 8880).
