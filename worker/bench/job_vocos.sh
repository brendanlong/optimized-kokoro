#!/bin/bash
# gpuc job: this checkout's worker (distilled, batch-first: ConvSeqs for the LSTMs, per-frame F0/N
# predictor, Vocos-style decoder) uncompiled ("eager", --no-compile) and compiled ("compiled"), against
# MAIN's (compiled), at --max-batch 8:
#   startup  cold (empty compile caches) and warm (the caches the cold start wrote), each worker in turn
#   ab       throughput: all three up at once, lookahead-shaped load at 1, 16 and 64 streams alternating
#            between them, ROUNDS rounds (bench/job_ab.sh's method)
# Untracked inputs: bench/bin/main/worker (git archive origin/main worker, unpacked there) and the
# distilled checkpoints bench/bin/{students,frame_side,decoder}.pt (kokoro_worker.convert's --students,
# --frame-side and --decoder).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
RUN=$(mktemp -d "$HOME/.cache/kw-vc-XXXX") # short: socket paths are limited to 108 bytes
trap 'kill $(jobs -p) 2> /dev/null; rm -rf "$RUN"' EXIT
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
PY=$PWD/.venv/bin/python
MAIN=$PWD/bench/bin/main/worker
DATA=${GPUC_DATA_DIR:-$HOME/.cache}
NEW_MODELS=$RUN/models
declare -A MODELS=([main]=$DATA/kokoro-worker-models [eager]=$NEW_MODELS [compiled]=$NEW_MODELS)
declare -A DIR=([main]=$MAIN [eager]=$PWD [compiled]=$PWD)
declare -A FLAGS=([main]="" [eager]="--no-compile" [compiled]="")
LABELS=(main eager compiled)
STEPS=${STEPS:-startup ab}
nvidia-smi --query-gpu=name,driver_version,memory.used --format=csv | tee results/gpu.txt

if [ ! -f "${MODELS[main]}/voices.safetensors" ]; then
    (cd "$MAIN" && "$PY" -m kokoro_worker.convert "${MODELS[main]}") || exit 1
fi
"$PY" -m kokoro_worker.convert "$NEW_MODELS" --students bench/bin/students.pt \
    --frame-side bench/bin/frame_side.pt --decoder bench/bin/decoder.pt || exit 1

start_worker() { # label log -> WORKER_PID, START_S (seconds to listening)
    local label=$1 log=$2 t0=$EPOCHREALTIME
    mkdir -p "$RUN/$label"
    export TRITON_CACHE_DIR=$RUN/$label/triton TORCHINDUCTOR_CACHE_DIR=$RUN/$label/inductor
    (cd "${DIR[$label]}" && exec "$PY" -m kokoro_worker.server --socket "$RUN/$label/worker.sock" \
        --models "${MODELS[$label]}" --max-batch 8 ${FLAGS[$label]}) > "$log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "$log"; do
        sleep 0.5
        kill -0 $WORKER_PID 2> /dev/null || {
            echo "$label died"
            tail -5 "$log"
            return 1
        }
    done
    local us=$((${EPOCHREALTIME/./} - ${t0/./}))
    START_S=$((us / 1000000)).$(((us / 100000) % 10))
}

if [[ " $STEPS " == *" startup "* ]]; then
    for kind in cold warm; do
        for label in "${LABELS[@]}"; do
            [ "$kind" = warm ] || rm -rf "${RUN:?}/$label"
            log=results/startup-$kind-$label.log
            start_worker "$label" "$log" || exit 1
            kill "$WORKER_PID"
            wait "$WORKER_PID" 2> /dev/null
            loaded=$(grep -oE 'loaded in [0-9.]+ s' "$log")
            shapes=$(grep -oE 'warmup \[[0-9, ]+\]: [0-9.]+ s' "$log" | sed -E 's/warmup //' | paste -sd' ')
            echo "startup $kind $label: $START_S s ($loaded; warmup $shapes)" | tee -a results/startup.txt
        done
    done
fi

if [[ " $STEPS " == *" ab "* ]]; then
    pids=()
    for label in "${LABELS[@]}"; do
        start_worker "$label" "results/ab-$label.worker.log" || exit 1
        pids+=("$WORKER_PID")
    done
    for round in $(seq "${ROUNDS:-3}"); do
        for label in "${LABELS[@]}"; do
            out=results/ab-$label-$round.json
            warmup=0
            [ "$round" = 1 ] && warmup=400
            "$PY" bench/lookahead_load.py "$RUN/$label/worker.sock" "${MODELS[$label]}/config.json" \
                --concurrency 1 16 64 --warmup "$warmup" --json "$out" > "${out%.json}.log" 2>&1
            "$PY" bench/summarize_load.py "$out" "$label" "$round" | tee -a results/ab.txt
        done
    done
    kill "${pids[@]}"
    wait
    grep -hiE "non-finite|out of memory|error" results/ab-*.worker.log | sort | uniq -c | head
fi
echo "done"
