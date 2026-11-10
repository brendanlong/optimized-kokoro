#!/bin/bash
# gpuc job: this checkout's worker ("new") against MAIN's, both up at once at --max-batch 8, with
# lookahead-shaped closed-loop load (bench/lookahead_load.py) at 1, 16 and 64 streams alternating
# between them, so load from other users of the GPU hits both alike. Then this checkout at
# --max-batch 1 against 8 (README.md, Batching).
# MAIN is a copy of main's worker/ (git archive origin/main worker | tar -x -C bench/bin/main).
# Used on 2026-11-10 for a sender thread writing replies off the GPU thread (not kept: no more
# throughput, and waiting chunks took longer; README.md).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
RUN=$(mktemp -d "$HOME/.cache/kw-ab-XXXX") # short: socket paths are limited to 108 bytes
trap 'kill $(jobs -p) 2> /dev/null; rm -rf "$RUN"' EXIT
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
PY=$PWD/.venv/bin/python
MAIN=${MAIN:-$PWD/bench/bin/main/worker}
if [ ! -f "$MODELS/voices.safetensors" ]; then
    "$PY" -m kokoro_worker.convert "$MODELS" || exit 1
fi

start_worker() { # label dir batch -> WORKER_PID
    local label=$1 dir=$2 batch=$3
    mkdir -p "$RUN/$label"
    export TRITON_CACHE_DIR=$RUN/$label/triton TORCHINDUCTOR_CACHE_DIR=$RUN/$label/inductor
    (cd "$dir" && exec "$PY" -m kokoro_worker.server --socket "$RUN/$label/worker.sock" --models "$MODELS" \
        --max-batch "$batch") > "results/$label.worker.log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "results/$label.worker.log"; do
        sleep 1
        kill -0 $WORKER_PID 2> /dev/null || {
            echo "$label died"
            tail -5 "results/$label.worker.log"
            return 1
        }
    done
}

load() { # label round
    local out="results/$1-$2.json" warmup=0
    [ "$2" = 1 ] || [ "$2" = b1 ] && warmup=400
    "$PY" bench/lookahead_load.py "$RUN/$1/worker.sock" "$MODELS/config.json" --concurrency 1 16 64 \
        --warmup "$warmup" --json "$out" > "${out%.json}.log" 2>&1
    "$PY" bench/summarize_load.py "$out" "$1" "$2"
}

start_worker main "$MAIN" 8 || exit 1
main_pid=$WORKER_PID
start_worker new "$PWD" 8 || exit 1
new_pid=$WORKER_PID
for round in $(seq "${ROUNDS:-3}"); do
    for label in main new; do
        load "$label" "$round"
    done
done
kill "$main_pid"
wait "$main_pid"
start_worker b1 "$PWD" 1 || exit 1
for round in b1 b2; do
    for label in b1 new; do
        load "$label" "$round"
    done
done
kill "$new_pid" "$WORKER_PID"
wait
echo "done"
