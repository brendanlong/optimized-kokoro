# shellcheck shell=bash
# Shared by the fakes in deploy/dry-run/bin (sourced).

log() {
    echo "[$1] ${*:2}" >> "$FAKE_LOG"
}

# Ctrl-C: SIGINT to the whole process group (test.sh runs each script in its own), the first time
# a call matches INT_AT.
maybe_interrupt() {
    [ -n "${INT_AT:-}" ] && [ ! -e "$FAKE_STATE/.interrupted" ] || return 0
    # shellcheck disable=SC2053
    [[ $1 == $INT_AT ]] || return 0
    touch "$FAKE_STATE/.interrupted"
    log interrupt "at: $1"
    kill -INT 0
    /usr/bin/sleep 1
}

# The project of the newest release (the one an update is staging).
newest_project() {
    local r
    r=$(find "$FAKE_ROOT/releases" -mindepth 1 -maxdepth 1 -printf '%f\n' 2> /dev/null | sort | tail -1)
    [ -n "$r" ] || return 0
    sed -n 's/^COMPOSE_PROJECT_NAME=//p' "$FAKE_ROOT/releases/$r/.env" 2> /dev/null
}
