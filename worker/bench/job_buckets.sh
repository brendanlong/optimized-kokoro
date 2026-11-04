#!/bin/bash
# gpuc job (size buckets), in the gpuc box, which is itself a gVisor sandbox with the GPU:
#   shapes  distinct shapes over 1,000 real requests, exact sizes vs buckets (bench/shape_count.py)
#   equiv   bucketed vs exact audio and durations (bench/equiv_buckets.py)
#   first   in-process first-time vs repeated, by stage (bench/first_shape.py)
#   serve   the worker before (--bucket-ratio 0) and after (default): startup, GPU memory,
#           first-time vs repeated latency over the socket and through the HTTP server
#           (KOKORO_SERVER_BIN, if set), and throughput (bench/load.py)
#   seed    startup from a seeded compile cache (--export-cache, then KOKORO_CACHE_SEED), before/after
#   ab      throughput A/B: a before and an after worker up at once, load runs alternating between
#           them (AB_ROUNDS rounds; AB_FLAGS adds flags to both, e.g. --max-batch 8)
# STEPS="shapes equiv first serve seed ab" (default all); SERVE_CONFIGS overrides the serve list.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
STEPS=${STEPS:-shapes equiv first serve seed ab}
uname -r | tee results/uname.txt
nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu --format=csv | tee results/gpu.txt
# Not uv run (or a function): a backgrounded worker must be the process that $! names and kill stops.
PY=$PWD/.venv/bin/python
fresh_cache() {
    rm -rf /tmp/kc
    export TRITON_CACHE_DIR=/tmp/kc/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc/inductor
}

uv sync --frozen -q && $PY -m pytest -q 2>&1 | tail -3
for _ in 1 2 3; do $PY -m pytest -q tests/test_server.py -k close_early 2>&1 | tail -1; done

if [[ " $STEPS " == *" shapes "* ]]; then
    echo "== shapes"
    $PY bench/shape_count.py "$MODELS" 2>&1 | tee results/shapes-exact.txt
    $PY bench/shape_count.py "$MODELS" --bucket-ratio 0.05 2>&1 | tee results/shapes-buckets.txt
fi

if [[ " $STEPS " == *" equiv "* ]]; then
    echo "== equivalence"
    fresh_cache
    $PY bench/equiv_buckets.py "$MODELS" --compile 2>&1 | tee results/equiv-fp16-compiled.txt
    $PY bench/equiv_buckets.py "$MODELS" --fp32 --n 200 2>&1 | tee results/equiv-fp32-eager.txt
    $PY bench/equiv_buckets.py "$MODELS" --compile --max-batch 8 2>&1 | tee results/equiv-fp16-compiled-b8.txt
fi

if [[ " $STEPS " == *" first "* ]]; then
    for ratio in 0 0.05; do
        echo "== first_shape, bucket ratio $ratio"
        fresh_cache
        $PY bench/first_shape.py "$MODELS" --bucket-ratio $ratio --n 150 2>&1 | grep -vE "^\s*$" | tee "results/first-$ratio.txt"
    done
fi

# label  worker flags
SERVE_CONFIGS=${SERVE_CONFIGS:-"
before-b1 --bucket-ratio 0
after-b1
before-b8 --max-batch 8 --bucket-ratio 0
after-b8 --max-batch 8
"}
serve_one() {
    local label=$1
    shift
    local sock=/tmp/sock-$label
    rm -rf "$sock"
    mkdir -p "$sock"
    fresh_cache
    echo "== serve $label: $*"
    local t0=$SECONDS
    $PY -m kokoro_worker.server --socket "$sock/worker.sock" --models "$MODELS" "$@" > "results/$label.worker.log" 2>&1 &
    local pid=$!
    until grep -q "listening on" "results/$label.worker.log"; do
        sleep 1
        kill -0 $pid 2> /dev/null || { echo "worker died"; tail "results/$label.worker.log"; return; }
    done
    echo "process start to listening: $((SECONDS - t0)) s"
    grep -E "warm|ready in" "results/$label.worker.log" | sed 's/^[^ ]* [^ ]* //'
    $PY bench/fresh_latency.py worker "$sock/worker.sock" "$MODELS/config.json" --json "results/$label.fresh.json" \
        2>&1 | tee "results/$label.fresh.txt"
    local server=
    if [ -n "${KOKORO_SERVER_BIN:-}" ]; then
        local port=$((20000 + RANDOM % 20000))
        printf 'listen = "127.0.0.1:%s"\nworker_socket = "%s"\n' "$port" "$sock/worker.sock" > "results/$label.server.toml"
        KOKORO_API_KEYS=bench-key RUST_LOG=warn "$KOKORO_SERVER_BIN" --config "results/$label.server.toml" \
            > "results/$label.server.log" 2>&1 &
        server=$!
        until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
            sleep 0.5
            kill -0 $server 2> /dev/null || { echo "server died"; tail -3 "results/$label.server.log"; break; }
        done
        # The server holds the worker's only connection; the socket client above has hung up.
        $PY bench/fresh_latency.py http "http://127.0.0.1:$port" bench-key --offset 203 --json "results/$label.http.json" \
            2>&1 | tee "results/$label.http.txt"
        kill $server
        wait $server 2> /dev/null
    fi
    $PY bench/load.py "$sock/worker.sock" "$MODELS/config.json" --concurrency 1 16 --requests 400 \
        --json "results/$label.load.json" 2>&1 | tee "results/$label.load.txt"
    kill $pid
    wait $pid 2> /dev/null
    echo "GPU memory used (whole card) after the worker stopped: $(nvidia-smi --query-gpu=memory.used --format=csv,noheader)"
    grep -E "connection done|Traceback|Error" "results/$label.worker.log" | sed 's/^[^ ]* [^ ]* //' | tail -6
}
if [[ " $STEPS " == *" serve "* ]] && [ -n "${KOKORO_SERVER_BIN:-}" ] && [ -n "${ESPEAK_DATA_LINK:-}" ]; then
    # The server binary reads espeak-ng-data from the path it was built at.
    mkdir -p "$(dirname "$ESPEAK_DATA_LINK")"
    ln -sfn "$(dirname "$KOKORO_SERVER_BIN")/espeak-ng-data" "$ESPEAK_DATA_LINK"
fi
if [[ " $STEPS " == *" serve "* ]]; then
    while read -r label flags; do
        [ -z "$label" ] && continue
        # shellcheck disable=SC2086
        serve_one "$label" $flags
    done <<< "$SERVE_CONFIGS"
fi
if [[ " $STEPS " == *" seed "* ]]; then
    for ratio in 0 0.05; do
        echo "== seeded startup, bucket ratio $ratio"
        fresh_cache
        rm -rf /tmp/seed
        $PY -m kokoro_worker.server --models "$MODELS" --bucket-ratio $ratio --export-cache /tmp/seed \
            > "results/seed-export-$ratio.log" 2>&1
        grep -E "exported" "results/seed-export-$ratio.log" | sed 's/^[^ ]* [^ ]* //'
        du -sh /tmp/seed
        fresh_cache
        rm -rf /tmp/sock-seed
        mkdir -p /tmp/sock-seed
        t0=$SECONDS
        KOKORO_CACHE_SEED=/tmp/seed $PY -m kokoro_worker.server --socket /tmp/sock-seed/worker.sock --models "$MODELS" \
            --bucket-ratio $ratio > "results/seed-serve-$ratio.log" 2>&1 &
        pid=$!
        until grep -q "listening on" "results/seed-serve-$ratio.log"; do
            sleep 1
            kill -0 $pid 2> /dev/null || { echo "worker died"; tail "results/seed-serve-$ratio.log"; break; }
        done
        echo "seeded: process start to listening $((SECONDS - t0)) s"
        grep -E "seeded|warm|ready in" "results/seed-serve-$ratio.log" | sed 's/^[^ ]* [^ ]* //'
        kill $pid
        wait $pid 2> /dev/null
    done
fi
if [[ " $STEPS " == *" ab "* ]]; then
    start_ab() {  # label, flags...
        local label=$1
        shift
        rm -rf "/tmp/sock-$label" "/tmp/kc-$label"
        mkdir -p "/tmp/sock-$label"
        # shellcheck disable=SC2086
        TRITON_CACHE_DIR=/tmp/kc-$label/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-$label/inductor \
            $PY -m kokoro_worker.server --socket "/tmp/sock-$label/worker.sock" --models "$MODELS" ${AB_FLAGS:-} "$@" \
            > "results/ab-$label.worker.log" 2>&1 &
        until grep -q "listening on" "results/ab-$label.worker.log"; do
            sleep 2
            kill -0 $! 2> /dev/null || { echo "$label died"; tail -3 "results/ab-$label.worker.log"; return 1; }
        done
    }
    echo "== A/B throughput, flags: ${AB_FLAGS:-none}"
    start_ab before --bucket-ratio 0 && start_ab after || exit 1
    for round in $(seq "${AB_ROUNDS:-3}"); do
        for label in before after; do
            $PY bench/load.py "/tmp/sock-$label/worker.sock" "$MODELS/config.json" --concurrency 1 16 --requests 400 \
                --warmup $([ "$round" = 1 ] && echo 200 || echo 0) --json "results/ab-$label-$round.json" > /dev/null 2>&1
            $PY -c "
import json, sys
d = json.load(open(sys.argv[1]))
print(sys.argv[2], sys.argv[3], ' '.join(f\"c{r['concurrency']}: {r['x_realtime']}x p50 {r['latency_ms']['p50']} ms\" for r in d['runs']))
" "results/ab-$label-$round.json" "$label" "$round"
        done
    done
    pkill -f "sock-before/worker.sock"
    pkill -f "sock-after/worker.sock"
    wait
fi
echo done
