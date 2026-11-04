#!/bin/bash
# gpuc job: first-time vs repeated sizes (bench/first_shape.py) under several settings, in the
# gpuc box (gVisor). VARIANTS overrides the list: "label env=value..." per line.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
uname -r | tee results/uname.txt
nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu --format=csv | tee results/gpu.txt
VARIANTS=${VARIANTS:-"
eager ARGS=--no-compile
eager-v7 ARGS=--no-compile TORCH_CUDNN_V8_API_DISABLED=1
compiled ARGS=--profile TORCH_LOGS=recompiles
compiled-v7 ARGS=--profile TORCH_CUDNN_V8_API_DISABLED=1
"}
while read -r label rest; do
    [ -z "$label" ] && continue
    echo "== $label: $rest"
    cache=$(mktemp -d)
    # shellcheck disable=SC2086
    env TRITON_CACHE_DIR="$cache/triton" TORCHINDUCTOR_CACHE_DIR="$cache/inductor" $rest \
        bash -c 'uv run --no-sync python bench/first_shape.py "$0" --n ${N:-120} $ARGS --json "results/$1.json"' \
        "$MODELS" "$label" > "results/$label.txt" 2>&1
    grep -E "^(torch|load|[0-9]+ sentences|first|repeated|same|  \.\.\.|dynamo|GPU|Traceback|[A-Za-z]*Error)" "results/$label.txt"
    echo "recompile log lines: $(grep -c "Recompiling" "results/$label.txt")"
    rm -rf "$cache"
done <<< "$VARIANTS"
echo done
