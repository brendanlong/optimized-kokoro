#!/bin/bash
# Builds the server and worker from this checkout and (re)starts them as systemd user services.
#
#   deploy/deploy.sh [--listen ADDR]     # default 127.0.0.1:8880
#
# State lives in ~/.local/share/optimized-kokoro: the server binary, a copy of worker/run.sh,
# the converted models, the compile-cache seed, server.toml and server.env (KOKORO_API_KEYS, mode
# 0600; written once with a new random key, then left alone). Server and worker are always
# updated together, since the protocol between them changes with the code.
set -euo pipefail
cd "$(dirname "$0")/.."
listen=127.0.0.1:8880
[ "${1:-}" = --listen ] && listen=$2
state=$HOME/.local/share/optimized-kokoro
units=$HOME/.config/systemd/user
mkdir -p "$state/bin" "$units"

echo "== build"
cargo build --release -p kokoro-server
podman build -t localhost/kokoro-worker:deploy -f worker/Containerfile worker

if [ ! -f "$state/models/voices.safetensors" ]; then
    echo "== convert models"
    (cd worker && uv sync --frozen && uv run --no-sync python -m kokoro_worker.convert "$state/models")
fi

echo "== compile cache (warms up in gVisor, then exits without listening)"
rm -rf "$state/cache-seed.new"
KOKORO_WORKER_IMAGE=localhost/kokoro-worker:deploy KOKORO_WORKER_NAME=kokoro-worker-export \
    worker/run.sh export-cache "$state/cache-seed.new" "$state/models"

echo "== install"
systemctl --user stop kokoro-server.service kokoro-worker.service 2> /dev/null || true
rm -rf "$state/cache-seed"
mv "$state/cache-seed.new" "$state/cache-seed"
install -D -m 0755 worker/run.sh "$state/worker/run.sh"
install -m 0755 target/release/kokoro-server "$state/bin/kokoro-server"
cat > "$state/server.toml" << EOF2
listen = "$listen"
worker_socket = "$XDG_RUNTIME_DIR/optimized-kokoro/worker.sock"
EOF2
if [ ! -f "$state/server.env" ]; then
    (umask 077 && echo "KOKORO_API_KEYS=$(python3 -c 'import secrets; print(secrets.token_urlsafe(24))')" > "$state/server.env")
fi
install -m 0644 deploy/kokoro-worker.service deploy/kokoro-server.service "$units/"
systemctl --user daemon-reload
systemctl --user enable --now kokoro-worker.service kokoro-server.service

echo "== waiting for /health"
for _ in $(seq 300); do
    curl -sf -o /dev/null "http://$listen/health" && { echo "up on $listen"; exit 0; }
    sleep 1
done
echo "not healthy after 5 minutes; see journalctl --user -u kokoro-worker -u kokoro-server" >&2
exit 1
