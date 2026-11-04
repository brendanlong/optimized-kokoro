# shellcheck shell=bash
# Shared by the fakes in deploy/dry-run/bin (sourced).

log() {
    echo "[$1] ${*:2}" >> "$FAKE_LOG"
}

# Ctrl-C: SIGINT to the whole process group (test.sh runs each script in its own), at the first
# INT_COUNT (1) calls that match INT_AT. KILL_AT: SIGKILL instead, at the first match (a kill -9 or
# a power cut: nothing gets to clean up).
maybe_interrupt() {
    local n
    # Both patterns are globs.
    # shellcheck disable=SC2053
    if [ -n "${KILL_AT:-}" ] && [[ $1 == $KILL_AT ]]; then
        log kill "at: $1"
        kill -KILL 0
    fi
    [ -n "${INT_AT:-}" ] || return 0
    # shellcheck disable=SC2053
    [[ $1 == $INT_AT ]] || return 0
    n=$(cat "$FAKE_STATE/.interrupted" 2> /dev/null || echo 0)
    [ "$n" -lt "${INT_COUNT:-1}" ] || return 0
    echo $((n + 1)) > "$FAKE_STATE/.interrupted"
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
