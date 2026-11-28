#!/bin/bash
# gpuc job: is the GPU the limit? This checkout's worker, compiled and uncompiled, at --max-batch 8
# and 32, under 64 closed-loop streams, with nvidia-smi's utilization (the share of time a kernel
# was running; the card also carries the desktop and production Kokoro, idle here) sampled twice a
# second and the worker process's CPU time over the same stretch.
# Untracked inputs: bench/bin/{students,frame_side,decoder}.pt.
set -uo pipefail
cd "$(dirname "$0")/.." || exit
RUN=$(mktemp -d "$HOME/.cache/kw-ut-XXXX")
trap 'kill $(jobs -p) 2> /dev/null; rm -rf "$RUN"' EXIT
mkdir -p results
export PYTHONUNBUFFERED=1 PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True CUDA_MODULE_LOADING=LAZY
PY=$PWD/.venv/bin/python
"$PY" -m kokoro_worker.convert "$RUN/models" --students bench/bin/students.pt \
    --frame-side bench/bin/frame_side.pt --decoder bench/bin/decoder.pt || exit 1
ticks=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }

for mode in eager compiled; do
    for batch in 8 32; do
        label=$mode-$batch
        flags=""
        [ "$mode" = eager ] && flags=--no-compile
        export TRITON_CACHE_DIR=$RUN/$mode/triton TORCHINDUCTOR_CACHE_DIR=$RUN/$mode/inductor
        "$PY" -m kokoro_worker.server --socket "$RUN/$label.sock" --models "$RUN/models" --max-batch "$batch" $flags \
            > "results/$label.worker.log" 2>&1 &
        worker=$!
        until grep -q "listening on" "results/$label.worker.log"; do
            sleep 1
            kill -0 $worker 2> /dev/null || { echo "$label died"; tail -5 "results/$label.worker.log"; exit 1; }
        done
        # Warm up, then measure a second pass with nothing new to compile or autotune.
        "$PY" bench/lookahead_load.py "$RUN/$label.sock" "$RUN/models/config.json" --concurrency 64 --warmup 400 \
            > "results/$label.warm.log" 2>&1
        (while :; do nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader,nounits; sleep 0.5; done) \
            > "results/$label.gpu.csv" &
        sampler=$!
        c0=$(cpu_ticks $worker); t0=$EPOCHREALTIME
        "$PY" bench/lookahead_load.py "$RUN/$label.sock" "$RUN/models/config.json" --concurrency 64 --warmup 0 \
            --json "results/$label.json" > "results/$label.log" 2>&1
        c1=$(cpu_ticks $worker); t1=$EPOCHREALTIME
        kill $sampler; kill $worker; wait $worker 2> /dev/null
        "$PY" - "$label" "results/$label.gpu.csv" "results/$label.json" "$c0" "$c1" "$t0" "$t1" "$ticks" <<'PY' | tee -a results/util.txt
import json, sys
label, gpu, load, c0, c1, t0, t1, ticks = sys.argv[1:]
util = sorted(int(line.split(",")[0]) for line in open(gpu) if line.strip())
r = json.load(open(load))
cores = (int(c1) - int(c0)) / int(ticks) / (float(t1) - float(t0))
print(f"{label}: gpu util mean {sum(util) / len(util):.0f}% median {util[len(util) // 2]}% p10 {util[len(util) // 10]}% "
      f"({len(util)} samples); worker CPU {cores:.2f} cores; load {json.dumps(r)[:400]}")
PY
    done
done
echo done
