#!/bin/bash
# gpuc job (PR #11 review): everything through run.sh with its default limits. Export a compile cache
# in its own run, serve seeded from it with load, then a batch-1 worker against the hostile front
# and clients that hang up early. Reports the scopes' memory peaks.
set -uo pipefail
cd "$(dirname "$0")/.."
mkdir -p results
MODELS=${GPUC_DATA_DIR:-$HOME/.cache}/kokoro-worker-models
CG=/sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/app.slice

peak() {  # container name -> memory.peak of its run.sh scope, while it runs
    local f
    f=$(ls -d "$CG/$1"-*.scope 2> /dev/null | head -1)
    [ -n "$f" ] && echo "$1 memory.peak $(($(cat "$f/memory.peak") >> 20)) MiB, max $(cat "$f/memory.max")"
}

FLAGS="--max-batch 8" bash bench/job_cache.sh

echo "== hostile front against a batch-1 worker"
sock=results/sock-hostile
rm -rf "$sock"
mkdir -p "$sock"
KOKORO_WORKER_NAME=kw-hostile ./run.sh serve "$sock" "$MODELS" > results/hostile.worker.log 2>&1 &
until grep -q "listening on" results/hostile.worker.log; do
    sleep 2
    kill -0 $! 2> /dev/null || { echo "worker died"; tail results/hostile.worker.log; exit 1; }
done
pid=$(podman inspect -f '{{.State.Pid}}' kw-hostile)
python3 -c "
import socket
for _ in range(50):
    s = socket.socket(socket.AF_UNIX); s.connect('$sock/worker.sock'); s.close()
print('50 early hang-ups sent')"
timeout 1500 python3 tools/hostile.py "$sock/worker.sock" "$pid" 2>&1 | tee results/hostile.txt | tail -40
peak kw-hostile | tee -a results/hostile.txt
python3 -c "
import sys; sys.path.insert(0, '.')
from kokoro_worker.client import Client
from kokoro_worker import protocol as P
c = Client('$sock/worker.sock'); c.sock.settimeout(60)
c.send(P.synthesize(1, [50, 83, 156, 43, 102] * 10, [(0, 1.0)]))
print('after all that:', c.receive()[0], c.receive()[0])"
podman stop -t 5 kw-hostile > /dev/null
wait
grep -cE "protocol violation|stalled|connection error|connection failed" results/hostile.worker.log
grep -E "Traceback|CRITICAL" results/hostile.worker.log | head -5
echo done
