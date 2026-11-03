#!/bin/bash
# gpuc job: gVisor startup with a cold compile cache vs a seeded one, then a short load run.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
seed=$(mktemp -d)

start() {  # label, extra env...
    local label=$1
    shift
    rm -rf "results/sock-$label"
    mkdir -p "results/sock-$label"
    env "$@" KOKORO_WORKER_NAME="kw-$label" ./run.sh "results/sock-$label" "$MODELS" --max-batch 8 \
        > "results/$label.worker.log" 2>&1 &
    uv run --no-sync python bench/load.py "results/sock-$label/worker.sock" "$MODELS/config.json" \
        --concurrency 1 16 --requests 200 --json "results/$label.json" 2>&1 | tee "results/$label.load.txt"
    podman stop -t 5 "kw-$label" > /dev/null 2>&1
    wait
    grep -E "seeded|exported|ready in|connection done" "results/$label.worker.log"
}

start cache-cold KOKORO_CACHE_EXPORT="$seed"
du -sh "$seed" | tee results/cache-size.txt
start cache-seeded KOKORO_CACHE_SEED="$seed"
rm -rf "$seed"
uv run --no-sync python bench/check_equivalence.py "$MODELS" --device cuda --half --compile --n 16 2>&1 \
    | grep -E "^vs|^B=" | tee results/equiv-compiled-2.txt
echo done
