# Deployment

Three containers on the host's system Docker, defined by `compose.yaml`: `worker` (the GPU worker), `server` (the Rust front, on `127.0.0.1:8880`) and `demo` (the demo page and its proxy, on `127.0.0.1:25436`). **Decided (Brendan, 2026-11-03):** the services must not run as the `claude` user, because that account holds his credentials. They used to run as systemd user units under `claude`: the worker in rootless podman with `--userns=keep-id`, mapped to claude's uid, and the server as a bare process. Brendan installs the new deployment with sudo; `claude` isn't in the `docker` group and can't manage it.

Every container runs in gVisor, as nobody (65534:65534), with a read-only root, no capabilities and `no-new-privileges`:

| | runtime | network | writable | reads |
|---|---|---|---|---|
| worker | `runsc-kokoro`, one GPU | none | `/tmp` (2 GB tmpfs, `exec` for Triton), the socket volume | models, its release's compile-cache seed |
| server | `runsc-kokoro` | `127.0.0.1:8880` | nothing (connects to the socket) | UniDic, `server.toml` (secret) |
| demo | `runsc-gpuc` | `127.0.0.1:25436` | nothing | `demo.key` (secret) |

The worker keeps run.sh's limits (12 GB RAM and no swap, 4 CPUs, 1024 pids); the server gets 8192 open files (`max_connections` needs one per connection), 3 GB and 256 pids, and the demo 512 MB.

Tailscale serve already proxies `svc:kokoro` (https://kokoro.tail8de88a.ts.net) to `http://127.0.0.1:8880`, so the server must keep publishing exactly that address.

## The runsc-kokoro runtime

`install.sh` adds a Docker runtime to `/etc/docker/daemon.json` (backing it up to `daemon.json.bak-kokoro`) and reloads Docker, which picks up new runtimes without restarting any container:

```json
"runsc-kokoro": {"path": "/usr/bin/runsc", "runtimeArgs": ["--directfs=false", "--host-uds=all"]}
```

- `--directfs=false`: under rootful Docker, runsc's default directfs mode runs the sandbox as root in the host's user namespace. With it off, the sentry runs as nobody in an empty user namespace, and all file access goes through the gofer (the same as `runsc-gpuc`; see the gVisor nvproxy wiki page). It costs metadata-heavy file I/O, which shows in the worker's startup: with the same image and cache seed under rootless podman, 205 s to ready with directfs off against 91 s with it on (the first warmup compile, 165 s against 69 s). Serving isn't affected.
- `--host-uds=all`: each container is its own gVisor kernel, so the worker's socket only reaches the server if it's a real host socket in the shared volume. The worker needs `create` and the server `open`. Checked with rootless podman and runsc release-20261102.0: without `open` the second sandbox gets `ECONNREFUSED`; with both, a socket bound by uid 65534 in one sandbox shows up on the host owned by that uid, mode 0660, and a uid-65534 client in another sandbox connects (uid 1000 gets `EACCES`).

The existing `runsc` runtime (`--nvproxy=true`, directfs on) isn't used. GPU support turns on by itself when Docker adds the NVIDIA devices (`--gpus`, gVisor's legacy hook mode); Brendan checked `docker run --runtime=runsc-gpuc --gpus all` on this host.

The socket lives in a named volume that is a tmpfs created as `uid=65534,gid=65534,mode=0700` (mounted with `nocopy`, so Docker never copies an image directory's ownership onto it). Only the worker and the server mount it. The volume is mounted on the host while either container uses it, and the worker replaces a stale `worker.sock` when it starts.

## Install (once)

Run it as yourself (`sudo` from your own account, not `claude`) from your own checkout: the script runs as root and builds what's in the checkout. `install.sh` and `update.sh` refuse a checkout that `claude` can write (any file in it, or any parent directory), or that has files owned by anyone but you or root, or world-writable ones. The converters run as you (`SUDO_USER`, or `KOKORO_BUILD_USER`), never as `claude`. Root never opens files inside a directory you or `claude` own: it reads them out with `sudo -u OWNER cat` into its own temporary directory. Only one `install.sh` or `update.sh` runs at a time (`/srv/optimized-kokoro/.lock`).

```sh
git clone https://github.com/brendanlong/optimized-kokoro ~/optimized-kokoro && cd ~/optimized-kokoro
sudo deploy/install.sh
```

What it does, in this order, while the old user units keep serving:
1. Adds the `runsc-kokoro` runtime (above), and creates `/srv/optimized-kokoro` with `settings.env`.
2. UniDic, as `unidic/SHA256.dic` (the checksum in `crates/g2p-ja/tools/fetch_unidic.sh`): copies the old `unidic.dic` (read as `claude`) if it matches, or fetches and converts it as you (this needs cargo). Checked again once root has the copy.
3. Models: converts them as you with `uv run python -m kokoro_worker.convert` (a pinned Hugging Face revision; this needs uv). Only `config.json`, `model.safetensors` and `voices.safetensors` are taken. `KOKORO_MODELS_FROM=DIR` copies a converted directory instead, read as the directory's owner, for example `KOKORO_MODELS_FROM=~claude/.local/share/optimized-kokoro/models` (the conversion is deterministic: a fresh one matched the deployed files byte for byte).
4. API keys: reuses the keys in `~claude/.local/share/optimized-kokoro/server.env` (read as `claude`), so clawed-abode needs no change, or makes a new one, and makes a separate key for the demo (limited to 4 streams, so it can't take all of clawed-abode's capacity). They go in `secrets/server.toml` and `secrets/demo.key`.
5. Builds the three images and exports a compile cache: a one-off `docker compose run` of the new worker as nobody, which warms up, writes the caches into a directory only nobody can write, and exits without listening. Root then takes ownership (dropping any setuid bits), and the serving worker mounts it read-only.
6. Stops the old units as `claude` (only now, to free port 8880), starts the containers and waits for `/health`. If they don't come up healthy, it shows their logs, removes them, starts the old units again, and removes the release.

It removes nothing of the old setup. Once you've checked the new deployment (below), run

```sh
sudo deploy/install.sh --remove-old
```

which first checks that the old units are stopped and that the new server answers `/health`, then, as `claude`: `systemctl --user disable --now kokoro-worker kokoro-server`, deletes the two unit files and daemon-reloads, and deletes `~/.local/share/optimized-kokoro`. Until then the old units are still enabled, and after a reboot they'd race the containers for port 8880. It leaves claude's podman images (`podman rmi localhost/kokoro-worker:deploy localhost/kokoro-worker:deploy-new` as `claude`).

The demo page listens on `127.0.0.1` only. To reach it from the tailnet over HTTPS, which also lets browsers use WebCodecs instead of the WASM encoder (plain-HTTP pages aren't a secure context), define `svc:kokoro-demo` in the Tailscale admin console like `svc:kokoro`, then:

```sh
sudo tailscale serve --service=svc:kokoro-demo --https=443 http://127.0.0.1:25436
```

## Checks after the first install

What couldn't be tested before the first install (no Docker access from `claude`): runsc and the GPU under Docker, the socket in a Docker volume, the limits, and the ports. With `c="sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml"`:

1. `$c ps` shows worker, server and demo up; `curl -s 127.0.0.1:8880/health` prints 200.
2. Sandboxed, as nobody: `$c exec worker sh -c 'id; uname -r'` prints uid 65534 and gVisor's kernel version, not the host's (the server and demo images have no shell). `ps -eo user,args | grep runsc-sandbox` should show the three sandboxes running as `nobody`, not root (directfs off).
3. GPU: `$c logs worker` shows `compile caches seeded`, the warmups and `listening on /run/kokoro/worker.sock`, with no CUDA errors and no `Read-only file system` or `Permission denied` from Triton under `/tmp`.
4. Socket: `sudo ls -l /var/lib/docker/volumes/optimized-kokoro_sock/_data` shows `srw-rw---- 65534 65534 worker.sock`, and `$c logs server` has no reconnect errors after the worker is up.
5. Speech, English and Japanese (UniDic), with the clawed-abode key from `sudo cat /srv/optimized-kokoro/secrets/server.toml`:
   `curl -s -o /tmp/en.pcm -w '%{http_code} %{size_download}\n' -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' -d '{"model":"kokoro","input":"Hello there.","voice":"af_heart","response_format":"pcm"}' 127.0.0.1:8880/v1/audio/speech`, then the same with `"input":"こんにちは、世界。","voice":"jf_alpha"`. Both 200 with audio. Then a clawed-abode TTS request, and Lion Reader if it uses this server.
6. Limits: `sudo docker stats --no-stream` shows a 12 GiB limit on the worker (3 GiB server, 512 MiB demo).
7. Ports: `ss -ltn | grep -E ':8880|:25436'` shows only `127.0.0.1`.
8. Demo: after `tailscale serve`, the page at https://kokoro-demo.tail8de88a.ts.net plays (and from a plain-HTTP tunnel to 127.0.0.1:25436, with the WASM encoder).
9. Recovery: `$c restart worker`; the server reconnects by itself (`/health` back to 200 after the worker's startup).
10. Then `sudo deploy/install.sh --remove-old`, and after the next reboot, check 1 again (the containers start with Docker, and nothing else holds port 8880).

## Update

```sh
git pull && sudo deploy/update.sh
```

It builds new images and exports a new compile cache into a new release directory while the running release keeps serving, then points `current` at it (by rename) and recreates the containers. If they don't come up healthy, it points `current` back at the previous release, starts that again, and removes the failed release and its images (as it does for a release whose build or cache export fails). It keeps the previous release (images, compose file, cache) for going back by hand:

```sh
sudo ln -sfn releases/PREVIOUS /srv/optimized-kokoro/current
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml up -d
```

To change the GPU, the demo port or the worker's `--max-batch` (the cache seed depends on it), edit `/srv/optimized-kokoro/settings.env` and run `update.sh`.

The worker's startup with a cache seed should take about 3.5 minutes with directfs off (above; 70-90 s under the old setup), and a cache export somewhat longer, so an update is down for about 3.5 minutes. `/health` answers 503 until the worker is connected.

```sh
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml ps
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml logs -f
```

## Layout

`/srv/optimized-kokoro`, all root-owned (see the top of `lib.sh`):
- `settings.env`: `KOKORO_GPU` (an index, a UUID or a MIG device), `KOKORO_DEMO_PORT`, `KOKORO_MAX_BATCH`. Any other line, apart from comments, is an error.
- `models/`, and `unidic/SHA256.dic`: world-readable data. The server memory-maps the dictionary, so a new one (a new checksum in `fetch_unidic.sh`) is a new file next to the old one. Each release's `.env` names its own (`KOKORO_UNIDIC_FILE`), so neither the running server nor a rollback ever sees the dictionary change. Dictionaries no kept release names are deleted.
- `secrets/` (0700): `server.toml` (listen address, socket path and API keys) and `demo.key`, each `root:65534` mode 0640. Docker bind-mounts them as Compose secrets at `/run/secrets/` (outside swarm, Compose can't change their owner or mode, so the files carry them). The keys never go through the environment.
- `releases/REL/`: `compose.yaml` (a copy of `deploy/compose.yaml`), `.env` (`KOKORO_ROOT`, `KOKORO_RELEASE`, `KOKORO_UNIDIC_FILE` and the settings) and `cache-seed/`. Images are `optimized-kokoro-{worker,server,demo}:REL`.
- `current`: a symlink to the running release.

## Notes

- The server and the demo share a bridge network (172.30.88.0/24, fixed addresses because Docker's embedded DNS doesn't work under runsc). It isn't `internal`, since published ports need a gateway, so both could make outbound connections; neither does.
- `worker/run.sh` (rootless podman) is still how to run the worker by hand and in `worker/bench/`.
- The server image keeps the espeak-ng data at `/src/crates/espeak-ng/espeak-ng-data`, the path it was built at: `kokoro-espeak-ng` finds it through `CARGO_MANIFEST_DIR` at run time.
- `compose.yaml` has a commented-out `cloudflared` service for later. Cloudflare Access (or the relay in `docs/plan.md`) must filter unauthenticated traffic before it reaches the server.
- The old Kokoro-FastAPI quadlet was disabled on 2026-11-03 by renaming `~claude/.config/containers/systemd/kokoro.container` to `kokoro.container.disabled`. It also uses port 8880.
