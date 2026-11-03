#!/bin/bash
# gpuc job: the whole server end to end. The worker runs in gVisor through run.sh, the Rust front
# (KOKORO_SERVER_BIN, built on this host) talks to it, and curl sends the two clients' requests.
# Reports time to first byte and audio length, transcribes the audio, then runs the hostile front
# against the same worker.
#   KOKORO_SERVER_BIN=/abs/path/kokoro-server [KOKORO_WORKER_IMAGE=...] bash bench/e2e.sh
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
if [ ! -f "$MODELS/voices.safetensors" ]; then
    uv run --no-sync python -m kokoro_worker.convert "$MODELS" || exit 1
fi
server_bin=${KOKORO_SERVER_BIN:?set KOKORO_SERVER_BIN to the kokoro-server binary}
sock=results/sock-e2e  # relative: a Unix socket path is limited to 108 bytes
rm -rf "$sock"
mkdir -p "$sock"

echo "== worker (gVisor, batch 1)"
KOKORO_WORKER_NAME=kw-e2e ./run.sh serve "$sock" "$MODELS" > results/e2e.worker.log 2>&1 &
worker=$!
until grep -q "listening on" results/e2e.worker.log; do
    sleep 2
    kill -0 $worker 2> /dev/null || { echo "worker died"; tail results/e2e.worker.log; exit 1; }
done
grep -E "ready in|listening" results/e2e.worker.log

port=$((20000 + RANDOM % 20000))
cat > results/server.toml << EOF
listen = "127.0.0.1:$port"
worker_socket = "$sock/worker.sock"
EOF
KOKORO_API_KEYS=e2e-key RUST_LOG=info "$server_bin" --config results/server.toml > results/e2e.server.log 2>&1 &
server=$!
until curl -sf "http://127.0.0.1:$port/health" > /dev/null; do
    sleep 0.5
    kill -0 $server 2> /dev/null || { echo "server died"; cat results/e2e.server.log; exit 1; }
done
cat results/e2e.server.log

speak() {  # name body
    curl -sS -o "results/$1.pcm" -w "$1: HTTP %{http_code}, first byte %{time_starttransfer} s, total %{time_total} s, %{size_download} bytes\n" \
        -H "Authorization: Bearer e2e-key" -H "Content-Type: application/json" \
        -d "$2" "http://127.0.0.1:$port/v1/audio/speech" | tee -a results/e2e.txt
}
text="The quick brown fox jumps over the lazy dog. Optimized Kokoro speaks this sentence from a sandboxed worker."
speak warmup '{"model": "kokoro", "voice": "af_heart", "input": "Warming up.", "response_format": "pcm", "stream": true}'
speak lion-reader "{\"model\": \"kokoro\", \"voice\": \"af_heart\", \"input\": \"$text\", \"response_format\": \"pcm\", \"stream\": true}"
speak clawed-abode "{\"model\": \"kokoro\", \"input\": \"$text\", \"voice\": \"bm_george\", \"speed\": 1.25, \"response_format\": \"pcm\"}"
curl -sS -w " HTTP %{http_code}\n" -H "Authorization: Bearer e2e-key" -H "Content-Type: application/json" \
    -d '{"input": "Hi.", "voice": "af_heart(2)+af_sky", "response_format": "pcm"}' \
    "http://127.0.0.1:$port/v1/audio/speech" | tee -a results/e2e.txt

uv run --no-sync python - << 'EOF' 2>&1 | tee -a results/e2e.txt
import numpy as np
for name in ["lion-reader", "clawed-abode"]:
    pcm = np.fromfile(f"results/{name}.pcm", dtype="<i2")
    print(f"{name}: {len(pcm) / 24000:.2f} s of audio, peak {abs(pcm).max()}, rms {np.sqrt((pcm.astype(float) ** 2).mean()):.0f}")
try:
    from transformers import pipeline
    asr = pipeline("automatic-speech-recognition", model="openai/whisper-base.en", device="cpu")
    for name in ["lion-reader", "clawed-abode"]:
        pcm = np.fromfile(f"results/{name}.pcm", dtype="<i2").astype(np.float32) / 32768
        at_16k = np.interp(np.arange(0, len(pcm), 1.5), np.arange(len(pcm)), pcm).astype(np.float32)
        print(f"{name} transcript: {asr({'raw': at_16k, 'sampling_rate': 16000})['text']!r}")
except Exception as e:
    print("transcription skipped:", e)
EOF
kill $server
wait $server 2> /dev/null

echo "== hostile front against the same worker"
pid=$(podman inspect -f '{{.State.Pid}}' kw-e2e)
timeout 1500 python3 tools/hostile.py "$sock/worker.sock" "$pid" > results/hostile.txt 2>&1
echo "hostile exit $?"
tail -60 results/hostile.txt
podman stop -t 5 kw-e2e > /dev/null
wait $worker
grep -cE "protocol violation|stalled|connection error|connection failed" results/e2e.worker.log
grep -E "Traceback|CRITICAL" results/e2e.worker.log | head -5
echo done
