#!/bin/bash
# gpuc job (kokoro_worker/kernels.py), in the gpuc box, itself a gVisor sandbox with the GPU:
#   tests   pytest
#   layers  the Triton conv1d against cuDNN, layer by layer (bench/kernels_bench.py)
#   first   in-process first-time vs repeated, by stage, kernels vs the cuDNN reference
#   equiv   audio against the cuDNN reference with Whisper, listening clips, and batched vs alone
#   serve   the main worker (MAIN_WORKER, a copy of main's worker/) and this one, at batch 1 and 8:
#           startup, GPU and host memory, first-time vs repeated latency over the socket and through
#           the HTTP server (KOKORO_SERVER_BIN, if set), throughput (bench/load.py)
#   seed    startup from a seeded compile cache, main and this
#   ab      throughput A/B: main and this worker up at once, load alternating between them
# STEPS="tests layers first equiv serve seed ab" (default all).
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
STEPS=${STEPS:-tests layers first equiv serve seed ab}
MAIN_WORKER=${MAIN_WORKER:-$PWD/bench/bin/main}
uname -r | tee results/uname.txt
nvidia-smi --query-gpu=name,driver_version,memory.used,utilization.gpu --format=csv | tee results/gpu.txt
echo "host load: $(cat /proc/loadavg)"
# Not uv run (or a function): a backgrounded worker must be the process that $! names and kill stops.
PY=$PWD/.venv/bin/python
fresh_cache() {
    rm -rf /tmp/kc
    export TRITON_CACHE_DIR=/tmp/kc/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc/inductor
}
gpu_used() { nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits; }
rss() { awk '/VmRSS/ {print int($2 / 1024) " MiB"}' "/proc/$1/status"; }

if [[ " $STEPS " == *" tests "* ]]; then
    uv sync --frozen -q && "$PY" -m pytest -q 2>&1 | tail -3
fi

if [[ " $STEPS " == *" layers "* ]]; then
    echo "== layers"
    fresh_cache
    "$PY" bench/kernels_bench.py 2>&1 | grep -vE "Warning|warn|recompile|diagnose|function:|last reason" \
        | tee results/layers.txt
fi

if [[ " $STEPS " == *" first "* ]]; then
    for args in "" "--reference"; do
        echo "== first_shape ${args:-(kernels)}"
        fresh_cache
        # shellcheck disable=SC2086
        "$PY" bench/first_shape.py "$MODELS" --n 150 --profile $args > "results/first${args}.txt" 2>&1
        grep -E "^(load|first|repeated|same|  \.\.\.|dynamo|GPU|Traceback|[A-Za-z]*Error)" "results/first${args}.txt"
        awk '/REPEATED/{p=1} p' "results/first${args}.txt" | grep -E "cudnn_convolution|_fft_|Self CUDA time total" \
            | cut -c1-60,100-125
    done
fi

if [[ " $STEPS " == *" equiv "* ]]; then
    echo "== equivalence"
    fresh_cache
    "$PY" bench/equiv_kernels.py "$MODELS" --compile --whisper 200 --clips results/clips 2>&1 \
        | grep -E "chunks|seed|Whisper|Error" | tee results/equiv.txt
    echo "== batched vs alone (masks), kernels"
    "$PY" bench/check_equivalence.py "$MODELS" --device cuda --half --compile --n 16 2>&1 | grep -E "^(B=|vs|non)" \
        | tee results/batched.txt
fi

start_worker() {  # label dir flags... -> sets WORKER_PID; logs to results/$label.worker.log
    local label=$1 dir=$2
    shift 2
    rm -rf "/tmp/sock-$label"
    mkdir -p "/tmp/sock-$label"
    (cd "$dir" && exec "$PY" -m kokoro_worker.server --socket "/tmp/sock-$label/worker.sock" --models "$MODELS" "$@") \
        > "results/$label.worker.log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "results/$label.worker.log"; do
        sleep 1
        kill -0 $WORKER_PID 2> /dev/null || { echo "$label died"; tail -5 "results/$label.worker.log"; return 1; }
    done
}

if [[ " $STEPS " == *" serve "* ]] && [ -n "${KOKORO_SERVER_BIN:-}" ] && [ -n "${ESPEAK_DATA_LINK:-}" ]; then
    # The server binary reads espeak-ng-data from the path it was built at.
    mkdir -p "$(dirname "$ESPEAK_DATA_LINK")"
    ln -sfn "$(dirname "$KOKORO_SERVER_BIN")/espeak-ng-data" "$ESPEAK_DATA_LINK"
fi
if [[ " $STEPS " == *" serve "* ]]; then
    while read -r label dir flags; do
        [ -z "$label" ] && continue
        if [ "$dir" = main ]; then dir=$MAIN_WORKER; else dir=$PWD; fi
        echo "== serve $label: ${flags:-}"
        fresh_cache
        before=$(gpu_used)
        t0=$SECONDS
        # shellcheck disable=SC2086
        start_worker "$label" "$dir" $flags || continue
        echo "process start to listening: $((SECONDS - t0)) s; GPU memory used by the worker: $(($(gpu_used) - before)) MiB; RSS $(rss $WORKER_PID)"
        grep -E "ready in" "results/$label.worker.log" | sed 's/^[^ ]* [^ ]* //'
        "$PY" bench/fresh_latency.py worker "/tmp/sock-$label/worker.sock" "$MODELS/config.json" \
            --json "results/$label.fresh.json" 2>&1 | tee "results/$label.fresh.txt"
        if [ -n "${KOKORO_SERVER_BIN:-}" ]; then
            port=$((20000 + RANDOM % 20000))
            printf 'listen = "127.0.0.1:%s"\nworker_socket = "%s"\n' "$port" "/tmp/sock-$label/worker.sock" \
                > "results/$label.server.toml"
            KOKORO_API_KEYS=bench-key RUST_LOG=warn "$KOKORO_SERVER_BIN" --config "results/$label.server.toml" \
                > "results/$label.server.log" 2>&1 &
            server=$!
            until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
                sleep 0.5
                kill -0 $server 2> /dev/null || { echo "server died"; tail -3 "results/$label.server.log"; break; }
            done
            "$PY" bench/fresh_latency.py http "http://127.0.0.1:$port" bench-key --offset 203 \
                --json "results/$label.http.json" 2>&1 | tee "results/$label.http.txt"
            kill $server
            wait $server 2> /dev/null
        fi
        "$PY" bench/load.py "/tmp/sock-$label/worker.sock" "$MODELS/config.json" --concurrency 1 16 --requests 400 \
            --json "results/$label.load.json" 2>&1 | tee "results/$label.load.txt"
        echo "after load: GPU memory used by the worker $(($(gpu_used) - before)) MiB; RSS $(rss $WORKER_PID)"
        kill $WORKER_PID
        wait $WORKER_PID 2> /dev/null
        grep -E "connection done|Traceback" "results/$label.worker.log" | sed 's/^[^ ]* [^ ]* //' | tail -2
    done <<< "${SERVE_CONFIGS:-"
main-b1 main
kernels-b1 .
main-b8 main --max-batch 8
kernels-b8 . --max-batch 8
"}"
fi

if [[ " $STEPS " == *" seed "* ]]; then
    for which in main kernels; do
        if [ "$which" = main ]; then dir=$MAIN_WORKER; else dir=$PWD; fi
        echo "== seeded startup, $which"
        fresh_cache
        rm -rf "/tmp/seed-$which"
        (cd "$dir" && exec "$PY" -m kokoro_worker.server --models "$MODELS" --export-cache "/tmp/seed-$which") \
            > "results/seed-export-$which.log" 2>&1
        grep -E "exported" "results/seed-export-$which.log" | sed 's/^[^ ]* [^ ]* //'
        du -sh "/tmp/seed-$which"
        fresh_cache
        t0=$SECONDS
        KOKORO_CACHE_SEED=/tmp/seed-$which start_worker "seed-$which" "$dir" || continue
        echo "seeded: process start to listening $((SECONDS - t0)) s"
        kill $WORKER_PID
        wait $WORKER_PID 2> /dev/null
    done
fi

if [[ " $STEPS " == *" ab "* ]]; then
    echo "== A/B throughput, batch 1"
    export TRITON_CACHE_DIR=/tmp/kc-ab-main/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-ab-main/inductor
    rm -rf /tmp/kc-ab-main /tmp/kc-ab-kernels
    start_worker ab-main "$MAIN_WORKER" || exit 1
    main_pid=$WORKER_PID
    export TRITON_CACHE_DIR=/tmp/kc-ab-kernels/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-ab-kernels/inductor
    start_worker ab-kernels "$PWD" || exit 1
    for round in $(seq "${AB_ROUNDS:-3}"); do
        for label in main kernels; do
            "$PY" bench/load.py "/tmp/sock-ab-$label/worker.sock" "$MODELS/config.json" --concurrency 1 16 \
                --requests 400 --warmup "$([ "$round" = 1 ] && echo 200 || echo 0)" \
                --json "results/ab-$label-$round.json" > /dev/null 2>&1
            "$PY" -c "
import json, sys
d = json.load(open(sys.argv[1]))
print(sys.argv[2], sys.argv[3], ' '.join(f\"c{r['concurrency']}: {r['x_realtime']}x p50 {r['latency_ms']['p50']} ms\" for r in d['runs']))
" "results/ab-$label-$round.json" "$label" "$round"
        done
    done
    kill "$main_pid" "$WORKER_PID"
    wait
fi
echo "host load: $(cat /proc/loadavg)"
echo "done"
