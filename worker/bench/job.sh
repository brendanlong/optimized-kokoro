#!/bin/bash
# gpuc job: GPU equivalence checks, listening pairs, and worker benchmarks in and out of gVisor.
#   STEPS="equiv pairs bench" (default all); CONFIGS overrides the benchmark list.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1

MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
STEPS=${STEPS:-equiv pairs bench}
nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu --format=csv | tee results/gpu.txt

if [[ " $STEPS " == *" equiv "* ]]; then
    echo "== equivalence (fp16 eager)"
    uv run --no-sync python bench/check_equivalence.py "$MODELS" --device cuda --half --n 16 2>&1 | tee results/equiv-eager.txt
    echo "== equivalence (fp16 compiled), NaN scan"
    uv run --no-sync python bench/check_equivalence.py "$MODELS" --device cuda --half --compile --n 16 --nan-lengths 300 2>&1 | tee results/equiv-compiled.txt
fi

if [[ " $STEPS " == *" pairs "* ]]; then
    echo "== listening pairs"
    uv run --no-sync python bench/make_pairs.py "$MODELS" results/pairs 2>&1 | tail -8
fi

# label  where(sandbox|host)  worker flags
CONFIGS=${CONFIGS:-"
sandbox-b1 sandbox --max-batch 1
sandbox-b8 sandbox --max-batch 8
sandbox-b8-plain sandbox --max-batch 8 --no-exact
sandbox-b16 sandbox --max-batch 16
host-b1 host --max-batch 1
host-b8 host --max-batch 8
"}

bench_one() {
    local label=$1 where=$2
    shift 2
    local sock=results/sock-$label
    rm -rf "$sock"
    mkdir -p "$sock"
    echo "== $label ($where): $*"
    local pid
    if [ "$where" = sandbox ]; then
        KOKORO_WORKER_NAME=kw-bench-$label ./run.sh serve "$sock" "$MODELS" "$@" > "results/$label.worker.log" 2>&1 &
        pid=$!
    else
        local cache
        cache=$(mktemp -d)
        TRITON_CACHE_DIR=$cache/triton TORCHINDUCTOR_CACHE_DIR=$cache/inductor \
            PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY \
            uv run --no-sync python -m kokoro_worker.server --socket "$sock/worker.sock" --models "$MODELS" "$@" \
            > "results/$label.worker.log" 2>&1 &
        pid=$!
    fi
    # shellcheck disable=SC2086  # LOAD_ARGS is a list of flags, split on purpose
    uv run --no-sync python bench/load.py "$sock/worker.sock" "$MODELS/config.json" --json "results/$label.json" ${LOAD_ARGS:-} \
        2>&1 | tee "results/$label.load.txt"
    nvidia-smi --query-gpu=memory.used --format=csv,noheader >> "results/$label.load.txt"
    if [ "$where" = sandbox ]; then
        podman stop -t 5 "kw-bench-$label" > /dev/null 2>&1
    else
        kill "$pid"
    fi
    wait "$pid" 2> /dev/null
    grep -E "ready in|connection done|Error|error" "results/$label.worker.log" | tail -5
}

if [[ " $STEPS " == *" bench "* ]]; then
    while read -r label where flags; do
        [ -z "$label" ] && continue
        # shellcheck disable=SC2086
        bench_one "$label" "$where" $flags
    done <<< "$CONFIGS"
fi
echo "done"
