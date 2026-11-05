#!/bin/bash
# Dry runs of deploy/install.sh, update.sh and keys.sh, as any user and without Docker or a GPU:
#
#   deploy/dry-run/test.sh [SCENARIO...]     # all of them by default; -l lists them
#   SHOW=1 deploy/dry-run/test.sh update      # and shows the last script's output
#   LEGACY_DEPLOY=DIR deploy/dry-run/test.sh  # the migration scenarios install with DIR/install.sh,
#                                             # a deploy/ from before per-release projects (83b4b9c)
#
# The scripts run from a copy of deploy/ with DESTDIR set (so /srv/optimized-kokoro and
# /etc/docker/daemon.json are under a temporary directory), and with the fakes in bin/ first on
# PATH: docker (Compose projects kept as files, see bin/docker), curl, nvidia-smi, nvidia-ctk,
# runsc, sudo, systemctl, chown, chgrp, sha256sum, date and sleep (which doesn't wait). Each
# scenario checks what is left: which release current points at, which projects have which
# containers, network and volume, and that the running release wasn't disturbed when it shouldn't
# have been. Needs the real docker compose for `compose config` (no daemon), flock, ss, jq.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd -P)
src=$(cd "$here/../.." && pwd -P)
REAL_DOCKER=$(command -v docker) || {
    echo "needs docker (the compose plugin, for compose config; no daemon)" >&2
    exit 1
}
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
export REAL_DOCKER
export FAKE_STATE=$W/state FAKE_LOG=$W/log FAKE_DATE=$W/date DESTDIR=$W/dest
export FAKE_ROOT=$W/dest/srv/optimized-kokoro TMPDIR=$W/tmp
export KOKORO_BUILD_USER
KOKORO_BUILD_USER=$(id -un)
D=$FAKE_ROOT
S=$FAKE_STATE
co=$W/repo
mkdir -p "$co/deploy" "$co/crates/g2p-ja/tools" "$W/in/models" "$TMPDIR"
cp -a "$src/deploy/." "$co/deploy/"
cp -a "$src/crates/g2p-ja/tools/fetch_unidic.sh" "$co/crates/g2p-ja/tools/"
if [ -n "${LEGACY_DEPLOY:-}" ]; then
    mkdir -p "$W/legacy/deploy" "$W/legacy/crates/g2p-ja/tools"
    cp -a "$LEGACY_DEPLOY/." "$W/legacy/deploy/"
    cp -a "$src/crates/g2p-ja/tools/fetch_unidic.sh" "$W/legacy/crates/g2p-ja/tools/"
    chmod -R go-w "$W/legacy"
fi
chmod -R go-w "$co"
export PATH=$co/deploy/dry-run/bin:$PATH
echo dry-run-client-key-0123456789 > "$W/in/keys"
for f in config.json model.safetensors voices.safetensors; do echo "$f" > "$W/in/models/$f"; done
echo unidic > "$W/in/unidic.dic"
echo license > "$W/in/unidic.dic.LICENSE"
imports=(--keys-from "$W/in/keys" --models-from "$W/in/models" --unidic-from "$W/in/unidic.dic")

# run [VAR=VALUE...] SCRIPT [ARGS...]: runs deploy/SCRIPT in its own session (so a fake's Ctrl-C
# reaches only it), output in $out and its exit status in $rc.
out='' rc=''
run() {
    local envs=()
    while [[ $1 == [A-Z]*=* ]]; do
        envs+=("$1")
        shift
    done
    rm -f "$S/.interrupted"
    : > "$FAKE_LOG"
    local script=$1
    [[ $script == /* ]] || script=$co/deploy/$1
    out=$(env "${envs[@]}" setsid --wait "$script" "${@:2}" 2>&1)
    rc=$?
}

fresh() {
    rm -rf "$W/dest" "$S"
    mkdir -p "$W/dest/etc/docker" "$D" "$S"
    echo '{}' > "$W/dest/etc/docker/daemon.json"
    printf 'KOKORO_LISTEN=127.0.0.1:18880\nKOKORO_DEMO_LISTEN=127.0.0.1:18881\n' > "$D/settings.env"
}

cur() { basename "$(readlink "$D/current")"; }
project() { sed -n 's/^COMPOSE_PROJECT_NAME=//p' "$D/releases/$1/.env" | grep . || echo optimized-kokoro; }
# The projects that have anything left (containers, network or volume), and what.
projects() {
    local d what
    for d in "$S"/*/; do
        [ -d "$d" ] || continue
        what="$(find "$d/svc" -mindepth 1 -printf '%f\n' 2> /dev/null | sort | tr '\n' ' ')$([ -e "$d/network" ] && echo net)$([ -e "$d/volume" ] && echo ' vol')"
        [ -z "$what" ] || echo "$(basename "$d"):$what"
    done
}
# The container ids of project $1.
ids() { cat "$S/$1"/svc/* 2> /dev/null | tr '\n' ' '; }
releases() { find "$D/releases" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort | tr '\n' ' '; }

fails=0
failed() {
    echo "FAIL $name: $*"
    echo "--- output"
    echo "$out" | tail -40
    echo "--- projects"
    projects
    fails=$((fails + 1))
}

# check_state [cloudflared]: only the current release's project is left, running all its services,
# with its network and volume; no temporary directory, GPU sampler or lock is left.
check_state() {
    local want="demo server worker net vol"
    [ "${1:-}" != cloudflared ] || want="cloudflared demo server worker net vol"
    [ -L "$D/current" ] || {
        failed "no current"
        return 1
    }
    [ "$(projects)" = "$(project "$(cur)"):$want" ] || {
        failed "expected only $(project "$(cur)"):$want"
        return 1
    }
    [ -z "$(ls -A "$TMPDIR")" ] || {
        failed "temporary directories left: $(ls "$TMPDIR")"
        return 1
    }
    ! pgrep -f "$co/deploy/dry-run/bin/nvidia-smi" > /dev/null || {
        failed "the GPU sampler is still running"
        return 1
    }
    flock -n "$D/.lock" true || {
        failed "the lock is held"
        return 1
    }
}

ok() {
    echo "ok   $name"
    # shellcheck disable=SC2001
    [ -z "${SHOW:-}" ] || sed 's/^/     | /' <<< "$out"
}

# expect MESSAGE COMMAND...: fails the scenario with MESSAGE unless COMMAND succeeds.
expect() {
    "${@:2}" || {
        failed "$1"
        return 1
    }
}

expect_rc() {
    [ "$rc" = "$1" ] || {
        failed "exit status $rc, expected $1"
        return 1
    }
}

expect_out() {
    grep -qF -- "$1" <<< "$out" || {
        failed "no '$1' in the output"
        return 1
    }
}

# The installed state for each update scenario: a fresh install, or with $1 = legacy, an install
# made to look like one from before per-release projects (project optimized-kokoro, no
# COMPOSE_PROJECT_NAME, and compose.yaml's name: optimized-kokoro).
installed() {
    local r p
    fresh
    if [ "${1:-}" = cloudflared ]; then
        echo token > "$D/secrets.token.tmp"
        mkdir -p "$D/secrets"
        mv "$D/secrets.token.tmp" "$D/secrets/cloudflared.token"
        echo KOKORO_CLOUDFLARED=1 >> "$D/settings.env"
    fi
    if [ "${1:-}" = legacy ] && [ -n "${LEGACY_DEPLOY:-}" ]; then
        run "$W/legacy/deploy/install.sh" "${imports[@]}"
    else
        run install.sh "${imports[@]}"
    fi
    [ "$rc" = 0 ] || {
        failed "install failed"
        return 1
    }
    if [ "${1:-}" = legacy ] && [ -z "${LEGACY_DEPLOY:-}" ]; then
        r=$(cur)
        p=$(project "$r")
        sed -i '/^COMPOSE_PROJECT_NAME=/d' "$D/releases/$r/.env"
        sed -i '0,/^x-hardening/s//name: optimized-kokoro\n\n&/' "$D/releases/$r/compose.yaml"
        mv "$S/$p" "$S/optimized-kokoro"
        sed -i "s/^$p-/optimized-kokoro-/" "$S"/optimized-kokoro/svc/*
    fi
    before_cur=$(cur)
    before_project=$(project "$before_cur")
    before_ids=$(ids "$before_project")
    before_worker=$(cat "$S/$before_project/svc/worker")
    before_releases=$(releases)
}

# After a failed or interrupted update: the previous release is current, untouched (its containers
# are the same ones), and the new release is gone.
check_untouched() {
    check_state "$@" || return 1
    [ "$(cur)" = "$before_cur" ] || {
        failed "current is $(cur), expected $before_cur"
        return 1
    }
    [ "$(ids "$before_project")" = "$before_ids" ] || {
        failed "the running release's containers changed: $(ids "$before_project"), were $before_ids"
        return 1
    }
    [ "$(releases)" = "$before_releases" ] || {
        failed "releases are $(releases), expected $before_releases"
        return 1
    }
}

# After a swap that was undone: the previous release is current, its worker the same container
# (so it came back without a worker startup), and the new release is gone.
check_back() {
    check_state "$@" || return 1
    [ "$(cur)" = "$before_cur" ] || {
        failed "current is $(cur), expected $before_cur"
        return 1
    }
    [ "$(cat "$S/$before_project/svc/worker")" = "$before_worker" ] || {
        failed "the previous release's worker was restarted"
        return 1
    }
    [ "$(releases)" = "$before_releases" ] || {
        failed "releases are $(releases), expected $before_releases"
        return 1
    }
    expect_out "going back to $before_cur"
}

# After a successful update: the new release is current, the previous one kept (directory only).
check_updated() {
    check_state "$@" || return 1
    [ "$(cur)" != "$before_cur" ] || {
        failed "current is still $before_cur"
        return 1
    }
    [ "$(releases)" = "$before_cur $(cur) " ] || {
        failed "releases are $(releases), expected $before_cur $(cur)"
        return 1
    }
    expect_out "downtime: " && expect_out "GPU memory while both workers ran: 2600 MiB"
}

# The order of the update's Docker calls: the new worker, then the old front down, then the new
# front, then the old worker down.
check_order() {
    local new old
    new=$(project "$(cur)")
    old=$before_project
    awk -v new="$new" -v old="$old" '
        index($0, "-p " new " ") && / up -d --no-deps worker/ { if (!a) a = NR }
        index($0, "-p " old " ") && / rm -sf server demo cloudflared/ { if (!b) b = NR }
        index($0, "network rm " old "_kokoro") { if (!c) c = NR }
        index($0, "-p " new " ") && / up -d --remove-orphans/ { if (!d) d = NR }
        index($0, "-p " old " ") && / down --volumes --remove-orphans/ { if (!e) e = NR }
        END { exit !(a && a < b && b < c && c < d && d < e) }' "$FAKE_LOG" || {
        failed "unexpected order of docker calls:
$(grep -E 'up -d|rm -sf|network rm|down' "$FAKE_LOG")"
        return 1
    }
}

t_install() {
    fresh
    run install.sh "${imports[@]}"
    expect_rc 0 && check_state && expect_out "Up: the server on 127.0.0.1:18880" &&
        expect "project $(project "$(cur)")" [ "$(project "$(cur)")" = "optimized-kokoro-$(cur | tr '[:upper:]' '[:lower:]')" ]
}
# The worker's settings reach both its export run and the release's .env (which compose.yaml
# reads); a value outside the range is refused before anything is built.
t_install_worker_settings() {
    fresh
    printf 'KOKORO_MAX_BATCH=8\nKOKORO_MAX_CHUNK_SECONDS=30\n' >> "$D/settings.env"
    run install.sh "${imports[@]}"
    expect_rc 0 && check_state || return 1
    expect "no --max-chunk-seconds 30 in the export" grep -q -- "--max-batch 8 --max-chunk-seconds 30 --export-cache" "$FAKE_LOG" &&
        expect "not in .env" grep -qx KOKORO_MAX_CHUNK_SECONDS=30 "$D/releases/$(cur)/.env" || return 1
    for bad in 24 61; do
        fresh
        echo KOKORO_MAX_CHUNK_SECONDS=$bad >> "$D/settings.env"
        run install.sh "${imports[@]}"
        expect_rc 1 && expect_out "KOKORO_MAX_CHUNK_SECONDS (25-60)" && expect "built anyway" [ -z "$(releases)" ] || return 1
    done
}
t_install_fails() {
    fresh
    run FAIL_HEALTH=new install.sh "${imports[@]}"
    expect_rc 1 || return 1
    expect "left: current, projects $(projects), releases $(releases)" \
        [ ! -e "$D/current" -a -z "$(projects)" -a -z "$(releases)" -a -z "$(ls -A "$TMPDIR")" ]
}
t_update() {
    installed || return 1
    run update.sh
    expect_rc 0 && check_updated && check_order
}
t_update_twice() {
    installed || return 1
    run update.sh
    expect_rc 0 || return 1
    before_cur=$(cur)
    before_project=$(project "$before_cur")
    run update.sh
    expect_rc 0 && check_state && expect "still $before_cur" [ "$(cur)" != "$before_cur" ] && check_order &&
        expect "releases $(releases)" [ "$(wc -w <<< "$(releases)")" = 2 ]
}
t_update_tunnel() {
    installed cloudflared || return 1
    run update.sh
    expect_rc 0 && check_updated cloudflared && expect_out "the tunnel was down for"
}
t_worker_never_ready() {
    installed || return 1
    run FAKE_WORKER=never update.sh
    expect_rc 1 && check_untouched && expect_out "wasn't ready after 10 minutes"
}
t_worker_crashes() {
    installed || return 1
    run FAKE_WORKER=crash update.sh
    expect_rc 1 && check_untouched && expect_out "stopped before it was ready (restarting, restarted 3 times)"
}
t_gpu_full() {
    installed || return 1
    run FAKE_GPU_TOTAL=3000 update.sh
    expect_rc 1 && check_untouched && expect_out "not enough free GPU memory for a second worker: 1700 MiB, against the 1600 MiB"
}
t_gpu_unknown() {
    installed || return 1
    run FAKE_NO_GPU_MEMORY=1 update.sh
    expect_rc 0 && check_state && expect_out "doesn't show the GPU's memory" && expect_out "downtime: "
}
t_build_fails() {
    installed || return 1
    run FAIL_BUILD=1 update.sh
    expect_rc 1 && check_untouched
}
t_health_fails() {
    installed || return 1
    run FAIL_HEALTH=new update.sh
    expect_rc 1 && check_back && expect_out "update failed"
}
t_tunnel_fails() {
    installed cloudflared || return 1
    run FAIL_TUNNEL=1 update.sh
    expect_rc 1 && check_back cloudflared && expect_out "cloudflared didn't connect"
}
# Ctrl-C before the swap leaves the running release untouched; during it, the swap is undone.
interrupt_before() {
    installed "${2:-}" || return 1
    run "INT_AT=$1" update.sh
    expect_rc 130 && check_untouched "${2:-}" && expect "no interrupt" grep -q '^\[interrupt\]' "$FAKE_LOG"
}
interrupt_during() {
    installed "${2:-}" || return 1
    run "INT_AT=$1" update.sh
    expect_rc 130 && check_back "${2:-}" && expect "no interrupt" grep -q '^\[interrupt\]' "$FAKE_LOG"
}
t_int_build() { interrupt_before 'docker build *'; }
t_int_export() { interrupt_before 'docker compose * run *'; }
t_int_worker_start() { interrupt_before 'docker compose * up -d --no-deps worker'; }
t_int_worker_wait() { interrupt_before 'docker compose * logs --no-log-prefix worker'; }
t_int_stop_front() { interrupt_during 'docker compose * rm -sf server demo cloudflared'; }
t_int_network_rm() { interrupt_during 'docker network rm *'; }
t_int_front_start() { interrupt_during 'docker compose * up -d --remove-orphans'; }
t_int_health() { interrupt_during 'curl *'; }
t_int_tunnel() { interrupt_during 'docker compose * logs --no-log-prefix cloudflared' cloudflared; }
# Ctrl-C once the new release is healthy: it stays; what's left of the old one goes next time.
t_int_after_swap() {
    installed || return 1
    run 'INT_AT=docker compose * down --volumes --remove-orphans' update.sh
    expect_rc 130 || return 1
    [ "$(cur)" != "$before_cur" ] || {
        failed "went back to $before_cur"
        return 1
    }
    [ -e "$S/$before_project/svc/worker" ] || {
        failed "the old worker isn't left over"
        return 1
    }
    run update.sh
    expect_rc 0 && check_state && expect_out "removing the Compose project $before_project"
}
# A second Ctrl-C while the first is being undone doesn't cut the undoing short.
t_int_twice() {
    installed || return 1
    run 'INT_AT=curl *' INT_COUNT=2 update.sh
    expect_rc 130 && check_back && expect "not interrupted twice" [ "$(grep -c '^\[interrupt\]' "$FAKE_LOG")" = 2 ]
}
# Killed (kill -9, a power cut) between taking the old server down and starting the new one: the
# next update puts the old server back next to its worker first, then updates.
t_killed_mid_swap() {
    installed || return 1
    run 'KILL_AT=docker network rm *' update.sh
    expect "the old server is still up" [ ! -e "$S/$before_project/svc/server" ] || return 1
    # What the killed run couldn't clean up: its temporary directories.
    rm -rf "${TMPDIR:?}"/*
    run update.sh
    expect_rc 0 && check_updated && expect_out "starting the rest of $before_cur" &&
        expect "the old release wasn't started again" grep -q "^\[docker\] compose -p $before_project .* up -d --remove-orphans" "$FAKE_LOG"
}
t_leftover_project() {
    installed || return 1
    mkdir -p "$S/optimized-kokoro-20290101t000000z/svc"
    echo optimized-kokoro-20290101t000000z-worker-99 > "$S/optimized-kokoro-20290101t000000z/svc/worker"
    touch "$S/optimized-kokoro-20290101t000000z/network" "$S/optimized-kokoro-20290101t000000z/volume"
    run update.sh
    expect_rc 0 && check_updated && expect_out "removing the Compose project optimized-kokoro-20290101t000000z"
}
t_migrate() {
    installed legacy || return 1
    run update.sh
    expect_rc 0 && check_updated && check_order && expect "optimized-kokoro is left" [ ! -e "$S/optimized-kokoro" ]
}
t_migrate_tunnel() {
    installed cloudflared || return 1
    # As legacy, with the tunnel.
    local r=$before_cur
    sed -i '/^COMPOSE_PROJECT_NAME=/d' "$D/releases/$r/.env"
    sed -i '0,/^x-hardening/s//name: optimized-kokoro\n\n&/' "$D/releases/$r/compose.yaml"
    mv "$S/$before_project" "$S/optimized-kokoro"
    before_project=optimized-kokoro
    run update.sh
    expect_rc 0 && check_updated cloudflared && check_order && expect "optimized-kokoro is left" [ ! -e "$S/optimized-kokoro" ]
}
t_migrate_health_fails() {
    installed legacy || return 1
    run FAIL_HEALTH=new update.sh
    expect_rc 1 && check_back && expect "not back on optimized-kokoro" [ "$(project "$(cur)")" = optimized-kokoro ]
}
t_migrate_int_worker_wait() {
    installed legacy || return 1
    run 'INT_AT=docker compose * logs --no-log-prefix worker' update.sh
    expect_rc 130 && check_untouched
}
t_migrate_int_health() {
    installed legacy || return 1
    run 'INT_AT=curl *' update.sh
    expect_rc 130 && check_back
}
t_migrate_worker_never_ready() {
    installed legacy || return 1
    run FAKE_WORKER=never update.sh
    expect_rc 1 && check_untouched
}
# keys.sh recreates only the current release's server, in its project.
keys_add() {
    local p w s
    p=$(project "$(cur)")
    w=$(cat "$S/$p/svc/worker") s=$(cat "$S/$p/svc/server")
    run keys.sh add dry-run
    expect_rc 0 && check_state || return 1
    expect "worker or server not as expected in $p" [ "$(cat "$S/$p/svc/worker")" = "$w" -a "$(cat "$S/$p/svc/server")" != "$s" ]
}
t_keys_after_update() {
    installed || return 1
    run update.sh
    expect_rc 0 && keys_add
}
t_keys_legacy() {
    installed legacy || return 1
    keys_add && expect "not optimized-kokoro" [ "$(project "$(cur)")" = optimized-kokoro ]
}

all=$(declare -F | sed -n 's/^declare -f t_//p')
if [ "${1:-}" = -l ]; then
    echo "$all"
    exit 0
fi
names=("$@")
[ $# -gt 0 ] || mapfile -t names <<< "$all"
for name in "${names[@]}"; do
    before_fails=$fails
    "t_$name"
    [ "$fails" != "$before_fails" ] || ok
done
echo "$fails failed"
[ "$fails" = 0 ]
