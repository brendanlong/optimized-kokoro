#!/bin/bash
# gpuc job: batch-of-one diagnosis, then the final gVisor benchmarks on the current code.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
echo "== diag_b1"
uv run --no-sync python bench/diag_b1.py "$MODELS" 2>&1 | grep -E "ms/call|rror" | tee results/diag_b1.txt
STEPS=bench CONFIGS="
final-sandbox-b1 sandbox --max-batch 1
final-sandbox-b8 sandbox --max-batch 8
final-sandbox-b8-plain sandbox --max-batch 8 --no-exact
final-host-b1 host --max-batch 1
final-host-b8 host --max-batch 8
" bash bench/job.sh
