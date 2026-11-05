#!/bin/bash
# gpuc job: the worker's GPU memory and throughput at each --max-chunk-seconds in CAPS (default
# "30 60"), --max-batch 8, ROUNDS times in alternation. For each: real corpus chunks closed-loop at
# 16 and 64 streams (bench/load.py), then the longest requests the front sends for that cap at
# speeds 0.25, 0.5 and 1 (bench/cap_load.py). Memory: the worker's own log (torch's peak reserved,
# per connection) and nvidia-smi's per-process usage, sampled every 0.5 s.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
export TRITON_CACHE_DIR=/tmp/kc-max-frames/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-max-frames/inductor
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
nvidia-smi --query-gpu=name,driver_version,memory.used,memory.total --format=csv,noheader | tee results/gpu.txt

for round in $(seq "${ROUNDS:-2}"); do
    for cap in ${CAPS:-30 60}; do
        label=cap$cap-r$round
        echo "== $label"
        sock=/tmp/sock-$label
        rm -rf "$sock" && mkdir -p "$sock"
        "$PY" -m kokoro_worker.server --socket "$sock/worker.sock" --models "$MODELS" --max-batch 8 \
            --max-chunk-seconds "$cap" > "results/$label.worker.log" 2>&1 &
        worker=$!
        (
            while kill -0 $worker 2> /dev/null; do
                nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits 2> /dev/null |
                    awk -F', *' -v pid=$worker '$1 == pid {print $2}'
                sleep 0.5
            done
        ) > "results/$label.smi" &
        sampler=$!
        until grep -q "listening on" "results/$label.worker.log"; do
            sleep 1
            kill -0 $worker 2> /dev/null || { echo "worker died"; tail -5 "results/$label.worker.log"; continue 2; }
        done
        grep -E "ready in" "results/$label.worker.log" | sed 's/^[^ ]* [^ ]* //'
        "$PY" bench/load.py "$sock/worker.sock" "$MODELS/config.json" --concurrency 16 64 \
            --requests "${LOAD_REQUESTS:-1600}" --json "results/$label.load.json" 2>&1 | tee "results/$label.load.txt"
        sleep 1
        grep -E "connection done" "results/$label.worker.log" | tail -1 | sed 's/^[^ ]* [^ ]* //'
        "$PY" bench/cap_load.py "$sock/worker.sock" "$MODELS/config.json" --cap-seconds "$cap" \
            --requests "${CAP_REQUESTS:-200}" --json "results/$label.cap.json" 2>&1 | tee "results/$label.cap.txt"
        sleep 1
        grep -E "connection done" "results/$label.worker.log" | tail -1 | sed 's/^[^ ]* [^ ]* //'
        kill $worker
        wait $worker 2> /dev/null
        wait $sampler
        echo "nvidia-smi, the worker process: peak $(sort -n "results/$label.smi" | tail -1) MiB"
    done
done
echo "done"
