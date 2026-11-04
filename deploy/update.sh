#!/bin/bash
# Updates the Docker deployment from this checkout (after deploy/install.sh), with a second or two
# of downtime:
#
#   sudo deploy/update.sh
#
# Builds new images, exports a compile cache with the new worker and stages a release, then starts
# the new release's worker next to the running release (each release is its own Compose project),
# which keeps serving while it warms up. Then it removes the running release's server, demo and
# tunnel, points /srv/optimized-kokoro/current at the new release (by rename) and starts its server,
# demo and tunnel; the old worker goes once the new server is healthy. If anything fails, or on
# Ctrl-C, the new release is taken down and removed and the previous one put back as it was (its
# worker never stopped). Server and worker are always updated together, since the protocol between
# them changes with the code. Also picks up changes to settings.env and a new UniDic (a new file
# next to the old one, which the previous release keeps).
set -euo pipefail
# shellcheck source=deploy/lib.sh
. "$(dirname "$0")/lib.sh"

[ $# = 0 ] || { sed -n '2,15p' "$0"; exit 2; }
require_root
take_lock
check_checkout
[ -L "$root/current" ] || die "not installed; run deploy/install.sh first"
[ -f "$root/secrets/server.toml" ] && [ -f "$root/secrets/demo.key" ] || die "$root/secrets is incomplete"
prev=$(current_release)
check_driver
ensure_dirs
check_cdi
# The ports the running release holds are its own; any other must be free.
used=("$(port_of "$(release_setting "$prev" LISTEN)")" "$(port_of "$(release_setting "$prev" DEMO_LISTEN)")")
check_port_free "$KOKORO_LISTEN" "${used[@]}"
check_port_free "$KOKORO_DEMO_LISTEN" "${used[@]}"
ensure_runtime
# A worker, network or volume an update killed before it could clean up left (also the previous
# scheme's project, optimized-kokoro, if an update already moved off it).
sweep_projects "$(project_of "$prev")"
ensure_unidic
ensure_models
stage_release

# Until the swap, a failure (or Ctrl-C) only removes the new release, project and all (cleanup in
# lib.sh); the running release isn't touched.
echo "== start $rel's worker next to $prev's, which keeps serving"
check_gpu_room
gpu_used_before=$(gpu_used)
gpu_watch_start overlap
compose_rel "$rel" up -d --no-deps worker || die "$rel's worker didn't start; $prev keeps serving"
wait_worker "$rel" || die "$rel's worker didn't get ready; $prev keeps serving"
worker_id=$(compose_rel "$rel" ps -q worker)

# Goes back to the previous release after a failed (or interrupted) swap; cleanup then removes the
# new one. The previous worker is still running, so this is as quick as the swap.
go_back() {
    show_logs "$rel"
    echo "going back to $prev" >&2
    # Frees the ports, the addresses and the subnet for the previous release's server and demo.
    compose_rel "$rel" down --volumes --remove-orphans || true
    point_current "$prev" && start_current
}

echo "== swap $prev -> $rel"
on_abort=go_back
swap_at=$EPOCHREALTIME
if ! { stop_front "$prev" && point_current "$rel" && start_current; }; then die "update failed"; fi
on_abort=
pending_release=
if [ "$(compose_rel "$rel" ps -q worker)" != "$worker_id" ]; then
    echo "warning: Compose recreated $rel's worker at the swap, so the service was down for its startup" >&2
fi
echo "== remove $prev's worker"
compose_rel "$prev" down --volumes --remove-orphans || echo "couldn't take $prev down; the next update removes what's left" >&2
gpu_watch_stop
prune_releases "$rel" "$prev"
echo "up: $rel (previous: $prev, kept for going back)"
echo "downtime: $(elapsed "$swap_at" "$healthy_at") s from stopping $prev's server to $rel's answering $(health_url)"
[ -z "$tunnel_at" ] || echo "the tunnel was down for $(elapsed "$swap_at" "$tunnel_at") s"
peak=$(gpu_peak overlap)
[ -z "$peak" ] || echo "GPU memory while both workers ran: $peak MiB at the peak, of $(gpu_total) ($gpu_used_before before the new worker)"
