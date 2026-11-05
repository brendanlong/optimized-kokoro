#!/bin/bash
# gpuc job: frames per phoneme id on the corpus (bench/frames_per_id.py).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
.venv/bin/python bench/frames_per_id.py "$MODELS" 2>&1 | grep -v Warning | tee results/frames_per_id.txt
exit "${PIPESTATUS[0]}"
