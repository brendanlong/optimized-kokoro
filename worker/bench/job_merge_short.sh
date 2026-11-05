#!/bin/bash
# gpuc job: the short-sentence merging listening test (bench/merge_short.py). One worker (default
# 60 s cap) serves each server binary bench/bin/kokoro-server-mMIN in turn (MIN 0 is today: nothing
# merges), then renders every merged chunk alone for its durations, then Whisper small.en reads every
# variant. Inputs: bench/merge_short/plan.json (merge_short.py plan), the binaries, and
# bench/bin/espeak-ng-data, with ESPEAK_DATA_LINK the espeak-ng-data path the binaries were built with.
# Results in results/merge_short/.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
OUT=results/merge_short
mkdir -p $OUT
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
export TRITON_CACHE_DIR=/tmp/kc-merge/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-merge/inductor
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
PLAN=bench/merge_short/plan.json
if [ -L "$ESPEAK_DATA_LINK" ] || [ ! -e "$ESPEAK_DATA_LINK" ]; then
    mkdir -p "$(dirname "$ESPEAK_DATA_LINK")"
    ln -sfn "$PWD/bench/bin/espeak-ng-data" "$ESPEAK_DATA_LINK"
fi

rm -rf /tmp/sock-merge && mkdir -p /tmp/sock-merge
"$PY" -m kokoro_worker.server --socket /tmp/sock-merge/worker.sock --models "$MODELS" > $OUT/worker.log 2>&1 &
WORKER_PID=$!
until grep -q "listening on" $OUT/worker.log; do
    sleep 1
    kill -0 $WORKER_PID 2> /dev/null || { echo "worker died"; tail -5 $OUT/worker.log; exit 1; }
done

for min in 0 25 50 80; do
    port=$((9000 + RANDOM % 900))
    printf 'listen = "127.0.0.1:%s"\nworker_socket = "/tmp/sock-merge/worker.sock"\n' "$port" > "$OUT/m$min.toml"
    KOKORO_API_KEYS=bench-key RUST_LOG=warn "bench/bin/kokoro-server-m$min" --config "$OUT/m$min.toml" \
        > "$OUT/m$min.server.log" 2>&1 &
    server=$!
    until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
        sleep 0.5
        kill -0 $server 2> /dev/null || { echo "server m$min died"; tail -3 "$OUT/m$min.server.log"; break; }
    done
    "$PY" bench/merge_short.py http "http://127.0.0.1:$port" bench-key $PLAN $OUT "$min"
    kill $server
    wait $server 2> /dev/null
done
"$PY" bench/merge_short.py worker /tmp/sock-merge/worker.sock $PLAN $OUT
kill $WORKER_PID
wait $WORKER_PID 2> /dev/null
"$PY" bench/merge_short.py asr $PLAN $OUT
echo "done"
# gpuc submit bench/job_merge_short.yaml --host local   (from worker/)
