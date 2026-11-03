#!/bin/bash
# gpuc job: run the whole server for a live trial until cancelled. The worker runs in gVisor through
# run.sh; the Rust front listens on KOKORO_LISTEN with the API key in KOKORO_API_KEY_FILE.
#   KOKORO_SERVER_BIN=... KOKORO_LISTEN=0.0.0.0:PORT KOKORO_API_KEY_FILE=... bash bench/serve.sh
set -uo pipefail
: "${KOKORO_LISTEN:?}" "${KOKORO_API_KEY_FILE:?}" "${KOKORO_SERVER_BIN:?}"
cd "$(dirname "$0")/.."
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
sock=results/sock-serve
rm -rf "$sock"
mkdir -p "$sock"
KOKORO_WORKER_NAME=kw-serve ./run.sh serve "$sock" "$MODELS" > results/serve.worker.log 2>&1 &
worker=$!
server=
trap '[ -n "$server" ] && kill $server 2> /dev/null; podman stop -t 5 kw-serve > /dev/null 2>&1; wait' EXIT
cat > results/server.toml << EOF2
listen = "$KOKORO_LISTEN"
worker_socket = "$sock/worker.sock"
EOF2
KOKORO_API_KEYS=$(cat "$KOKORO_API_KEY_FILE") RUST_LOG=info "$KOKORO_SERVER_BIN" --config results/server.toml > results/serve.server.log 2>&1 &
server=$!
wait -n $worker $server
echo "exited: worker or server"; tail -20 results/serve.worker.log results/serve.server.log
exit 1
