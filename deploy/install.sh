#!/bin/bash
# Installs the server, worker and demo page on the host's system Docker (deploy/compose.yaml), in
# place of the old systemd user units. Run once, with sudo, from a checkout only the build user (you)
# and root can change; it refuses one the old user (claude) can write:
#
#   sudo deploy/install.sh               # set up /srv/optimized-kokoro, build, start, wait for /health
#   sudo deploy/install.sh --remove-old  # afterwards: remove the old user units and state
#
# The first form stops the old units (as the old user, KOKORO_OLD_USER, default claude) only to free
# port 8880, once everything is built and staged, and starts them again if the new deployment
# doesn't come up healthy. It never removes them: that's --remove-old, which first checks that the
# new deployment answers /health.
#
# Environment: KOKORO_BUILD_USER (default SUDO_USER) runs cargo and uv for the converters;
# KOKORO_MODELS_FROM=DIR copies converted models instead of converting them. Later updates:
# deploy/update.sh. See deploy/README.md.
set -euo pipefail
# shellcheck source=deploy/lib.sh
. "$(dirname "$0")/lib.sh"

old_units=(kokoro-server.service kokoro-worker.service)

old_systemctl() {
    local uid
    uid=$(id -u "$old_user")
    sudo -u "$old_user" -- env XDG_RUNTIME_DIR="/run/user/$uid" systemctl --user "$@"
}

old_active() {
    old_systemctl is-active --quiet "${old_units[@]}" 2> /dev/null
}

# Stops the old units (if they ran) and starts the new release. Every step returns on failure rather
# than exiting, so the caller can recover.
go_live() {
    if [ -n "$was_active" ]; then
        echo "== stopping the old user units (${old_units[*]}) to free port 8880"
        old_systemctl stop "${old_units[@]}" || return 1
    fi
    echo "== start $rel"
    point_current "$rel" || return 1
    start_current
}

install_new() {
    require_root
    take_lock
    check_checkout
    [ ! -e "$root/current" ] || die "already installed ($root/current); use deploy/update.sh"
    ensure_runtime
    ensure_dirs
    prune_releases # leftovers of a failed install
    ensure_unidic
    ensure_models
    ensure_secrets
    stage_release

    was_active=
    if id "$old_user" > /dev/null 2>&1 && old_active; then
        was_active=1
    fi
    if ! go_live; then
        # Free port 8880 and bring the old units back before anything else.
        if [ -e "$root/current" ]; then
            show_logs
            compose down || true
        fi
        if [ -n "$was_active" ]; then
            echo "starting the old user units again" >&2
            old_systemctl start "${old_units[@]}" || echo "starting the old units failed too" >&2
        fi
        rm -f "$root/current"
        pending_release=
        prune_releases
        die "install failed; nothing of the old setup was removed"
    fi
    pending_release=

    local old port
    old=$(old_state)
    port=$(sed -n 's/^KOKORO_DEMO_PORT=//p' "$root/settings.env")
    cat << EOF

Healthy on 127.0.0.1:8880, which Tailscale serve's svc:kokoro already proxies to.
  sudo docker compose -f $root/current/compose.yaml ps        # or logs -f
The demo page is on http://127.0.0.1:$port. To give it HTTPS on the tailnet (which also lets
browsers use WebCodecs instead of the WASM encoder), define svc:kokoro-demo in the Tailscale admin
console like svc:kokoro, then:
  sudo tailscale serve --service=svc:kokoro-demo --https=443 http://127.0.0.1:$port
EOF
    if [ -n "$was_active" ] || [ -n "$old" ]; then
        cat << EOF

The old setup is stopped but not removed. Its units are still enabled, so after a reboot they'd
race the containers for port 8880: once you're happy with the new deployment, run
  sudo deploy/install.sh --remove-old
which, as $old_user, runs systemctl --user disable --now ${old_units[*]}, deletes the two unit
files from ~$old_user/.config/systemd/user and daemon-reloads, and deletes ${old:-the old state directory}.
EOF
    fi
}

remove_old() {
    require_root
    take_lock
    [ -e "$root/current" ] || die "the new deployment isn't installed"
    if id "$old_user" > /dev/null 2>&1 && old_active; then
        die "the old units are running, so port 8880 may be theirs; stop them and start the new deployment first"
    fi
    curl -sf -o /dev/null "$health_url" || die "$health_url isn't healthy; not removing the old setup"
    [ -n "$(compose ps -q server)" ] || die "the server container isn't running"

    local home old
    home=$(getent passwd "$old_user" | cut -d: -f6) || die "no user $old_user"
    [ -n "$home" ] || die "$old_user has no home directory"
    old=$(old_state)
    echo "== as $old_user: disable and remove ${old_units[*]}"
    old_systemctl disable --now "${old_units[@]}" || true
    sudo -u "$old_user" -- rm -f -- "${old_units[@]/#/$home/.config/systemd/user/}"
    old_systemctl daemon-reload || echo "daemon-reload failed ($old_user's user manager isn't running?); harmless" >&2
    if [ -n "$old" ]; then
        echo "== as $old_user: delete $old"
        sudo -u "$old_user" -- rm -rf -- "$old"
    fi
    cat << EOF
Done. Left for you (as $old_user): its podman images, e.g.
  podman rmi localhost/kokoro-worker:deploy localhost/kokoro-worker:deploy-new
EOF
}

case ${1:-} in
"") install_new ;;
--remove-old) remove_old ;;
*)
    sed -n '2,16p' "$0"
    exit 2
    ;;
esac
