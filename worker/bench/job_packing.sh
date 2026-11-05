#!/bin/bash
# gpuc job: packed decoder rows (kokoro_worker/masked.py) against main's length groups.
#   check   packed vs alone with the noise off, fp32 (bench/check_packing.py)
#   equiv   batched vs alone, fp16 compiled, against the seed noise floor (bench/check_equivalence.py)
#   scale   in process: corpus batches and a profile (bench/batch_scale.py)
#   load    main's worker (MAIN_WORKER, a copy of main's worker/) and this one over the socket, at each
#           --max-batch in LOAD_BATCHES and LOAD_CONCURRENCY closed-loop streams (bench/load.py)
# STEPS="check equiv scale load" (default all).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
STEPS=${STEPS:-check equiv scale load}
MAIN_WORKER=${MAIN_WORKER:-$PWD/bench/bin/main/worker}
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader | tee results/gpu.txt
quiet() { grep -vE "Warning|warn\(|recompile|diagnose|function:|last reason|_POSIX_C_SOURCE|define|included from|^ +[|^]|^ +[0-9]+ \|"; }

if [[ " $STEPS " == *" check "* ]]; then
    echo "== packed vs alone, noise off, fp32"
    "$PY" bench/check_packing.py "$MODELS" 2>&1 | quiet | tee results/check.txt
    [ "${PIPESTATUS[0]}" = 0 ] || exit 1
fi

if [[ " $STEPS " == *" equiv "* ]]; then
    echo "== batched vs alone, fp16 compiled"
    "$PY" bench/check_equivalence.py "$MODELS" --device cuda --half --compile --n 16 2>&1 \
        | grep -E "^(B=|vs|non)|Error" | tee results/equiv.txt
fi

if [[ " $STEPS " == *" scale "* ]]; then
    # shellcheck disable=SC2086
    "$PY" bench/batch_scale.py "$MODELS" --steps mixed profile --max-batch 64 --frame-budget 9600 \
        --profile-batches 8 32 ${SCALE_ARGS:-} --json results/scale.json 2>&1 | quiet | tee results/scale.txt
fi

if [[ " $STEPS " == *" load "* ]]; then
    for which in ${LOAD_WHICH:-main packed}; do
        if [ "$which" = main ]; then dir=$MAIN_WORKER; else dir=$PWD; fi
        export TRITON_CACHE_DIR=/tmp/kc-$which/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-$which/inductor
        for mb in ${LOAD_BATCHES:-8 32}; do
            label=$which-b$mb
            echo "== $label"
            sock=/tmp/sock-$label
            rm -rf "$sock" && mkdir -p "$sock"
            (cd "$dir" && exec "$PY" -m kokoro_worker.server --socket "$sock/worker.sock" --models "$MODELS" \
                --max-batch "$mb") > "results/$label.worker.log" 2>&1 &
            worker=$!
            until grep -q "listening on" "results/$label.worker.log"; do
                sleep 1
                kill -0 $worker 2> /dev/null || { echo "worker died"; tail -5 "results/$label.worker.log"; continue 2; }
            done
            # shellcheck disable=SC2086
            "$PY" bench/load.py "$sock/worker.sock" "$MODELS/config.json" --concurrency ${LOAD_CONCURRENCY:-1 16 64} \
                --requests "${LOAD_REQUESTS:-800}" --json "results/$label.json" 2>&1 | tee "results/$label.txt"
            kill $worker
            wait $worker 2> /dev/null
            grep -E "connection done" "results/$label.worker.log" | sed 's/^[^ ]* [^ ]* //'
        done
    done
fi
echo "done"
