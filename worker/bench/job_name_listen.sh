#!/bin/bash
# gpuc job: render a listening plan of ids (default experiments/names', bench/name_listen/plan.json) to
# a results directory (default results/name_listen/). PLAN and OUT override them.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
OUT=${OUT:-results/name_listen}
PLAN=${PLAN:-bench/name_listen/plan.json}
mkdir -p "$OUT"
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
SOCK=/tmp/sock-names/worker.sock
rm -rf /tmp/sock-names && mkdir -p /tmp/sock-names
.venv/bin/python -m kokoro_worker.server --socket $SOCK --models "$MODELS" > "$OUT"/worker.log 2>&1 &
WORKER=$!
.venv/bin/python bench/name_listen.py $SOCK "$PLAN" "$OUT"
status=$?
kill $WORKER
wait $WORKER 2> /dev/null
exit $status
