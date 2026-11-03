#!/bin/bash
# Builds the server and worker from this checkout and (re)starts them as systemd user services.
#
#   deploy/deploy.sh [--listen ADDR]     # default 127.0.0.1:8880
#
# State lives in ~/.local/share/optimized-kokoro: the server binary, a copy of worker/run.sh,
# the converted models, UniDic for Japanese (unidic.dic), the compile-cache seed, server.toml and server.env (KOKORO_API_KEYS, mode
# 0600; written once with a new random key, then left alone). Server and worker are always
# updated together, since the protocol between them changes with the code.
#
# Everything is built and staged while the old services keep running; they're stopped only for
# the final renames, and restarted (old or new) if anything fails after that.
set -euo pipefail
cd "$(dirname "$0")/.."
listen=127.0.0.1:8880
if [ $# -gt 0 ]; then
    [ $# -eq 2 ] && [ "$1" = --listen ] || { sed -n '4p' "$0"; exit 2; }
    listen=$2
fi
state=$HOME/.local/share/optimized-kokoro
units=$HOME/.config/systemd/user
new=$state/new
rm -rf "$new"
mkdir -p "$state" "$new/bin" "$new/worker" "$units"

echo "== build"
cargo build --release -p kokoro-server
podman build -t localhost/kokoro-worker:deploy-new -f worker/Containerfile worker

if [ ! -f "$state/models/voices.safetensors" ]; then
    echo "== convert models"
    (cd worker && uv sync --frozen && uv run --no-sync python -m kokoro_worker.convert "$state/models")
fi

# The server maps this file; fetch_unidic.sh skips it if its checksum already matches, and
# otherwise installs the new one by rename, never in place.
echo "== Japanese dictionary (UniDic)"
crates/g2p-ja/tools/fetch_unidic.sh "$state/unidic.dic"

echo "== compile cache (warms up in gVisor next to the running worker, then exits without listening)"
KOKORO_WORKER_IMAGE=localhost/kokoro-worker:deploy-new KOKORO_WORKER_NAME=kokoro-worker-export \
    worker/run.sh export-cache "$new/cache-seed" "$state/models"

echo "== stage"
install -m 0755 target/release/kokoro-server "$new/bin/kokoro-server"
install -m 0755 worker/run.sh "$new/worker/run.sh"
cat > "$new/server.toml" << EOF2
listen = "$listen"
worker_socket = "$XDG_RUNTIME_DIR/optimized-kokoro/worker.sock"
EOF2
if [ ! -f "$state/server.env" ]; then
    key=$(python3 -c 'import secrets; print(secrets.token_urlsafe(24))')
    (umask 077 && echo "KOKORO_API_KEYS=$key" > "$state/server.env")
fi
cp deploy/kokoro-worker.service deploy/kokoro-server.service "$new/"

echo "== swap"
trap 'echo "deploy failed after stopping the services; starting whatever is installed" >&2; systemctl --user daemon-reload; systemctl --user start kokoro-worker.service kokoro-server.service' ERR
systemctl --user stop kokoro-server.service kokoro-worker.service
podman tag localhost/kokoro-worker:deploy-new localhost/kokoro-worker:deploy
for path in bin worker cache-seed server.toml; do
    rm -rf "$state/$path.old"
    [ -e "$state/$path" ] && mv "$state/$path" "$state/$path.old"
    mv "$new/$path" "$state/$path"
done
mv "$new/kokoro-worker.service" "$new/kokoro-server.service" "$units/"
systemctl --user daemon-reload
systemctl --user enable --now kokoro-worker.service kokoro-server.service
trap - ERR
rm -rf "$new" "$state"/*.old

echo "== waiting for /health"
port=${listen##*:}
for _ in $(seq 300); do
    curl -sf -o /dev/null "http://127.0.0.1:$port/health" && { echo "up on $listen"; exit 0; }
    sleep 1
done
echo "not healthy after 5 minutes; see journalctl --user -u kokoro-worker -u kokoro-server" >&2
exit 1
