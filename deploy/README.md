# Deployment

Three containers on the host's system Docker, defined by `compose.yaml`:
- `worker`: the GPU worker.
- `server`: the Rust front, on `127.0.0.1:8880` by default.
- `demo`: the demo page and its proxy, on `127.0.0.1:25436` by default.

`install.sh` sets them up once and `update.sh` replaces them with a new build. Both run as root (with sudo), and nothing runs as the user who installs it.

## Requirements

- Linux on x86-64-v3 (AVX2: Intel Haswell, AMD Zen or newer; `.cargo/config.toml`).
- Docker Engine (rootful, the system daemon) with the Compose plugin.
- gVisor's `runsc`, recent enough to support your NVIDIA driver: `runsc nvproxy list-supported-drivers` must list the version `nvidia-smi` reports. `install.sh` checks this. nvproxy supports exact driver versions only, so hold the driver and runsc together. Distribution packages of runsc are often too old; gVisor's own apt repository tracks its releases.
- An NVIDIA GPU with its driver and the NVIDIA Container Toolkit, so that `docker run --gpus` works (gVisor uses its legacy hook mode). The model needs about 2 GB of GPU memory at `--max-batch 1`.
- `jq`, `curl`, `flock`, `ss` and `sha256sum` on the host. For the converters (unless you import their output, below): uv for the models, and cargo for UniDic.
- About 16 GB of disk for the images and data, and 12 GB of RAM for the worker.

## Security model

- Every container runs in gVisor, as nobody (65534:65534), with a read-only root, no capabilities and `no-new-privileges`. They all use a Docker runtime that `install.sh` adds to `/etc/docker/daemon.json` (backing it up to `daemon.json.bak-kokoro`). It then reloads Docker, which picks up new runtimes without restarting any container:

  ```json
  "runsc-kokoro": {"path": "/usr/bin/runsc", "runtimeArgs": ["--directfs=false", "--host-uds=all"]}
  ```

  - `--directfs=false`: under rootful Docker, runsc's default directfs mode runs the sandbox as root in the host's user namespace. With it off, the sentry runs as nobody in an empty user namespace, and all file access goes through the gofer. It costs metadata-heavy file I/O. That shows in the worker's startup: with the same image and cache seed, 205 s to ready with directfs off against 91 s with it on (measured with rootless podman). Serving isn't affected.
  - `--host-uds=all`: each container is its own gVisor kernel. The worker's socket reaches the server only as a real host socket in their shared volume. The worker needs `create` and the server `open`.
- The worker has no network, a 2 GB tmpfs at `/tmp` (mounted `exec` for Triton) and one GPU. It reads the models and its release's compile-cache seed, both read-only. Its limits are 12 GB of RAM and no swap, 4 CPUs and 1024 pids. GPU memory isn't limited.
- The server reads UniDic and its config. It writes nothing. Its limits are 8192 open files (`max_connections` needs one per connection), 3 GB and 256 pids. The demo gets 512 MB.
- The socket lives in a named volume: a tmpfs created as `uid=65534,gid=65534,mode=0700` and mounted with `nocopy`, so Docker never copies an image directory's ownership onto it. Only the worker and the server mount it, and the worker replaces a stale `worker.sock` when it starts.
- The API keys are in `secrets/server.toml` and `secrets/demo.key`, each `root:65534` mode 0640, in a 0700 directory. Docker bind-mounts them as Compose secrets. Outside swarm, Compose can't change their owner or mode, so the files carry them. The keys never go through the environment.
- The server and the demo share a bridge network, 172.30.88.0/24. The addresses are fixed because Docker's embedded DNS doesn't work under runsc. The network isn't `internal`, since published ports need a gateway, so both could make outbound connections; neither does. Ports are published only on the addresses in `settings.env`, `127.0.0.1` by default. Behind anything public, an edge must filter unauthenticated traffic first (see `crates/server/src/config.rs`).
- The scripts run as root from your checkout, and build images from it. So they refuse a checkout, or any directory above it, that anyone but you and root can change: by owner or by mode. Group write is allowed only for your private group, and sticky directories are allowed. ACLs aren't checked.
- The converters run as you, not as root. Root never opens a file inside a directory someone else owns. It reads each one with `sudo -u OWNER cat` into its own temporary directory, so a symlink can't make it copy something that user couldn't read. A root-owned import is read directly, but only through root-owned directories without symlinks.

## Install

From your own checkout, with sudo from your own account:

```sh
git clone https://github.com/brendanlong/optimized-kokoro && cd optimized-kokoro
sudo deploy/install.sh
```

It fails before building anything if the server's or the demo's port is already in use, naming what holds it. For other addresses or another GPU, create `/srv/optimized-kokoro/settings.env` first. Otherwise `install.sh` writes it with these defaults commented out:

```sh
KOKORO_GPU=0                        # an index, a UUID or a MIG device from nvidia-smi -L
KOKORO_LISTEN=127.0.0.1:8880        # the server (IPv4:port); 0.0.0.0 exposes it everywhere
KOKORO_DEMO_LISTEN=127.0.0.1:25436  # the demo page
KOKORO_MAX_BATCH=1                  # the worker's --max-batch (worker/README.md)
```

What it does:
1. Checks the checkout, the driver against runsc and the ports. Adds the runtime and creates `/srv/optimized-kokoro`.
2. UniDic: fetches and converts it as you (`crates/g2p-ja/tools/fetch_unidic.sh`), and checks it against that script's checksum once root has the copy.
3. Models: converts them as you with `uv run python -m kokoro_worker.convert`, from a pinned Hugging Face revision. The conversion is deterministic. Only `config.json`, `model.safetensors` and `voices.safetensors` are taken.
4. API keys: makes one key for your clients, and a separate key for the demo page. The demo's key is limited to 4 streams, so it can't take all of the clients' capacity.
5. Builds the three images and exports a compile cache. That's a one-off `docker compose run` of the worker as nobody, which warms up, writes the caches into a directory only nobody can write, and exits without listening. Root then takes ownership (dropping any setuid bits), and the serving worker mounts it read-only.
6. Starts the containers and waits for `/health`. If they don't come up healthy, it shows their logs and removes them and the release.

Existing data can be imported instead. Each file is read as its owner:

```sh
sudo deploy/install.sh --keys-from FILE --models-from DIR --unidic-from FILE
```

- `--keys-from FILE`: a `KOKORO_API_KEYS=a,b` line, or one key per line (with `#` comments).
- `--models-from DIR`: what `kokoro_worker.convert` wrote.
- `--unidic-from FILE`: a converted `unidic.dic` with its `unidic.dic.LICENSE` next to it. It must match the checksum.

The clients' keys are then in `/srv/optimized-kokoro/secrets/server.toml` (`sudo cat` it). Clients send one as `Authorization: Bearer KEY`.

The demo page needs HTTPS (a secure context) to use WebCodecs; over plain HTTP it falls back to a WASM encoder. Put it behind a TLS-terminating proxy on the host, pointing at `KOKORO_DEMO_LISTEN`.

## Checks after installing

With `c="sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml"`:

1. `$c ps` shows worker, server and demo up, and `curl -s 127.0.0.1:8880/health` prints 200. It's 503 until the worker is ready, which takes about 3.5 minutes.
2. `$c exec worker sh -c 'id; uname -r'` prints uid 65534 and gVisor's kernel version, not the host's. The server and demo images have no shell. `ps -eo user,args | grep runsc-sandbox` should show the sandboxes running as `nobody`, not root.
3. `$c logs worker` shows `compile caches seeded`, the warmups and `listening on /run/kokoro/worker.sock`. There should be no CUDA errors, and no `Read-only file system` or `Permission denied` from Triton under `/tmp`.
4. `sudo ls -l /var/lib/docker/volumes/optimized-kokoro_sock/_data` shows `srw-rw---- 65534 65534 worker.sock`.
5. Speech in English and in Japanese (Japanese reads UniDic), with a key from `server.toml`:
   `curl -s -o /tmp/en.pcm -w '%{http_code} %{size_download}\n' -H "Authorization: Bearer $KEY" -H 'Content-Type: application/json' -d '{"model":"kokoro","input":"Hello there.","voice":"af_heart","response_format":"pcm"}' 127.0.0.1:8880/v1/audio/speech`. Then the same with `"input":"こんにちは、世界。","voice":"jf_alpha"`. Both should print 200 and a nonzero size.
6. `sudo docker stats --no-stream` shows the limits: 12 GiB for the worker, 3 GiB for the server and 512 MiB for the demo.
7. `ss -ltn` shows the two ports only on the addresses in `settings.env`.
8. The demo page plays.
9. `$c restart worker`: the server reconnects by itself, and `/health` returns 200 again after the worker's startup.
10. After a reboot, check 1 again. The containers start with Docker (`restart: unless-stopped`).

## Update

```sh
git pull && sudo deploy/update.sh
```

It builds new images and exports a new compile cache into a new release directory while the running release keeps serving. Then it points `current` at the new release (by rename) and recreates the containers, so the service is down for the worker's startup, about 3.5 minutes.

If they don't come up healthy, it points `current` back at the previous release, starts that again, and removes the failed release and its images. It does the same for a release whose build or cache export fails. It keeps the previous release (images, compose file, cache) for going back by hand:

```sh
sudo ln -sfn releases/PREVIOUS /srv/optimized-kokoro/current
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml up -d
```

To change a setting, edit `/srv/optimized-kokoro/settings.env` and run `update.sh`. A new address must be free.

## Uninstall

```sh
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml down --volumes
sudo docker image ls --filter 'reference=optimized-kokoro-*' -q | sort -u | xargs -r sudo docker image rm -f
sudo rm -rf /srv/optimized-kokoro
```

Then, if nothing else uses it, remove `runsc-kokoro` from `/etc/docker/daemon.json` (the original is in `daemon.json.bak-kokoro`) and run `sudo systemctl reload docker`.

## Layout

`/srv/optimized-kokoro`, all root-owned (see the top of `lib.sh`):
- `settings.env`: the settings above. Any other line, apart from comments, is an error.
- `models/` and `unidic/SHA256.dic`: world-readable data. The server memory-maps the dictionary, so a new one (a new checksum in `fetch_unidic.sh`) is a new file next to the old one. Each release's `.env` names its own (`KOKORO_UNIDIC_FILE`), so neither the running server nor a rollback sees it change. Dictionaries that no kept release names are deleted.
- `secrets/`: `server.toml` (listen address, socket path and API keys) and `demo.key`.
- `releases/REL/`: `compose.yaml` (a copy of `deploy/compose.yaml`), `.env` (`KOKORO_ROOT`, `KOKORO_RELEASE`, `KOKORO_UNIDIC_FILE` and the settings) and `cache-seed/`. Images are `optimized-kokoro-{worker,server,demo}:REL`.
- `current`: a symlink to the running release.
- `.lock`: held while `install.sh` or `update.sh` runs.

## Notes

- `worker/run.sh` (rootless podman) is still how to run the worker by hand, and how `worker/bench/` runs it.
- The server image keeps the espeak-ng data at `/src/crates/espeak-ng/espeak-ng-data`, the path it was built at, because `kokoro-espeak-ng` finds it through `CARGO_MANIFEST_DIR` at run time.
- `compose.yaml` has a commented-out `cloudflared` service for later.
