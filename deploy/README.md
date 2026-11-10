# Deployment

Three containers on the host's system Docker, defined by `compose.yaml`:
- `worker`: the GPU worker.
- `server`: the Rust front, on `127.0.0.1:8880` by default.
- `demo`: the demo page and its proxy, on `127.0.0.1:25436` by default.

`install.sh` sets them up once and `update.sh` replaces them with a new build. Both run as root (with sudo). No container runs as the user who installs it: they all run as nobody. Only the converters run as that user, outside the containers.

## Requirements

- Linux on x86-64-v3 (AVX2: Intel Haswell, AMD Zen or newer; `.cargo/config.toml`).
- Docker Engine (rootful, the system daemon) with the Compose plugin.
- gVisor's `runsc`, recent enough to support your NVIDIA driver: `runsc nvproxy list-supported-drivers` must list the version `nvidia-smi` reports. `install.sh` checks this. nvproxy supports exact driver versions only, so hold the driver and runsc together. Distribution packages of runsc are often too old; gVisor's own apt repository tracks its releases.
- An NVIDIA GPU with its driver, and the NVIDIA Container Toolkit with a CDI spec (`nvidia-ctk cdi list` shows `nvidia.com/gpu=0`; `sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml` makes one). Docker's CDI support must be on, which is the default from Docker 28 (before that, `"features": {"cdi": true}` in `daemon.json`). `install.sh` checks all of this. The model needs about 2 GB of GPU memory at `--max-batch 1`.

  The worker gets its GPU as a CDI device (`devices: nvidia.com/gpu=N`) with `NVIDIA_VISIBLE_DEVICES=void`, not with `--gpus`. With an image that sets `NVIDIA_VISIBLE_DEVICES=all` (every CUDA base image), `--gpus` takes the NVIDIA legacy hook path. Under runsc release-20261102.0 the sandbox then dies at boot with `Error reading mounts file: error unmarshaling mounts: unexpected end of JSON input`. CDI plus `void` works under gVisor: checked with `docker run --runtime=runsc-kokoro --device nvidia.com/gpu=0 -e NVIDIA_VISIBLE_DEVICES=void nvidia/cuda:12.6.3-base-ubuntu24.04 nvidia-smi` (`runsc-kokoro` was the single runtime of releases before the split, below).
- `jq`, `curl`, `flock`, `ss` and `sha256sum` on the host. For the converters (unless you import their output, below): uv for the models, and cargo for UniDic.
- About 16 GB of disk for the images and data, and 12 GB of RAM for the worker (twice that during the cache export and an update, when two workers run).

## Security model

- Every container runs in gVisor, as nobody (65534:65534), with a read-only root, no capabilities and `no-new-privileges`. They use three Docker runtimes that `install.sh` and `update.sh` add to `/etc/docker/daemon.json` (backing up the file from before their first change to `daemon.json.bak-kokoro`). They then reload Docker, which picks up new runtimes without restarting any container:

  ```json
  "runsc-kokoro-uds-create": {"path": "/usr/bin/runsc", "runtimeArgs": ["--directfs=false", "--host-uds=create"]},
  "runsc-kokoro-uds-open": {"path": "/usr/bin/runsc", "runtimeArgs": ["--directfs=false", "--host-uds=open"]},
  "runsc-kokoro-no-uds": {"path": "/usr/bin/runsc", "runtimeArgs": ["--directfs=false"]}
  ```

  - `--directfs=false`: under rootful Docker, runsc's default directfs mode runs the sandbox as root in the host's user namespace. With it off, the sentry runs as nobody in an empty user namespace, and all file access goes through the gofer. It costs metadata-heavy file I/O. That shows in the worker's startup: with the same image and cache seed, 205 s to ready with directfs off against 91 s with it on (measured with rootless podman). Serving isn't affected.
  - `--host-uds`: each container is its own gVisor kernel. The worker's socket reaches the server only as a real host socket in their shared volume. The worker binds it, so it gets `create` (`runsc-kokoro-uds-create`); the server connects to it, so it gets `open` (`runsc-kokoro-uds-open`). The demo and the tunnel use no Unix sockets on the host (`runsc-kokoro-no-uds`).
  - A runtime with one of these names but other flags stops `install.sh` and `update.sh` before they change anything.
  - Releases from before 2026-11-10 ran every container under one runtime, `runsc-kokoro`, with `--host-uds=all`. `update.sh` keeps it while a kept release uses it (so going back still works), and removes it from `daemon.json` once none does: at the end of the second update after the split.
- The worker has no network, a 2 GB tmpfs at `/tmp` (mounted `exec` for Triton) and one GPU. It reads the models and its release's compile-cache seed, both read-only. Its limits are 12 GB of RAM and no swap, 4 CPUs and 1024 pids. GPU memory isn't limited.
- The server reads UniDic and its config. It writes nothing. Its limits are 8192 open files (`max_connections` needs one per connection), 3 GB and 256 pids. The demo gets 512 MB.
- The socket lives in a named volume: a tmpfs created as `uid=65534,gid=65534,mode=0700` and mounted with `nocopy`, so Docker never copies an image directory's ownership onto it. Only the worker and the server mount it, and the worker replaces a stale `worker.sock` when it starts.
- The API keys are in `secrets/server.toml` and `secrets/demo.key`, each `root:65534` mode 0640, in a 0700 directory. Docker bind-mounts them as Compose secrets. Outside swarm, Compose can't change their owner or mode, so the files carry them. The keys never go through the environment.
- The server and the demo share a bridge network, 172.30.88.0/24. The addresses are fixed because Docker's embedded DNS doesn't work under runsc. Each release has its own network with this subnet; only one exists at a time (see Update). The network isn't `internal`, since published ports need a gateway, so both could make outbound connections; neither does. Ports are published only on the addresses in `settings.env`, `127.0.0.1` by default. Behind anything public, an edge must filter unauthenticated traffic first (see `crates/server/src/config.rs`).
- The scripts run as root from your checkout, and build images from it. So they refuse a checkout, or any directory above it, that anyone but you and root can change: by owner or by mode. Group write is allowed only for your private group, and sticky directories are allowed. ACLs aren't checked. This catches a misconfigured checkout, such as one another user can write. It can't detect a checkout that has already been tampered with, since the check itself runs from it.
- The converters run as you, not as root. Root never opens a file inside a directory someone else owns. It reads each one with `sudo -u OWNER cat` into its own temporary directory, so a symlink can't make it copy something that user couldn't read. A root-owned import is read directly, but only through root-owned directories without symlinks.

## Install

From your own checkout, with sudo from your own account:

```sh
git clone https://github.com/brendanlong/optimized-kokoro && cd optimized-kokoro
sudo deploy/install.sh
```

It fails before building anything if the server's or the demo's port is already in use, naming what holds it. For other addresses or another GPU, create `/srv/optimized-kokoro/settings.env` first. Otherwise `install.sh` writes it with these defaults commented out:

```sh
KOKORO_GPU=0                        # a CDI device name from nvidia-ctk cdi list (an index or UUID; not all)
KOKORO_LISTEN=127.0.0.1:8880        # the server (IPv4:port); 0.0.0.0 exposes it everywhere
KOKORO_DEMO_LISTEN=127.0.0.1:25436  # the demo page
KOKORO_MAX_BATCH=8                  # the worker's --max-batch, 1-64 (below)
KOKORO_MAX_CHUNK_SECONDS=60         # the worker's --max-chunk-seconds, 25-60: its GPU memory (below)
```

`KOKORO_MAX_CHUNK_SECONDS` is the most audio one request to the worker may make, at any speed, and the most audio in one of its packed decoder rows, so it sets the worker's peak GPU memory: about 200 MB plus 0.5 MB per frame of 25 ms (40 per second) reserved, packed rows a few percent more, plus about 175 MB for the CUDA context. Measured at `--max-batch 8` under load (`worker/README.md`): 60 s (the default) peaked at 1,336 MiB reserved, 30 s at 816 MiB, about 1 GB on the card, for about 8% less throughput. 30 s suits a GPU shared with a desktop. The server splits chunks that could be longer into overlapping pieces before sending them, sized from a fit to measured durations (`worker/README.md`). So a lower cap mostly costs more stitched sentences, especially at slow speeds: at 30 s, pieces of up to 270 ids at speed 1 and 45 at speed 0.25. A request can still be refused, failing its whole speech request with HTTP 500, when the model predicts more audio than the fit allows. That happened to none of the measured cases (English, and the reference fixtures' Chinese, Japanese and mixed-script chunks), but other text or voices could. The minimum, 25 s, keeps pieces at speed 0.25 to at least 32 ids, about five words. The server refuses a worker that announces more than 60 s (`worker_max_chunk_samples`). The compile-cache export runs with the same value.

`KOKORO_MAX_BATCH` defaults to 8: with many listeners it renders more audio per second than batch 1, at about the same latency for a single request, and the packed decoder rows keep its GPU memory to the chunk cap's (`worker/README.md`). Its compile cache takes longer to build (about 10 minutes cold against 4-6 at batch 1), which the export run pays.

What it does:
1. Checks the checkout, the driver against runsc, the CDI device and the ports. Adds the runtimes and creates `/srv/optimized-kokoro`.
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
- `--unidic-from FILE`: a converted `unidic.dic`, with its license next to it as `FILE.LICENSE` (what `fetch_unidic.sh` writes). It must match the checksum.

Keys are at least 16 characters, from letters, digits and `._~+/=-`. An import is skipped, with a message, when that data is already installed. The clients' keys are then in `/srv/optimized-kokoro/secrets/server.toml` (`sudo cat` it). Clients send one as `Authorization: Bearer KEY`.

The demo page needs HTTPS (a secure context) to use WebCodecs; over plain HTTP it falls back to a WASM encoder. Put it behind a TLS-terminating proxy on the host, pointing at `KOKORO_DEMO_LISTEN`.

## Checks after installing

With `c="sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml"`:

1. `$c ps` shows worker, server and demo up, and `curl -s -o /dev/null -w '%{http_code}\n' 127.0.0.1:8880/health` prints 200. It's 503 until the worker is ready, which takes about 3.5 minutes.
2. `$c exec worker sh -c 'id; uname -r'` prints uid 65534 and gVisor's kernel version, not the host's. The server and demo images have no shell. `ps -eo user,args | grep runsc-sandbox` should show the sandboxes running as `nobody`, not root.
3. `$c logs worker` shows `compile caches seeded`, the warmups and `listening on /run/kokoro/worker.sock`. There should be no CUDA errors, and no `Read-only file system` or `Permission denied` from Triton under `/tmp`.
4. `sudo ls -l /var/lib/docker/volumes/PROJECT_sock/_data` shows `srw-rw---- 65534 65534 worker.sock`, where PROJECT is the release's Compose project (`COMPOSE_PROJECT_NAME` in `/srv/optimized-kokoro/current/.env`, see Layout).
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

The service is down for seconds: the old server's shutdown (instant when idle, up to 10 s while requests finish) and the new one's start. Each release is its own Compose project (`optimized-kokoro-REL`, with its own socket volume and network), so the new worker warms up next to the running release.

1. It builds new images and exports a new compile cache into a new release directory while the running release keeps serving.
2. It starts the new release's worker next to the running one, and waits until it listens on its socket, which it does only after warming up (about as long as the export). The worker has no network, so it can start alongside the running release. For that time both workers are on the GPU. Before starting the new worker, `update.sh` checks that the GPU has as much free memory as the new worker's warmup took during the export, plus 256 MiB. Afterwards it prints the peak. A failed check throws away that build, so the retry builds again.
3. The swap: it removes the old release's server, demo and tunnel, then their network, points `current` at the new release (by rename), and starts the new server, demo and tunnel. When `/health` answers (and the tunnel has connected), it removes the old worker. It prints the downtime: from stopping the old server to the new one answering `/health`. That's the old server's shutdown, which waits up to 10 s for requests in progress (`shutdown_grace_ms`), plus the new server's start. With the tunnel, Cloudflare can't reach the server until the new cloudflared registers, a few seconds more; it prints that too.

Every release's network has the same subnet, 172.30.88.0/24, and the same addresses, so the tunnel's ingress stays `172.30.88.2:8880` and nothing changes in Cloudflare. Two networks can't share a subnet, so the old network goes before the new one is made, in step 3.

If the new worker doesn't get ready (it exits, restarts, or doesn't listen within 10 minutes), or there isn't the GPU memory, the new release is removed and the running one isn't touched. If the new server isn't healthy, or the tunnel doesn't connect, it takes the new release down, points `current` back, and starts the previous server, demo and tunnel again. The previous worker kept running, so that takes about as long as the swap. Then it removes the failed release and its images. Ctrl-C does the same at any point until the new release is healthy, and a failed build or cache export only removes the new release. A second Ctrl-C while it undoes is ignored. An update that was killed outright (so it couldn't clean up) leaves a project behind. The next `update.sh` removes any `optimized-kokoro*` project other than the running one. If the kill came mid-swap, it also restarts the previous release's server, demo and tunnel next to its worker before building. Over ssh, run it in tmux or screen: a dropped session (SIGHUP) kills it without undoing anything.

It keeps the previous release (images, compose file, cache) for going back by hand. That means a full worker startup, since the running release must go first (it holds the ports and the subnet):

```sh
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml down --volumes
sudo ln -sfn releases/PREVIOUS /srv/optimized-kokoro/current
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml up -d
```

To change a setting, edit `/srv/optimized-kokoro/settings.env` and run `update.sh`. A new port must be free; a port the running release holds is fine (for example 127.0.0.1:8880 to 0.0.0.0:8880).

## API keys

Clients' keys are named, in `/srv/optimized-kokoro/secrets/keys` (root only); `server.toml` is generated from it. Give each client its own key, so one can't use up another's capacity:

```sh
sudo deploy/keys.sh list                              # names and limits, never the keys
sudo deploy/keys.sh add lion-reader --max-streams 8   # prints the new key once
sudo deploy/keys.sh add batch-jobs --max-priority 50   # its requests can only go below the default (128)
sudo deploy/keys.sh rename imported-1 clawed-abode
sudo deploy/keys.sh remove old-client
```

`--max-streams` caps the speech requests a key may have in progress (default: the server's `max_streams_per_key`, 48). `--max-priority` (0-255) is the highest `priority` the key's requests run at; a request above it runs at it (the response's `X-Priority` header says the priority used), and a request without one runs at the lower of 128 and this (default: the server's `max_priority_per_key`, 255). The demo page's key is fixed at 4 streams and priority 64, below the clients' default (an existing install's `server.toml` gets the priority cap at its next `keys.sh` change). In `secrets/keys` each line is `NAME<tab>KEY<tab>MAX_STREAMS<tab>MAX_PRIORITY`, `-` for a default; lines written before `MAX_PRIORITY` existed have three fields and keep working, with the default. `add` and `remove` recreate only the server container, in the current release's project (a second or two; the worker keeps running) and put the old keys back if it doesn't come up healthy. Keys imported by `install.sh --keys-from` are named `imported-1`, `imported-2`, …; a new install's key is `default`. An install from before `keys` existed gets its keys named that way on the first `keys.sh` run.

## Cloudflare Tunnel (optional)

A `cloudflared` container can publish the server through a [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/), with no port open on the host. It runs like the others: gVisor, nobody, read-only, no capabilities. It shares the compose network with the server and demo, and needs outbound internet access to reach Cloudflare. `install.sh` and `update.sh` wait for it to register with Cloudflare, and roll back if it doesn't within 2 minutes.

1. In Cloudflare Zero Trust, create a tunnel (Networks → Tunnels → cloudflared). Copy the token from the install command it shows (the long string after `--token`).
2. Add a public hostname to the tunnel, e.g. `tts.example.com`, with service type HTTP and URL `172.30.88.2:8880` (the server's address on the compose network; service names don't resolve under gVisor). Every release's server gets this address, so updates don't change it.
3. Protect the hostname with Cloudflare Access: add a self-hosted application for it with a policy whose action is **Service Auth**, allowing a service token you create. Clients send that token's `CF-Access-Client-Id` and `CF-Access-Client-Secret` headers as well as the server's API key. The server assumes unauthenticated traffic is filtered before it (`crates/server/src/config.rs`), so don't skip this.
4. On the host, after `install.sh` (the token goes into its `secrets/` directory; the `<<<` form keeps it out of the process list, but it lands in your shell history unless you prefix the line with a space or clear it):
   ```sh
   sudo install -m 0640 -o root -g 65534 /dev/stdin /srv/optimized-kokoro/secrets/cloudflared.token <<< 'TOKEN'
   echo KOKORO_CLOUDFLARED=1 | sudo tee -a /srv/optimized-kokoro/settings.env
   sudo deploy/update.sh
   ```
5. Check: `sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml logs cloudflared` shows "Registered tunnel connection", and `curl -s -o /dev/null -w '%{http_code}\n' https://tts.example.com/health` prints 403 without the Access headers and 200 with them.

Cloudflare terminates TLS, so it sees the request text. `KOKORO_CLOUDFLARED=0` and `update.sh` turn the tunnel off again.

## Uninstall

```sh
sudo docker compose -f /srv/optimized-kokoro/current/compose.yaml down --volumes
# Anything an interrupted update left: the Compose projects optimized-kokoro and optimized-kokoro-*.
sudo docker ps -a --format '{{.ID}} {{.Label "com.docker.compose.project"}}' | awk '$2 ~ /^optimized-kokoro/ {print $1}' | xargs -r sudo docker rm -f
sudo docker network ls --format '{{.Name}}' | grep '^optimized-kokoro' | xargs -r sudo docker network rm
sudo docker volume ls --format '{{.Name}}' | grep '^optimized-kokoro' | xargs -r sudo docker volume rm
sudo docker image ls --filter 'reference=optimized-kokoro-*' -q | sort -u | xargs -r sudo docker image rm -f
sudo rm -rf /srv/optimized-kokoro
```

Then, if nothing else uses them, remove `runsc-kokoro-uds-create`, `runsc-kokoro-uds-open` and `runsc-kokoro-no-uds` from `/etc/docker/daemon.json` (the original is in `daemon.json.bak-kokoro`) and run `sudo systemctl reload docker`.

## Layout

`/srv/optimized-kokoro`, all root-owned (see the top of `lib.sh`):
- `settings.env`: the settings above. Any other line, apart from comments, is an error.
- `models/` and `unidic/SHA256.dic`: world-readable data. The server memory-maps the dictionary, so a new one (a new checksum in `fetch_unidic.sh`) is a new file next to the old one. Each release's `.env` names its own (`KOKORO_UNIDIC_FILE`), so neither the running server nor a rollback sees it change. Dictionaries that no kept release names are deleted.
- `secrets/`: `server.toml` (listen address, socket path and API keys), `demo.key` and, with the tunnel, `cloudflared.token`.
- `resolv.conf`: for cloudflared, written when the tunnel is on.
- `releases/REL/`: `compose.yaml` (a copy of `deploy/compose.yaml`), `.env` (`KOKORO_ROOT`, `KOKORO_RELEASE`, `KOKORO_UNIDIC_FILE`, `COMPOSE_PROJECT_NAME` and the settings) and `cache-seed/`. Images are `optimized-kokoro-{worker,server,demo}:REL`. The release's Compose project is `optimized-kokoro-rel` (REL in lower case), with the volume `optimized-kokoro-rel_sock` and the network `optimized-kokoro-rel_kokoro`. A plain `docker compose -f releases/REL/compose.yaml` finds it in the `.env`.
- `current`: a symlink to the running release.
- `.lock`: held while `install.sh`, `update.sh` or `keys.sh` runs.

The scripts ignore inherited `KOKORO_*`, `COMPOSE_*` and `DOCKER_*` variables (except `KOKORO_BUILD_USER`), so `sudo -E` can't change what Compose sees. If either script is interrupted (Ctrl-C) or fails after swapping, it puts the previous release back, or takes a first install down again.

`deploy/dry-run/test.sh` runs `install.sh`, `update.sh` and `keys.sh` as any user, without Docker or a GPU, against fakes (`deploy/dry-run/bin`), through success, failures, and Ctrl-C at each step of an update. CI runs it.

## Notes

- `worker/run.sh` (rootless podman) is still how to run the worker by hand, and how `worker/bench/` runs it.
- The server image keeps the espeak-ng data at `/src/crates/espeak-ng/espeak-ng-data`, the path it was built at, because `kokoro-espeak-ng` finds it through `CARGO_MANIFEST_DIR` at run time.
