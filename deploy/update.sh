#!/bin/bash
# Updates the Docker deployment from this checkout (after deploy/install.sh):
#
#   sudo deploy/update.sh
#
# Builds new images, exports a compile cache with the new worker and stages a release while the
# running one keeps serving; then points /srv/optimized-kokoro/current at it (by rename) and
# recreates the containers. If it doesn't come up healthy, current goes back to the previous release,
# which is started again, and the failed release is removed. Server and worker are always updated
# together, since the protocol between them changes with the code. Also picks up changes to
# settings.env and a new UniDic (a new file next to the old one, which the previous release keeps).
set -euo pipefail
# shellcheck source=deploy/lib.sh
. "$(dirname "$0")/lib.sh"

[ $# = 0 ] || { sed -n '2,11p' "$0"; exit 2; }
require_root
take_lock
check_checkout
[ -L "$root/current" ] || die "not installed; run deploy/install.sh first"
[ -f "$root/secrets/server.toml" ] && [ -f "$root/secrets/demo.key" ] || die "$root/secrets is incomplete"
prev=$(basename "$(readlink "$root/current")")
check_driver
ensure_dirs
# The ports the running release holds are its own; any other must be free.
used=("$(port_of "$(release_setting "$prev" LISTEN)")" "$(port_of "$(release_setting "$prev" DEMO_LISTEN)")")
check_port_free "$KOKORO_LISTEN" "${used[@]}"
check_port_free "$KOKORO_DEMO_LISTEN" "${used[@]}"
ensure_runtime
ensure_unidic
ensure_models
stage_release

# Goes back to the previous release after a failed (or interrupted) swap; cleanup then removes the
# new one.
go_back() {
    show_logs
    echo "going back to $prev" >&2
    point_current "$prev" && start_current
}

echo "== swap $prev -> $rel"
on_abort=go_back
if ! { point_current "$rel" && start_current; }; then die "update failed"; fi
on_abort=
pending_release=
prune_releases "$rel" "$prev"
echo "up: $rel (previous: $prev, kept for going back)"
