#!/bin/bash
# gpuc job: fit af_heart's short rows, then score them on the held-out phrases.
#   gpuc submit train/job.yaml --host local   (from worker/)
# FIT_ARGS goes to fit_short_rows.py (e.g. "--steps 300"); STEPS="fit score" (default both);
# ROWS scores another rows file instead of the one just fitted.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
export PYTHONUNBUFFERED=1
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
mkdir -p results
nvidia-smi --query-gpu=name,driver_version --format=csv,noheader | tee results/gpu.txt
STEPS=${STEPS:-fit score}
quiet() { grep --line-buffered -vE "Warning|warn\(|^ +warnings|^  [a-z_]+ = |Triggered internally"; }
if [[ " $STEPS " == *" fit "* ]]; then
    # shellcheck disable=SC2086
    .venv/bin/python train/fit_short_rows.py "$MODELS" train/phrases.json results/fit --teacher-cache "${GPUC_DATA_DIR:-/tmp}/short-rows-teacher.pt" ${FIT_ARGS:-} 2>&1 \
        | quiet | tee results/fit.txt
    [ "${PIPESTATUS[0]}" = 0 ] || exit 1
fi
if [[ " $STEPS " == *" score "* ]]; then
    .venv/bin/python train/score_short_rows.py "$MODELS" train/phrases.json "${ROWS:-results/fit/rows.safetensors}" \
        results/score 2>&1 | quiet | tee results/score.txt
    [ "${PIPESTATUS[0]}" = 0 ] || exit 1
fi
echo done
