#!/bin/bash
# gpuc job: waiting chunks in rows of their own (this branch) against main's rows, which mix them
# with prefetch. Both workers up at once (--max-batch AB_BATCH), and lookahead-shaped closed-loop
# load (bench/lookahead_load.py) alternating between them, so load from other users of the GPU hits
# both alike. MAIN is a copy of main's worker/ (git archive origin/main worker | tar -x -C bench/bin/main).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
PY=$PWD/.venv/bin/python
MAIN=${MAIN:-$PWD/bench/bin/main/worker}
LOADS=${LOADS:-front fixed}
cache() { export TRITON_CACHE_DIR=/tmp/kc-$1/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-$1/inductor; }

start_worker() {  # label dir -> WORKER_PID
    local label=$1 dir=$2
    rm -rf "/tmp/sock-$label" && mkdir -p "/tmp/sock-$label"
    cache "$label"
    (cd "$dir" && exec "$PY" -m kokoro_worker.server --socket "/tmp/sock-$label/worker.sock" --models "$MODELS" \
        --max-batch "${AB_BATCH:-8}") > "results/$label.worker.log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "results/$label.worker.log"; do
        sleep 1
        kill -0 $WORKER_PID 2> /dev/null || { echo "$label died"; tail -5 "results/$label.worker.log"; return 1; }
    done
}

start_worker main "$MAIN" || exit 1
main_pid=$WORKER_PID
start_worker rows "$PWD" || exit 1
for round in $(seq "${ROUNDS:-3}"); do
    for load in $LOADS; do
        for label in main rows; do
            out="results/$label-$load-$round.json"
            "$PY" bench/lookahead_load.py "/tmp/sock-$label/worker.sock" "$MODELS/config.json" --model "$load" \
                --concurrency 16 64 --warmup "$([ "$round" = 1 ] && echo 400 || echo 0)" --json "$out" > /dev/null 2>&1
            "$PY" -c "
import json, sys
for r in json.load(open(sys.argv[1]))['runs']:
    w, p = r['waiting_ms'], r['prefetch_ms']
    print(*sys.argv[2:], f\"c{r['concurrency']}: {r['x_realtime']}x, waiting n {w['n']} p50 {w.get('p50')} p90 {w.get('p90')} ms, prefetch p50 {p.get('p50')} p90 {p.get('p90')} ms\")
" "$out" "$label" "$load" "$round"
            grep "connection done" "results/$label.worker.log" | tail -1
        done
    done
done
kill "$main_pid" "$WORKER_PID"
wait
echo "done"
