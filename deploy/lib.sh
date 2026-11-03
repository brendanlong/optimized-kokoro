# shellcheck shell=bash
# Shared by install.sh and update.sh (sourced, not run). Runs as root.
#
# Layout of $root (/srv/optimized-kokoro, everything root-owned):
#   settings.env            KOKORO_GPU, KOKORO_DEMO_PORT, KOKORO_MAX_BATCH (edit, then run update.sh)
#   models/                 what kokoro_worker.convert wrote (0644)
#   unidic/unidic.dic       UniDic for Japanese, checked against fetch_unidic.sh's checksum, replaced
#                           only by rename (the server maps it)
#   secrets/                0700; server.toml (with the API keys) and demo.key, root:65534 0640
#   releases/REL/           compose.yaml, .env and the compile-cache seed made with REL's worker image
#   current -> releases/REL the release that runs; swapped by rename
# Images are tagged optimized-kokoro-{worker,server,demo}:REL.

root=${DESTDIR:-}/srv/optimized-kokoro
daemon_json=${DESTDIR:-}/etc/docker/daemon.json
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
health_url=http://127.0.0.1:8880/health
nobody=65534
images=(worker server demo)
# The user whose old systemd user units and state install.sh replaces.
old_user=${KOKORO_OLD_USER:-claude}
# Builds the converters (cargo, uv) and runs them; it must be able to write the checkout.
build_user=${KOKORO_BUILD_USER:-${SUDO_USER:-}}

die() {
    echo "$*" >&2
    exit 1
}

require_root() {
    [ "$(id -u)" = 0 ] || [ -n "${DESTDIR:-}" ] || die "run this with sudo"
    command -v docker > /dev/null || die "docker isn't installed"
    docker compose version > /dev/null || die "the docker compose plugin isn't installed"
}

# Runs a command as the build user in a login shell (for cargo and uv on its PATH), in directory $1.
as_build_user() {
    local dir=$1
    shift
    sudo -u "$build_user" -H -- bash -lc 'cd "$1" && shift && exec "$@"' bash "$dir" "$@"
}

check_build_user() {
    [ -n "$build_user" ] || die "run with sudo from your own account, or set KOKORO_BUILD_USER"
    id "$build_user" > /dev/null 2>&1 || die "no user $build_user"
    if [ "$build_user" = root ] && [ "${KOKORO_BUILD_USER:-}" != root ]; then
        die "SUDO_USER is root: set KOKORO_BUILD_USER to the user who runs cargo and uv (or to root)"
    fi
    as_build_user "$repo" test -w . || die "$build_user can't write $repo; run from a checkout it owns"
}

# A directory the build user can write, removed on exit.
build_user_tmp() {
    local tmp
    tmp=$(mktemp -d)
    chown "$build_user" "$tmp"
    tmps+=("$tmp")
    echo "$tmp"
}
tmps=()
trap 'rm -rf "${tmps[@]}"' EXIT

compose() {
    docker compose -f "$root/current/compose.yaml" "$@"
}

new_key() {
    # 32 URL-safe characters (192 bits).
    head -c 24 /dev/urandom | base64 | tr '+/' '-_'
}

# Adds the runsc-kokoro runtime (gVisor with directfs off and host sockets, see compose.yaml) to
# daemon.json and reloads Docker, which picks up runtimes without restarting any container.
ensure_runtime() {
    local want='{"path": "/usr/bin/runsc", "runtimeArgs": ["--directfs=false", "--host-uds=all"]}'
    [ -f "$daemon_json" ] || die "$daemon_json is missing"
    if jq -e --argjson want "$want" '.runtimes["runsc-kokoro"] == $want' "$daemon_json" > /dev/null; then
        return
    fi
    jq -e '.runtimes["runsc-gpuc"]' "$daemon_json" > /dev/null || die "$daemon_json has no runsc-gpuc runtime (the demo uses it)"
    if jq -e '.runtimes["runsc-kokoro"]' "$daemon_json" > /dev/null; then
        die "$daemon_json already has a different runsc-kokoro runtime; make it $want"
    fi
    echo "== adding the runsc-kokoro runtime to $daemon_json (backup: $daemon_json.bak-kokoro)"
    cp -p "$daemon_json" "$daemon_json.bak-kokoro"
    jq --argjson want "$want" '.runtimes["runsc-kokoro"] = $want' "$daemon_json" > "$daemon_json.new"
    chmod --reference="$daemon_json" "$daemon_json.new"
    mv "$daemon_json.new" "$daemon_json"
    systemctl reload docker
    for _ in $(seq 30); do
        docker info --format '{{json .Runtimes}}' 2> /dev/null | jq -e 'has("runsc-kokoro")' > /dev/null && return
        sleep 1
    done
    die "Docker didn't pick up runsc-kokoro after a reload; check journalctl -u docker"
}

ensure_dirs() {
    install -d -m 0755 "$root" "$root/releases" "$root/unidic"
    install -d -m 0700 "$root/secrets"
    if [ ! -f "$root/settings.env" ]; then
        cat > "$root/settings.env" << 'EOF'
# Copied into each release's .env by deploy/install.sh and deploy/update.sh; run update.sh after a change.
# The GPU for the worker: an index or a UUID from nvidia-smi -L.
KOKORO_GPU=0
# The demo page listens on 127.0.0.1:KOKORO_DEMO_PORT.
KOKORO_DEMO_PORT=25436
# The worker's --max-batch (8 for many listeners: worker/README.md). The cache seed depends on it.
KOKORO_MAX_BATCH=1
EOF
        chmod 0644 "$root/settings.env"
    fi
}

# The old state directory, if the old user has one.
old_state() {
    local home
    home=$(getent passwd "$old_user" | cut -d: -f6) || return 0
    [ -n "$home" ] && [ -d "$home/.local/share/optimized-kokoro" ] && echo "$home/.local/share/optimized-kokoro"
    return 0
}

# Reads a file in the old user's directory as that user (it may be a symlink to anything root can read).
old_cat() {
    sudo -u "$old_user" -- cat -- "$1"
}

unidic_ok() {
    local sha
    sha=$(sed -n 's/^OUT_SHA256=//p' "$repo/crates/g2p-ja/tools/fetch_unidic.sh")
    [ -n "$sha" ] || die "no OUT_SHA256 in fetch_unidic.sh"
    [ -f "$1" ] && echo "$sha  $1" | sha256sum -c --status -
}

# UniDic: kept if it matches fetch_unidic.sh's checksum; otherwise copied from the old state (if
# that matches) or fetched and converted by the build user. Checked again once root owns the copy.
ensure_unidic() {
    local dest=$root/unidic/unidic.dic old tmp
    unidic_ok "$dest" && return
    echo "== UniDic"
    old=$(old_state)
    tmp=$(build_user_tmp)
    if [ -n "$old" ] && old_cat "$old/unidic.dic" > "$tmp/unidic.dic" 2> /dev/null && unidic_ok "$tmp/unidic.dic"; then
        old_cat "$old/unidic.dic.LICENSE" > "$tmp/unidic.dic.LICENSE"
        echo "reusing $old/unidic.dic (checksum matches)"
    else
        rm -f "$tmp/unidic.dic"
        check_build_user
        as_build_user "$repo" crates/g2p-ja/tools/fetch_unidic.sh "$tmp/unidic.dic"
    fi
    install -m 0644 "$tmp/unidic.dic" "$dest.new"
    install -m 0644 "$tmp/unidic.dic.LICENSE" "$root/unidic/unidic.dic.LICENSE"
    unidic_ok "$dest.new" || die "UniDic checksum mismatch"
    mv -f "$dest.new" "$dest"
}

# Models: converted once by the build user (kokoro_worker.convert downloads a pinned revision),
# or copied from KOKORO_MODELS_FROM. Only the three files the converter writes are taken.
ensure_models() {
    local files=(config.json model.safetensors voices.safetensors) src tmp f
    [ -f "$root/models/voices.safetensors" ] && return
    echo "== models"
    tmp=$(build_user_tmp)
    if [ -n "${KOKORO_MODELS_FROM:-}" ]; then
        src=$KOKORO_MODELS_FROM
    else
        check_build_user
        src=$tmp/models
        as_build_user "$repo/worker" uv sync --frozen
        as_build_user "$repo/worker" uv run --no-sync python -m kokoro_worker.convert "$src"
    fi
    rm -rf "$root/models.new"
    install -d -m 0755 "$root/models.new"
    for f in "${files[@]}"; do
        [ -f "$src/$f" ] && [ ! -L "$src/$f" ] || die "$src/$f is missing or not a regular file"
        install -m 0644 "$src/$f" "$root/models.new/$f"
    done
    rm -rf "$root/models"
    mv "$root/models.new" "$root/models"
}

# secrets/server.toml (the API keys: the old deploy's keys, so clawed-abode needs no change, or a new
# one, plus the demo's) and secrets/demo.key. Written once; root:65534 0640, so the containers
# (nobody) read them and no other host user does. The demo's key may use 4 streams at most, which
# keeps capacity for clawed-abode.
ensure_secrets() {
    local dir=$root/secrets old line k keys=() demo
    if [ -f "$dir/server.toml" ] && [ -f "$dir/demo.key" ]; then
        return
    fi
    [ ! -e "$dir/server.toml" ] && [ ! -e "$dir/demo.key" ] || die "$dir has only one of server.toml and demo.key; remove it to start over"
    echo "== API keys"
    old=$(old_state)
    if [ -n "$old" ] && line=$(old_cat "$old/server.env" 2> /dev/null); then
        line=$(sed -n 's/^KOKORO_API_KEYS=//p' <<< "$line" | head -n 1)
        IFS=, read -ra keys <<< "$line"
        for k in "${keys[@]}"; do
            [[ $k =~ ^[A-Za-z0-9._~-]+$ ]] || die "a key in $old/server.env has unexpected characters"
        done
        echo "reusing ${#keys[@]} key(s) from $old/server.env"
    fi
    if [ ${#keys[@]} = 0 ]; then
        keys=("$(new_key)")
        echo "new API key for clients (clawed-abode, Lion Reader): sudo grep -A1 api_keys $dir/server.toml"
    fi
    demo=$(new_key)
    (
        umask 077
        {
            echo '# Written by deploy/install.sh. listen and worker_socket match deploy/compose.yaml.'
            echo 'listen = "0.0.0.0:8880"'
            echo 'worker_socket = "/run/kokoro/worker.sock"'
            echo 'api_keys = ['
            for k in "${keys[@]}"; do echo "    \"$k\","; done
            echo "    # The demo page (demo.key)."
            echo "    { key = \"$demo\", max_streams = 4 },"
            echo ']'
        } > "$dir/server.toml.new"
        echo "$demo" > "$dir/demo.key.new"
    )
    chgrp "$nobody" "$dir/server.toml.new" "$dir/demo.key.new"
    chmod 0640 "$dir/server.toml.new" "$dir/demo.key.new"
    mv "$dir/demo.key.new" "$dir/demo.key"
    mv "$dir/server.toml.new" "$dir/server.toml"
}

# Builds the images and the compile cache for a new release, while whatever runs keeps running.
# Sets $rel.
stage_release() {
    local dir
    rel=$(date -u +%Y%m%dT%H%M%SZ)
    dir=$root/releases/$rel
    [ ! -e "$dir" ] || die "$dir exists"
    echo "== build $rel"
    docker build -t "optimized-kokoro-worker:$rel" -f "$repo/worker/Containerfile" "$repo/worker"
    docker build -t "optimized-kokoro-server:$rel" -f "$repo/deploy/server.Containerfile" "$repo"
    docker build -t "optimized-kokoro-demo:$rel" -f "$repo/demo/Containerfile" "$repo/demo"

    echo "== stage $dir"
    install -d -m 0755 "$dir"
    install -m 0644 "$repo/deploy/compose.yaml" "$dir/compose.yaml"
    {
        echo "KOKORO_ROOT=$root"
        echo "KOKORO_RELEASE=$rel"
        grep -E '^KOKORO_(GPU|DEMO_PORT|MAX_BATCH)=[A-Za-z0-9-]+$' "$root/settings.env" || true
    } > "$dir/.env"
    chmod 0644 "$dir/.env"
    docker compose -f "$dir/compose.yaml" config --quiet

    echo "== compile cache (warms up in gVisor next to the running worker, then exits without listening)"
    # Written by nobody into a directory only it can write, then handed to root and mounted
    # read-only from then on. Same image and flags as the worker that will load it.
    install -d -m 0700 "$dir/cache-seed"
    chown "$nobody:$nobody" "$dir/cache-seed"
    (
        set -a
        # shellcheck source=/dev/null
        . "$dir/.env"
        docker compose -f "$dir/compose.yaml" run --rm --no-deps -T -e KOKORO_CACHE_SEED= \
            -v "$dir/cache-seed:/cache-export" worker \
            --max-batch "${KOKORO_MAX_BATCH:-1}" --export-cache /cache-export
    )
    chown -hR root:root "$dir/cache-seed"
    chmod -R u=rwX,go=rX "$dir/cache-seed"
    [ -n "$(ls -A "$dir/cache-seed")" ] || die "the compile cache is empty"
}

# Points current at releases/$1 (by rename).
point_current() {
    ln -sfn "releases/$1" "$root/current.new"
    mv -T "$root/current.new" "$root/current"
}

wait_healthy() {
    # The worker takes about 3.5 minutes to start in gVisor with directfs off, even with a seed.
    echo "== waiting for $health_url"
    for _ in $(seq 600); do
        curl -sf -o /dev/null "$health_url" && return 0
        sleep 1
    done
    echo "not healthy after 10 minutes" >&2
    return 1
}

# Starts the release current points at (recreating containers whose image or config changed).
start_current() {
    compose up -d --remove-orphans && wait_healthy
}

# Removes a release's images and directory.
remove_release() {
    local i
    for i in "${images[@]}"; do docker image rm "optimized-kokoro-$i:$1" > /dev/null 2>&1 || true; done
    rm -rf "${root:?}/releases/$1"
}

# Removes every release except the ones given (no arguments: all).
# shellcheck disable=SC2120
prune_releases() {
    local d r k keep
    for d in "$root"/releases/*/; do
        [ -d "$d" ] || continue
        r=$(basename "$d")
        keep=
        for k in "$@"; do [ "$r" = "$k" ] && keep=1; done
        [ -n "$keep" ] || remove_release "$r"
    done
}

show_logs() {
    echo "--- last log lines of the failed release" >&2
    compose logs --no-color --tail 40 >&2 || true
}
