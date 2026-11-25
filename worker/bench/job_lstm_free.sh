#!/bin/bash
# gpuc job: this checkout's worker (LSTMs replaced by ConvSeqs, "new") against MAIN's, at --max-batch 8:
#   startup  cold (empty compile caches) and warm (the caches the cold start wrote), each worker in turn
#   recompile  new kernel variants or recompiles over 300 random requests after warmup (this checkout)
#   ab       throughput: both up at once, lookahead-shaped load at 1, 16 and 64 streams alternating
#            between them, ROUNDS rounds (bench/job_ab.sh's method)
# Untracked inputs: bench/bin/main/worker (git archive origin/main worker, unpacked there) and
# bench/bin/students.pt (the ConvSeqs, from brendanlong/kokoro#3's train_te.py).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
RUN=$(mktemp -d "$HOME/.cache/kw-lf-XXXX") # short: socket paths are limited to 108 bytes
trap 'kill $(jobs -p) 2> /dev/null; rm -rf "$RUN"' EXIT
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
PY=$PWD/.venv/bin/python
MAIN=$PWD/bench/bin/main/worker
DATA=${GPUC_DATA_DIR:-$HOME/.cache}
declare -A MODELS=([main]=$DATA/kokoro-worker-models [new]=$DATA/kokoro-worker-models-lstm-free)
declare -A DIR=([main]=$MAIN [new]=$PWD)
STEPS=${STEPS:-startup recompile ab}
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv | tee results/gpu.txt

if [ ! -f "${MODELS[main]}/voices.safetensors" ]; then
    (cd "$MAIN" && "$PY" -m kokoro_worker.convert "${MODELS[main]}") || exit 1
fi
if [ ! -f "${MODELS[new]}/voices.safetensors" ]; then
    "$PY" -m kokoro_worker.convert "${MODELS[new]}" --students bench/bin/students.pt || exit 1
fi

start_worker() { # label log -> WORKER_PID, START_S (seconds to listening)
    local label=$1 log=$2 t0=$EPOCHREALTIME
    mkdir -p "$RUN/$label"
    export TRITON_CACHE_DIR=$RUN/$label/triton TORCHINDUCTOR_CACHE_DIR=$RUN/$label/inductor
    (cd "${DIR[$label]}" && exec "$PY" -m kokoro_worker.server --socket "$RUN/$label/worker.sock" \
        --models "${MODELS[$label]}" --max-batch 8) > "$log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "$log"; do
        sleep 0.5
        kill -0 $WORKER_PID 2> /dev/null || {
            echo "$label died"
            tail -5 "$log"
            return 1
        }
    done
    local us=$((${EPOCHREALTIME/./} - ${t0/./}))
    START_S=$((us / 1000000)).$(((us / 100000) % 10))
}

if [[ " $STEPS " == *" startup "* ]]; then
    for kind in cold warm; do
        for label in main new; do
            [ "$kind" = warm ] || rm -rf "${RUN:?}/$label"
            log=results/startup-$kind-$label.log
            start_worker "$label" "$log" || exit 1
            kill "$WORKER_PID"
            wait "$WORKER_PID" 2> /dev/null
            loaded=$(grep -oE 'loaded in [0-9.]+ s' "$log")
            shapes=$(grep -oE 'warmup \[[0-9, ]+\]: [0-9.]+ s' "$log" | sed -E 's/warmup //' | paste -sd' ')
            echo "startup $kind $label: $START_S s ($loaded; warmup $shapes)" | tee -a results/startup.txt
        done
    done
fi

if [[ " $STEPS " == *" recompile "* ]]; then
    export TRITON_CACHE_DIR=$RUN/new/triton TORCHINDUCTOR_CACHE_DIR=$RUN/new/inductor
    "$PY" bench/recompile.py "${MODELS[new]}" 8 300 > results/recompile.txt 2>&1
    grep -vE "Warning|warn" results/recompile.txt | tail -8
fi

if [[ " $STEPS " == *" ab "* ]]; then
    start_worker main results/ab-main.worker.log || exit 1
    main_pid=$WORKER_PID
    start_worker new results/ab-new.worker.log || exit 1
    new_pid=$WORKER_PID
    for round in $(seq "${ROUNDS:-3}"); do
        for label in main new; do
            out=results/ab-$label-$round.json
            warmup=0
            [ "$round" = 1 ] && warmup=400
            "$PY" bench/lookahead_load.py "$RUN/$label/worker.sock" "${MODELS[$label]}/config.json" \
                --concurrency 1 16 64 --warmup "$warmup" --json "$out" > "${out%.json}.log" 2>&1
            "$PY" bench/summarize_load.py "$out" "$label" "$round" | tee -a results/ab.txt
        done
    done
    kill "$main_pid" "$new_pid"
    wait
    grep -hiE "non-finite|out of memory|error" results/ab-*.worker.log | sort | uniq -c | head
fi
echo "done"
