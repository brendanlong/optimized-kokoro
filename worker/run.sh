#!/bin/bash
# Run the worker in gVisor with one host GPU, no network, a read-only root and bounded RAM/CPU.
#
#   worker/run.sh serve SOCKET_DIR MODELS_DIR [worker flags, e.g. --max-batch 8]
#   worker/run.sh export-cache CACHE_DIR MODELS_DIR [worker flags]
#   worker/run.sh probe SOCKET_DIR MODELS_DIR     # tools/sandbox_probe.py with the serve flags
#
# serve: SOCKET_DIR gets worker.sock (mode 0660, owned by the calling user). MODELS_DIR is what
#   `python -m kokoro_worker.convert` wrote. With KOKORO_CACHE_SEED=DIR, a compile cache made by
#   export-cache is mounted read-only and copied into the tmpfs at startup (compiling in gVisor
#   takes minutes). A seed is only valid for the same image, GPU and driver.
# export-cache: warms up with the same flags, writes the compile caches to CACHE_DIR and exits.
#   It never listens, so a cache is only ever written by a worker that saw no requests, and the
#   writable mount never coexists with serving.
#
# Environment:
#   KOKORO_GPU         CDI device name: an index or a GPU UUID (default 0; `all` is refused)
#   KOKORO_MEMORY_MAX  host RAM for podman, gVisor and the worker (default 12G)
#   KOKORO_CPU_QUOTA   CPU time, 100% = one core (default 400%)
#   KOKORO_TASKS_MAX   host threads (default 1024; gVisor runs guest threads on a few of these)
#   KOKORO_NO_LIMITS=1 skip the systemd scope (no RAM/CPU limits; for hosts without a user systemd)
#
# Needs rootless podman, /usr/bin/runsc, the NVIDIA CDI spec, and a user systemd with the memory,
# cpu and pids controllers delegated (the default on Ubuntu). No root, no podman/docker config.
set -euo pipefail

usage() {
    sed -n '2,26p' "$0"
    exit 2
}
[ $# -ge 3 ] || usage
mode=$1
dir=$(realpath "$2")
models=$(realpath "$3")
shift 3

image=${KOKORO_WORKER_IMAGE:-localhost/kokoro-worker:latest}
name=${KOKORO_WORKER_NAME:-kokoro-worker}
runsc=${RUNSC:-/usr/bin/runsc}
gpu=${KOKORO_GPU:-0}
if [ "$gpu" = all ]; then
    echo "KOKORO_GPU=all: name one GPU (an index or UUID from nvidia-smi -L)" >&2
    exit 2
fi
if [ -n "${KOKORO_CACHE_EXPORT:-}" ]; then
    echo "KOKORO_CACHE_EXPORT is gone: use '$0 export-cache CACHE_DIR MODELS_DIR'" >&2
    exit 2
fi

mounts=(-v "$models:/models:ro")
entry=()
case $mode in
probe)
    mkdir -p "$dir"
    chmod 0700 "$dir"
    mounts+=(-v "$dir:/run/kokoro:rw" -v "$(cd "$(dirname "$0")" && pwd)/tools:/probe:ro")
    entry=(--entrypoint python)
    set -- /probe/sandbox_probe.py
    ;;
serve)
    mkdir -p "$dir"
    chmod 0700 "$dir"
    mounts+=(-v "$dir:/run/kokoro:rw")
    if [ -n "${KOKORO_CACHE_SEED:-}" ]; then
        mounts+=(-v "$(realpath "$KOKORO_CACHE_SEED"):/cache-seed:ro" -e KOKORO_CACHE_SEED=/cache-seed)
    fi
    ;;
export-cache)
    if [ -n "${KOKORO_CACHE_SEED:-}" ]; then
        echo "export-cache starts from an empty cache; unset KOKORO_CACHE_SEED" >&2
        exit 2
    fi
    mkdir -p "$dir"
    mounts+=(-v "$dir:/cache-export:rw")
    set -- "$@" --export-cache /cache-export
    ;;
*)
    usage
    ;;
esac

limits=()
if [ "${KOKORO_NO_LIMITS:-0}" != 1 ]; then
    # runsc's own cgroup support needs root (rootless podman passes ignore-cgroups), and podman
    # won't run runsc without cgroups, so --cgroups=split keeps everything inside this scope.
    limits=(systemd-run --user --scope --quiet --collect --unit "$name-$$"
        -p "MemoryMax=${KOKORO_MEMORY_MAX:-12G}" -p MemorySwapMax=0
        -p "CPUQuota=${KOKORO_CPU_QUOTA:-400%}" -p "TasksMax=${KOKORO_TASKS_MAX:-1024}")
fi

# --runtime-flag:
#   ignore-cgroups   rootless podman can't hand runsc a cgroup
#   host-uds=create  lets the sandbox create a Unix socket the host can connect to
# NVIDIA_VISIBLE_DEVICES=void: the CUDA base image sets it to "all", which makes runsc run the
# legacy nvidia-container-cli hook (and fail rootless); the device comes from CDI instead.
# --userns=keep-id --user UID:GID: the process runs as the calling user's uid, whatever it is (not
# the image's 1001), so the socket it creates belongs to that user on the host.
# --tmpfs /tmp: Triton and Inductor write their caches and compile their launcher there (exec needed).
exec "${limits[@]}" podman run --rm --name "$name" \
    --runtime "$runsc" \
    --runtime-flag ignore-cgroups \
    --runtime-flag host-uds=create \
    --cgroups=split \
    --network=none \
    --read-only \
    --tmpfs /tmp:rw,exec,nosuid,nodev,size=2g \
    --userns=keep-id \
    --user "$(id -u):$(id -g)" \
    --cap-drop=all \
    --security-opt no-new-privileges \
    --device "nvidia.com/gpu=$gpu" \
    -e NVIDIA_VISIBLE_DEVICES=void \
    "${mounts[@]}" \
    "${entry[@]}" \
    "$image" "$@"
