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
# A changed address must be free; the current one is the running release's own.
[ "$KOKORO_LISTEN" = "$(release_setting "$prev" LISTEN)" ] || check_port_free "$KOKORO_LISTEN"
[ "$KOKORO_DEMO_LISTEN" = "$(release_setting "$prev" DEMO_LISTEN)" ] || check_port_free "$KOKORO_DEMO_LISTEN"
ensure_runtime
ensure_unidic
ensure_models
stage_release

echo "== swap $prev -> $rel"
point_current "$rel"
if ! start_current; then
    show_logs
    echo "update failed; going back to $prev" >&2
    point_current "$prev"
    start_current || echo "$prev isn't healthy either; see sudo docker compose -f $root/current/compose.yaml logs" >&2
    remove_release "$rel"
    pending_release=
    exit 1
fi
pending_release=
prune_releases "$rel" "$prev"
echo "up: $rel (previous: $prev, kept for going back)"
