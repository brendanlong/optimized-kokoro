#!/bin/bash
# gpuc job: startup profile (bench/compile_profile.py) and throughput, this checkout ("new") against
# PREV, a copy of another worker/ (default bench/bin/prev/worker), both on the same models:
#   profile    cold (empty compile caches), then warm (the caches the cold run wrote), each in turn
#   recompile  new kernel variants or recompiles over 300 random requests after warmup (this checkout)
#   ab         both up at once, lookahead-shaped load at 1, 16 and 64 streams alternating, ROUNDS rounds
# Untracked inputs: bench/bin/prev/worker and bench/bin/students.pt (kokoro_worker.convert --students).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
RUN=$(mktemp -d "$HOME/.cache/kw-cp-XXXX") # short: socket paths are limited to 108 bytes
trap 'kill $(jobs -p) 2> /dev/null; rm -rf "$RUN"' EXIT
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
PY=$PWD/.venv/bin/python
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models-lstm-free
declare -A DIR=([prev]=${PREV:-$PWD/bench/bin/prev/worker} [new]=$PWD)
STEPS=${STEPS:-profile recompile ab}
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv | tee results/gpu.txt
if [ ! -f "$MODELS/voices.safetensors" ]; then
    "$PY" -m kokoro_worker.convert "$MODELS" --students bench/bin/students.pt || exit 1
fi
caches() { export TRITON_CACHE_DIR=$RUN/$1/triton TORCHINDUCTOR_CACHE_DIR=$RUN/$1/inductor; }

if [[ " $STEPS " == *" profile "* ]]; then
    for kind in cold warm; do
        for label in prev new; do
            [ "$kind" = warm ] || rm -rf "${RUN:?}/$label"
            caches "$label"
            echo "== profile $kind $label"
            (cd "${DIR[$label]}" && "$PY" "$OLDPWD/bench/compile_profile.py" "$MODELS" 8 \
                "$OLDPWD/results/profile-$kind-$label.json") > "results/profile-$kind-$label.log" 2>&1
            grep -E '^\[|^total' "results/profile-$kind-$label.log"
        done
    done
fi

if [[ " $STEPS " == *" recompile "* ]]; then
    caches new
    "$PY" bench/recompile.py "$MODELS" 8 300 > results/recompile.txt 2>&1
    grep -vE "Warning|warn|too_long" results/recompile.txt | tail -8
fi

start_worker() { # label -> WORKER_PID
    local label=$1 log=results/ab-$1.worker.log
    mkdir -p "$RUN/$label"
    caches "$label"
    (cd "${DIR[$label]}" && exec "$PY" -m kokoro_worker.server --socket "$RUN/$label/worker.sock" \
        --models "$MODELS" --max-batch 8) > "$log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "$log"; do
        sleep 0.5
        kill -0 $WORKER_PID 2> /dev/null || { echo "$label died"; tail -5 "$log"; return 1; }
    done
}

if [[ " $STEPS " == *" ab "* ]]; then
    start_worker prev || exit 1
    prev_pid=$WORKER_PID
    start_worker new || exit 1
    new_pid=$WORKER_PID
    for round in $(seq "${ROUNDS:-3}"); do
        for label in prev new; do
            out=results/ab-$label-$round.json
            warmup=0
            [ "$round" = 1 ] && warmup=400
            "$PY" bench/lookahead_load.py "$RUN/$label/worker.sock" "$MODELS/config.json" \
                --concurrency 1 16 64 --warmup "$warmup" --json "$out" > "${out%.json}.log" 2>&1
            "$PY" bench/summarize_load.py "$out" "$label" "$round" | tee -a results/ab.txt
        done
    done
    kill "$prev_pid" "$new_pid"
    wait
    grep -hiE "non-finite|out of memory|error" results/ab-*.worker.log | sort | uniq -c | head
fi
echo "done"
