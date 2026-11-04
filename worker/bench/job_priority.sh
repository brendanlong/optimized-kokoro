#!/bin/bash
# gpuc job (issue #26): priorities and load shedding through the HTTP server against the real
# worker. For each worker batch size (BATCHES, default "1 8"), starts the worker; then for each
# workload (WORKLOADS: bench/priority_load.py arguments) and server configuration (SERVER_CONFIGS:
# server.toml settings), a fresh server at RUST_LOG=debug and bench/priority_load.py against it:
# many low-priority streams plus occasional high-priority requests. Needs KOKORO_SERVER_BIN (a
# kokoro-server binary, with its espeak-ng-data next to it) and ESPEAK_DATA_LINK (the
# espeak-ng-data path the binary was built with).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
uname -r | tee results/uname.txt
nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu --format=csv | tee results/gpu.txt
echo "host load: $(cat /proc/loadavg)"
"$PY" -m pytest -q tests/test_scheduler.py tests/test_protocol.py 2>&1 | tail -1

mkdir -p "$(dirname "$ESPEAK_DATA_LINK")"
ln -sfn "$(dirname "$KOKORO_SERVER_BIN")/espeak-ng-data" "$ESPEAK_DATA_LINK"

SECONDS_EACH=${SECONDS_EACH:-60}
# Below the ephemeral range, which the load generator's many connections use up.
next_port=$((12000 + RANDOM % 4000))
WORKLOADS=${WORKLOADS:-"
articles --low-streams 32 --low-chunks 4
sentences --low-streams 64 --low-chunks 1
"}
SERVER_CONFIGS=${SERVER_CONFIGS:-"
defaults
wide max_concurrent_requests=128 max_streams_per_key=128
wide-noshed max_concurrent_requests=128 max_streams_per_key=128 max_estimated_latency_ms=0
"}

# run_one LABEL SOCKET NAME "WORKLOAD ARGS" "SERVER SETTINGS"
run_one() {
    local label=$1 socket=$2 name=$3 load=$4 extra=$5 port server kv
    echo "== $label $name: load ${load}; server ${extra:-defaults}"
    # A new port each run, so a server still closing never holds the next one's.
    next_port=$((next_port + 1))
    port=$next_port
    {
        printf 'listen = "127.0.0.1:%s"\nworker_socket = "%s"\n' "$port" "$socket"
        for kv in $extra; do echo "${kv/=/ = }"; done
    } > "results/$label-$name.server.toml"
    KOKORO_API_KEYS=bench-key-0123456789 RUST_LOG=debug "$KOKORO_SERVER_BIN" \
        --config "results/$label-$name.server.toml" > "results/$label-$name.server.log" 2>&1 &
    server=$!
    until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
        sleep 0.5
        kill -0 $server 2> /dev/null || { echo "server died"; tail -3 "results/$label-$name.server.log"; return; }
    done
    # shellcheck disable=SC2086
    "$PY" bench/priority_load.py "http://127.0.0.1:$port" bench-key-0123456789 --seconds "$SECONDS_EACH" $load \
        --server-log "results/$label-$name.server.log" --json "results/$label-$name.json" 2>&1 \
        | tee "results/$label-$name.txt"
    grep "shed a request" "results/$label-$name.server.log" | tail -2
    kill $server
    wait $server 2> /dev/null
    # The debug lines are kept for the estimate comparison; the rest is noise.
    grep -vE "DEBUG kokoro_server::(http|serve)" "results/$label-$name.server.log" > /tmp/log
    mv /tmp/log "results/$label-$name.server.log"
}

for batch in ${BATCHES:-1 8}; do
    label=b$batch
    echo "== worker --max-batch $batch"
    rm -rf /tmp/kc "/tmp/sock-$label"
    mkdir -p "/tmp/sock-$label"
    export TRITON_CACHE_DIR=/tmp/kc/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc/inductor
    "$PY" -m kokoro_worker.server --socket "/tmp/sock-$label/worker.sock" --models "$MODELS" --max-batch "$batch" \
        > "results/$label.worker.log" 2>&1 &
    worker=$!
    until grep -q "listening on" "results/$label.worker.log"; do
        sleep 2
        kill -0 $worker 2> /dev/null || { echo "worker died"; tail -5 "results/$label.worker.log"; continue 2; }
    done
    while read -r wname wargs; do
        [ -z "$wname" ] && continue
        while read -r cname extra; do
            [ -z "$cname" ] && continue
            run_one "$label" "/tmp/sock-$label/worker.sock" "$wname-$cname" "$wargs" "$extra"
        done <<< "$SERVER_CONFIGS"
    done <<< "$WORKLOADS"
    kill $worker
    wait $worker 2> /dev/null
    grep -E "connection done|Traceback" "results/$label.worker.log" | tail -3
done
echo "host load: $(cat /proc/loadavg)"
echo "done"
