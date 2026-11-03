#!/bin/bash
# gpuc job: export a compile cache in its own run, then start gVisor workers seeded from it and not,
# under run.sh's default resource limits, each followed by a short load run.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
FLAGS=${FLAGS:---max-batch 8}
seed=$(mktemp -d)

echo "== export-cache"
# shellcheck disable=SC2086
KOKORO_WORKER_NAME=kw-export ./run.sh export-cache "$seed" "$MODELS" $FLAGS > results/cache-export.worker.log 2>&1
echo "export exit $?"
grep -E "exported|listening" results/cache-export.worker.log
du -sh "$seed" | tee results/cache-size.txt

start() {  # label, extra env...
    local label=$1
    shift
    rm -rf "results/sock-$label"
    mkdir -p "results/sock-$label"
    # shellcheck disable=SC2086
    env "$@" KOKORO_WORKER_NAME="kw-$label" ./run.sh serve "results/sock-$label" "$MODELS" $FLAGS \
        > "results/$label.worker.log" 2>&1 &
    uv run --no-sync python bench/load.py "results/sock-$label/worker.sock" "$MODELS/config.json" \
        --concurrency 1 16 --requests 200 --json "results/$label.json" 2>&1 | tee "results/$label.load.txt"
    f=$(ls -d /sys/fs/cgroup/user.slice/user-"$(id -u)".slice/user@"$(id -u)".service/app.slice/kw-"$label"-*.scope 2> /dev/null | head -1)
    [ -n "$f" ] && echo "kw-$label memory.peak $(($(cat "$f/memory.peak") >> 20)) MiB" | tee -a "results/$label.load.txt"
    podman stop -t 5 "kw-$label" > /dev/null 2>&1
    wait
    grep -E "seeded|ready in|connection done" "results/$label.worker.log"
}

start cache-seeded KOKORO_CACHE_SEED="$seed"
rm -rf "$seed"
echo done
