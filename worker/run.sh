#!/bin/bash
# Run the worker in gVisor with the host GPU, no network and a read-only root.
#
#   worker/run.sh SOCKET_DIR MODELS_DIR [worker flags, e.g. --max-batch 8]
#
# SOCKET_DIR gets worker.sock (mode 0660, owned by the calling user). MODELS_DIR is what
# `python -m kokoro_worker.convert` wrote. Needs rootless podman, /usr/bin/runsc, and the NVIDIA
# CDI spec (nvidia.com/gpu=all); no root and no changes to podman's or docker's config.
set -euo pipefail

if [ $# -lt 2 ]; then
    sed -n '2,8p' "$0"
    exit 2
fi
socket_dir=$(realpath "$1")
models=$(realpath "$2")
shift 2

image=${KOKORO_WORKER_IMAGE:-localhost/kokoro-worker:latest}
name=${KOKORO_WORKER_NAME:-kokoro-worker}
runsc=${RUNSC:-/usr/bin/runsc}

mkdir -p "$socket_dir"
chmod 0700 "$socket_dir"

# --runtime-flag:
#   ignore-cgroups   rootless podman can't hand runsc a cgroup
#   host-uds=create  lets the sandbox create a Unix socket the host can connect to
# NVIDIA_VISIBLE_DEVICES=void: the CUDA base image sets it to "all", which makes runsc run the
# legacy nvidia-container-cli hook (and fail rootless); the device comes from CDI instead.
# --userns=keep-id: the socket is owned by the calling user, so only they (and their group) connect.
# --tmpfs /tmp: Triton and Inductor write their caches and compile their launcher there (exec needed).
exec podman run --rm --name "$name" \
    --runtime "$runsc" \
    --runtime-flag ignore-cgroups \
    --runtime-flag host-uds=create \
    --network=none \
    --read-only \
    --tmpfs /tmp:rw,exec,nosuid,nodev,size=2g \
    --userns=keep-id \
    --cap-drop=all \
    --security-opt no-new-privileges \
    --device nvidia.com/gpu=all \
    -e NVIDIA_VISIBLE_DEVICES=void \
    -v "$socket_dir:/run/kokoro:rw" \
    -v "$models:/models:ro" \
    "$image" "$@"
