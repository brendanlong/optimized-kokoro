#!/bin/bash
# gpuc job: the clause-split listening comparison (bench/clause_joins.py). A worker at
# --max-chunk-seconds 30 serves each server binary in SERVERS in turn (bench/bin/kokoro-server-LABEL,
# with espeak-ng-data in bench/bin/ and the path the binaries were built with in ESPEAK_DATA_LINK or
# bench/bin/espeak-data-link);
# then a worker at 60 s renders every sentence whole, and short clauses alone and in context. Results in
# results/clause/.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results/clause
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
export TRITON_CACHE_DIR=/tmp/kc-clause/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-clause/inductor
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
ESPEAK_DATA_LINK=${ESPEAK_DATA_LINK:-$(cat bench/bin/espeak-data-link)}
# A link left by an earlier job points into its (deleted) workdir; a real directory is the data.
if [ -L "$ESPEAK_DATA_LINK" ] || [ ! -e "$ESPEAK_DATA_LINK" ]; then
    mkdir -p "$(dirname "$ESPEAK_DATA_LINK")"
    ln -sfn "$PWD/bench/bin/espeak-ng-data" "$ESPEAK_DATA_LINK"
fi

start_worker() {  # label cap -> WORKER_PID, socket /tmp/sock-$label/worker.sock
    rm -rf "/tmp/sock-$1" && mkdir -p "/tmp/sock-$1"
    "$PY" -m kokoro_worker.server --socket "/tmp/sock-$1/worker.sock" --models "$MODELS" --max-chunk-seconds "$2" \
        > "results/clause/$1.worker.log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "results/clause/$1.worker.log"; do
        sleep 1
        kill -0 $WORKER_PID 2> /dev/null || { echo "worker $1 died"; tail -5 "results/clause/$1.worker.log"; exit 1; }
    done
}

start_worker cap30 30
for label in ${SERVERS:-main b c}; do
    port=$((9000 + RANDOM % 900))
    printf 'listen = "127.0.0.1:%s"\nworker_socket = "/tmp/sock-cap30/worker.sock"\n' "$port" > "results/clause/$label.toml"
    KOKORO_API_KEYS=bench-key RUST_LOG=warn "bench/bin/kokoro-server-$label" --config "results/clause/$label.toml" \
        > "results/clause/$label.server.log" 2>&1 &
    server=$!
    until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
        sleep 0.5
        kill -0 $server 2> /dev/null || { echo "server $label died"; tail -3 "results/clause/$label.server.log"; break; }
    done
    "$PY" bench/clause_joins.py http "http://127.0.0.1:$port" bench-key bench/clause/sentences.json results/clause "$label"
    kill $server
    wait $server 2> /dev/null
done
kill $WORKER_PID
wait $WORKER_PID 2> /dev/null

start_worker cap60 60
"$PY" bench/clause_joins.py worker /tmp/sock-cap60/worker.sock bench/clause/sentences.json results/clause
# Clauses alone vs in their sentence (bench/clause_short.py), for the minimum chunk size.
"$PY" bench/clause_short.py /tmp/sock-cap60/worker.sock bench/clause/short.json results/clause | tail -n 1
kill $WORKER_PID
wait $WORKER_PID 2> /dev/null
echo "done"
