#!/bin/bash
# gpuc job: bench/batch_first.py, the worker's packed frame side against a batch-first one with the
# same layers: compiled cold (empty caches), compiled warm (reverse order, the caches the cold runs
# wrote), then eager. Untracked input: bench/bin/students.pt (kokoro_worker.convert --students).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
RUN=$(mktemp -d "$HOME/.cache/kw-bf-XXXX")
trap 'rm -rf "$RUN"' EXIT
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
PY=$PWD/.venv/bin/python
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models-lstm-free
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv | tee results/gpu.txt
if [ ! -f "$MODELS/voices.safetensors" ]; then
    "$PY" -m kokoro_worker.convert "$MODELS" --students bench/bin/students.pt || exit 1
fi
bench() { # variant mode kind
    export TRITON_CACHE_DIR=$RUN/$1/triton TORCHINDUCTOR_CACHE_DIR=$RUN/$1/inductor
    echo "== $1 $2 $3"
    "$PY" bench/batch_first.py "$MODELS" "$1" "$2" "results/$1-$2-$3.json" 2>&1 | grep -E '^\{|^peak|Error|Traceback'
}
for v in packed batchfirst; do bench "$v" compiled cold; done
for v in batchfirst packed; do bench "$v" compiled warm; done
for v in packed batchfirst; do bench "$v" eager run; done
echo done
