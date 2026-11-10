#!/bin/bash
# gpuc job: the chunk-packing listening test (bench/pack_listen.py). A worker at --max-chunk-seconds 10
# serves the demo set's server binaries, one at 60 s the default set's; then every chunk is rendered
# alone for its durations, decode time is measured at batch 1, and Whisper small.en reads every
# rendering. Inputs: bench/pack_listen/plan.json (pack_listen.py plan), bench/bin/kokoro-server-VARIANT
# for each variant, and bench/bin/espeak-ng-data, with the espeak-ng-data path the binaries were
# built with in ESPEAK_DATA_LINK or bench/bin/espeak-data-link. Results in results/pack_listen/.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
OUT=results/pack_listen
mkdir -p $OUT
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
export TRITON_CACHE_DIR=/tmp/kc-pack/triton TORCHINDUCTOR_CACHE_DIR=/tmp/kc-pack/inductor
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    .venv/bin/python -m kokoro_worker.convert "$MODELS" || exit 1
fi
PY=$PWD/.venv/bin/python
PLAN=bench/pack_listen/plan.json
ESPEAK_DATA_LINK=${ESPEAK_DATA_LINK:-$(cat bench/bin/espeak-data-link)}
# A link left by an earlier job points into its (deleted) workdir; a real directory is the data.
if [ -L "$ESPEAK_DATA_LINK" ] || [ ! -e "$ESPEAK_DATA_LINK" ]; then
    mkdir -p "$(dirname "$ESPEAK_DATA_LINK")"
    ln -sfn "$PWD/bench/bin/espeak-ng-data" "$ESPEAK_DATA_LINK"
fi

start_worker() {  # label cap -> WORKER_PID, socket /tmp/sock-pack-$label/worker.sock
    rm -rf "/tmp/sock-pack-$1" && mkdir -p "/tmp/sock-pack-$1"
    "$PY" -m kokoro_worker.server --socket "/tmp/sock-pack-$1/worker.sock" --models "$MODELS" --max-chunk-seconds "$2" \
        > "$OUT/$1.worker.log" 2>&1 &
    WORKER_PID=$!
    until grep -q "listening on" "$OUT/$1.worker.log"; do
        sleep 1
        kill -0 $WORKER_PID 2> /dev/null || { echo "worker $1 died"; tail -5 "$OUT/$1.worker.log"; exit 1; }
    done
}

serve() {  # set label variants...
    local set=$1 label=$2
    shift 2
    for variant in "$@"; do
        port=$((9000 + RANDOM % 900))
        printf 'listen = "127.0.0.1:%s"\nworker_socket = "/tmp/sock-pack-%s/worker.sock"\n' "$port" "$label" > "$OUT/$set-$variant.toml"
        KOKORO_API_KEYS=bench-key RUST_LOG=warn "bench/bin/kokoro-server-$variant" --config "$OUT/$set-$variant.toml" \
            > "$OUT/$set-$variant.server.log" 2>&1 &
        server=$!
        until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
            sleep 0.5
            kill -0 $server 2> /dev/null || { echo "server $variant died"; tail -3 "$OUT/$set-$variant.server.log"; break; }
        done
        "$PY" bench/pack_listen.py http "http://127.0.0.1:$port" bench-key $PLAN $OUT "$set" "$variant"
        kill $server
        wait $server 2> /dev/null
    done
}

start_worker cap10 10
serve demo cap10 today min50 g30 g60
kill $WORKER_PID
wait $WORKER_PID 2> /dev/null

start_worker cap60 60
serve default cap60 today min50 g90 g130 g200
"$PY" bench/pack_listen.py worker /tmp/sock-pack-cap60/worker.sock $PLAN $OUT
kill $WORKER_PID
wait $WORKER_PID 2> /dev/null
"$PY" bench/pack_listen.py asr $PLAN $OUT
echo "done"
# gpuc submit bench/job_pack_listen.yaml --host local   (from worker/)
