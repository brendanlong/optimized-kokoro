#!/bin/bash
cd "$(dirname "$0")/.." || exit
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
for b in 1 8; do TORCH_LOGS=recompiles uv run --no-sync python bench/diag_b1_server.py "$MODELS" $b > results/diag-b$b.log 2>&1; grep "ms per" results/diag-b$b.log; done
grep -c "Recompiling" results/diag-b8.log
