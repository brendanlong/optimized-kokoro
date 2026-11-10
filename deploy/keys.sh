#!/bin/bash
# Manages the clients' API keys of the Docker deployment (after deploy/install.sh):
#
#   sudo deploy/keys.sh list                          # names and limits, never the keys
#   sudo deploy/keys.sh add NAME [--max-streams N] [--max-priority P]   # prints the new key, once
#   sudo deploy/keys.sh remove NAME
#   sudo deploy/keys.sh rename OLD NEW
#
# NAME is 1-32 of a-z, 0-9 and -, starting with a letter or digit. --max-streams caps that key's
# speech requests in progress (default: the server's max_streams_per_key, 48). --max-priority, 0-255,
# is the highest request priority the key's requests run at (default: the server's
# max_priority_per_key, 255): a request above it runs at it, and one that doesn't set one runs at
# the lower of 128 and this. Changes go to /srv/optimized-kokoro/secrets/keys (NAME, KEY,
# MAX_STREAMS and MAX_PRIORITY, tab-separated, - for the default; lines from before MAX_PRIORITY
# have three fields, which means its default),
# server.toml is made from it, and only the server container of the current release (in its
# Compose project) is recreated; the worker keeps running. If the server doesn't come back healthy,
# the old keys are put back (also on Ctrl-C).
set -euo pipefail
# shellcheck source=deploy/lib.sh
. "$(dirname "$0")/lib.sh"

usage() {
    sed -n '2,16p' "$0"
    exit 2
}
[ $# -ge 1 ] || usage
require_root
take_lock
check_checkout
[ -L "$root/current" ] || die "not installed; run deploy/install.sh first"
dir=$root/secrets
[ -f "$dir/server.toml" ] && [ -f "$dir/demo.key" ] || die "$dir is incomplete"
check_releases
ensure_keys_file
# The running release's address, not settings.env's, which may hold a change update.sh hasn't applied.
KOKORO_LISTEN=$(release_setting "$(basename "$(readlink "$root/current")")" LISTEN)

valid_name() {
    [[ $1 =~ ^[a-z0-9][a-z0-9-]{0,31}$ ]] || die "$1: a name is 1-32 of a-z, 0-9 and -, starting with a letter or digit"
}

has_name() {
    cut -f1 "$dir/keys" | grep -qxF -- "$1"
}

# Puts the previous keys back (after a failed or interrupted apply) and restarts the server on them.
restore() {
    mv "$dir/keys.old" "$dir/keys"
    mv "$dir/server.toml.old" "$dir/server.toml"
    compose up -d --no-deps --force-recreate server && wait_healthy
}

# Makes server.toml from keys.new, then swaps both in and recreates the server; puts the old ones
# back if it doesn't come up healthy.
apply() {
    chmod 0600 "$dir/keys.new"
    render_server_toml "$dir/keys.new"
    # /health needs the worker too, so a stopped or starting worker would look like bad keys.
    compose ps --status running --services | grep -qx worker || die "the worker isn't running; start the deployment first (update.sh, or compose up -d)"
    curl -sf -o /dev/null "$(health_url)" || die "the server isn't healthy now, so a change couldn't be checked; fix that first"
    cp -p "$dir/keys" "$dir/keys.old"
    cp -p "$dir/server.toml" "$dir/server.toml.old"
    mv "$dir/keys.new" "$dir/keys"
    mv "$dir/server.toml.new" "$dir/server.toml"
    on_abort=restore
    # The secret is a bind mount of the file, which a rename replaces, so the container is recreated.
    if compose up -d --no-deps --force-recreate server && wait_healthy; then
        on_abort=
        rm -f "$dir/keys.old" "$dir/server.toml.old"
        return 0
    fi
    die "the server didn't come back healthy with the new keys"
}

cmd=$1
shift
case $cmd in
list)
    [ $# = 0 ] || usage
    printf '%-32s %-18s %s\n' NAME MAX_STREAMS MAX_PRIORITY
    while IFS=$'\t' read -r name _ streams priority || [ -n "$name" ]; do
        [ "$streams" != - ] || streams="(server default)"
        priority=${priority:--}
        [ "$priority" != - ] || priority="(server default)"
        printf '%-32s %-18s %s\n' "$name" "$streams" "$priority"
    done < "$dir/keys"
    printf '%-32s %-18s %s\n' "demo (demo.key)" 4 64
    ;;
add)
    [ $# -ge 1 ] || usage
    name=$1 streams=- priority=-
    shift
    valid_name "$name"
    while [ $# -gt 0 ]; do
        [ $# -ge 2 ] || usage
        case $1 in
        --max-streams)
            [[ $2 =~ ^[1-9][0-9]{0,3}$ ]] || die "--max-streams needs a number from 1 to 9999"
            streams=$2
            ;;
        --max-priority)
            [[ $2 =~ ^(0|[1-9][0-9]{0,2})$ && $2 -le 255 ]] || die "--max-priority needs a number from 0 to 255"
            priority=$2
            ;;
        *) usage ;;
        esac
        shift 2
    done
    ! has_name "$name" || die "there's already a key named $name"
    key=$(new_key)
    # sed '$a\' ends the last line with a newline if it has none (not a quote escape).
    # shellcheck disable=SC1003
    (umask 077 && { sed '$a\' "$dir/keys"; printf '%s\t%s\t%s\t%s\n' "$name" "$key" "$streams" "$priority"; } > "$dir/keys.new")
    apply
    echo "added $name. Its key (shown only now):"
    echo "$key"
    ;;
remove)
    [ $# = 1 ] || usage
    valid_name "$1"
    has_name "$1" || die "no key named $1"
    [ "$(wc -l < "$dir/keys")" -gt 1 ] || die "$1 is the only client key; add another first"
    (umask 077 && awk -F'\t' -v n="$1" '$1 != n' "$dir/keys" > "$dir/keys.new")
    apply
    echo "removed $1"
    ;;
rename)
    [ $# = 2 ] || usage
    valid_name "$1"
    has_name "$1" || die "no key named $1"
    valid_name "$2"
    ! has_name "$2" || die "there's already a key named $2"
    (umask 077 && awk -F'\t' -v OFS='\t' -v o="$1" -v n="$2" '$1 == o { $1 = n } 1' "$dir/keys" > "$dir/keys.new")
    # Names are only comments in server.toml, so the server needn't restart.
    chmod 0600 "$dir/keys.new"
    render_server_toml "$dir/keys.new"
    mv "$dir/keys.new" "$dir/keys"
    mv "$dir/server.toml.new" "$dir/server.toml"
    echo "renamed $1 to $2"
    ;;
*)
    usage
    ;;
esac
