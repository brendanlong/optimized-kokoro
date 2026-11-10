#!/bin/bash
# gpuc job: packed decoder rows against main's length groups, interleaved so load from other users
# of the GPU hits both alike.
#   scale  in process, main's bench/batch_scale.py and this one, alternating ROUNDS times: corpus
#          batches and a profile at batch 8
#   ab     both workers up at once (--max-batch AB_BATCH), closed-loop load alternating between them
# MAIN is a copy of main's worker/ (git archive origin/main worker | tar -x -C bench/bin/main).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
PY=$PWD/.venv/bin/python
MAIN=${MAIN:-$PWD/bench/bin/main/worker}
STEPS=${STEPS:-scale ab}
quiet() { grep -E '^\{"wall|"gpu_busy_share|"unprofiled_wall_ms|^  "(triton|inductor|matmul|LSTM|aten|memcpy|attention|other)'; }
cache() { export TRITON_CACHE_DIR=/tmp/kc-$1/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-$1/inductor; }

if [[ " $STEPS " == *" scale "* ]]; then
    for round in $(seq "${ROUNDS:-2}"); do
        for which in main packed; do
            if [ "$which" = main ]; then dir=$MAIN; else dir=$PWD; fi
            cache "$which"
            echo "== scale $which, round $round"
            "$PY" "$dir/bench/batch_scale.py" "$MODELS" --steps mixed profile --max-batch 32 --frame-budget 2400 \
                --profile-batches 8 --json "results/scale-$which-$round.json" 2>&1 | quiet
        done
    done
fi

start_worker() {  # label dir -> WORKER_PID
    local label=$1 dir=$2
    rm -rf "/tmp/sock-$label" && mkdir -p "/tmp/sock-$label"
    cache "$label"
    (cd "$dir" && exec "$PY" -m kokoro_worker.server --socket "/tmp/sock-$label/worker.sock" --models "$MODELS" \
        --max-batch "${AB_BATCH:-8}") > "results/ab-$label.worker.log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "results/ab-$label.worker.log"; do
        sleep 1
        kill -0 $WORKER_PID 2> /dev/null || { echo "$label died"; tail -5 "results/ab-$label.worker.log"; return 1; }
    done
}

if [[ " $STEPS " == *" ab "* ]]; then
    start_worker main "$MAIN" || exit 1
    main_pid=$WORKER_PID
    start_worker packed "$PWD" || exit 1
    for round in $(seq "${AB_ROUNDS:-3}"); do
        for label in main packed; do
            "$PY" bench/load.py "/tmp/sock-$label/worker.sock" "$MODELS/config.json" --concurrency 16 64 \
                --requests 800 --warmup "$([ "$round" = 1 ] && echo 200 || echo 0)" \
                --json "results/ab-$label-$round.json" > /dev/null 2>&1
            "$PY" -c "
import json, sys
d = json.load(open(sys.argv[1]))
print('ab', sys.argv[2], sys.argv[3], ' '.join(f\"c{r['concurrency']}: {r['x_realtime']}x p50 {r['latency_ms']['p50']} ms\" for r in d['runs']))
" "results/ab-$label-$round.json" "$label" "$round"
        done
    done
    kill "$main_pid" "$WORKER_PID"
    wait
fi
echo "done"
