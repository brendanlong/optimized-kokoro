#!/bin/bash
# gpuc job: throughput against batch size, and where the GPU time goes.
#   scale  bench/batch_scale.py in process (SCALE_ARGS)
#   load   the worker over its socket at each --max-batch in LOAD_BATCHES, closed-loop load at
#          LOAD_CONCURRENCY streams (bench/load.py), with nvidia-smi utilization sampled meanwhile
# STEPS="scale load" (default both).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
STEPS=${STEPS:-scale load}
nvidia-smi --query-gpu=name,driver_version,memory.total,clocks.max.sm --format=csv | tee results/gpu.txt
nproc | tee results/nproc.txt

if [[ " $STEPS " == *" scale "* ]]; then
    # shellcheck disable=SC2086
    "$PY" bench/batch_scale.py "$MODELS" ${SCALE_ARGS:-} --json results/scale.json 2>&1 \
        | grep -vE "Warning|warn\(|recompile|diagnose|function:|last reason" | tee results/scale.txt
fi

if [[ " $STEPS " == *" load "* ]]; then
    for mb in ${LOAD_BATCHES:-8 32 64}; do
        echo "== worker --max-batch $mb"
        sock=/tmp/sock-scale-$mb
        rm -rf "$sock" && mkdir -p "$sock"
        "$PY" -m kokoro_worker.server --socket "$sock/worker.sock" --models "$MODELS" --max-batch "$mb" \
            > "results/load-$mb.worker.log" 2>&1 &
        worker=$!
        until grep -q "listening on" "results/load-$mb.worker.log"; do
            sleep 1
            kill -0 $worker 2> /dev/null || { echo "worker died"; tail -5 "results/load-$mb.worker.log"; continue 2; }
        done
        nvidia-smi dmon -s u -d 1 > "results/load-$mb.dmon.txt" &
        dmon=$!
        # shellcheck disable=SC2086
        "$PY" bench/load.py "$sock/worker.sock" "$MODELS/config.json" --concurrency ${LOAD_CONCURRENCY:-16 64 128} \
            --requests "${LOAD_REQUESTS:-1500}" --json "results/load-$mb.json" 2>&1 | tee "results/load-$mb.txt"
        kill $dmon $worker
        wait $dmon $worker 2> /dev/null
        grep -E "connection done" "results/load-$mb.worker.log" | sed 's/^[^ ]* [^ ]* //'
        awk '!/^#/ {n++; s+=$2} END {if (n) printf "mean SM utilization over the run: %.0f%%\n", s/n}' \
            "results/load-$mb.dmon.txt"
    done
fi
echo "done"
