# shellcheck shell=bash
# Shared by install.sh and update.sh (sourced, not run). Runs as root.
#
# Layout of $root (/srv/optimized-kokoro, everything root-owned):
#   settings.env            KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN, KOKORO_MAX_BATCH
#   models/                 what kokoro_worker.convert wrote (0644)
#   unidic/SHA256.dic       UniDic for Japanese, named by its checksum (fetch_unidic.sh's OUT_SHA256);
#                           each release's .env names the one it uses, so a new dictionary never
#                           replaces the one a running server maps
#   secrets/                0700; server.toml (with the API keys) and demo.key, root:65534 0640
#   releases/REL/           compose.yaml, .env and the compile-cache seed made with REL's worker image
#   current -> releases/REL the release that runs; swapped by rename
#   .lock                   held by install.sh and update.sh while they run
# Images are tagged optimized-kokoro-{worker,server,demo}:REL.

root=${DESTDIR:-}/srv/optimized-kokoro
daemon_json=${DESTDIR:-}/etc/docker/daemon.json
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
nobody=65534
images=(worker server demo)
# Owns the checkout, and builds and runs the converters (cargo, uv).
build_user=${KOKORO_BUILD_USER:-${SUDO_USER:-}}

die() {
    echo "$*" >&2
    exit 1
}

require_root() {
    local c
    [ "$(id -u)" = 0 ] || [ -n "${DESTDIR:-}" ] || die "run this with sudo"
    for c in docker jq curl flock sha256sum ss runsc; do
        command -v "$c" > /dev/null || die "$c isn't installed"
    done
    docker compose version > /dev/null || die "the docker compose plugin isn't installed"
}

# One install or update at a time.
take_lock() {
    install -d -m 0755 "$root"
    exec 9> "$root/.lock"
    flock -n 9 || die "another install.sh or update.sh is running"
}

# nvproxy supports exact driver versions only.
check_driver() {
    local driver
    command -v nvidia-smi > /dev/null || die "nvidia-smi isn't installed (the NVIDIA driver is required)"
    driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -n 1)
    runsc nvproxy list-supported-drivers | grep -qxF "$driver" ||
        die "this runsc ($(runsc --version | head -n 1)) doesn't support NVIDIA driver $driver; see runsc nvproxy list-supported-drivers"
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
        die "SUDO_USER is root: set KOKORO_BUILD_USER to the user who owns the checkout (or to root)"
    fi
}

# The group whose write bit is harmless: the build user's private group (its own name, no members).
private_group() {
    local g members
    g=$(id -gn "$build_user")
    members=$(getent group "$g" | cut -d: -f4)
    if [ "$g" = "$build_user" ] && [ -z "$members" ]; then echo "$g"; fi
}

# Whether $1 is a sticky directory (others may write it, but not rename or delete what isn't theirs).
sticky_dir() {
    [ -d "$1" ] && [ -k "$1" ]
}

# Whether directory or file $1 can be changed only by the build user and root: owned by one of
# them, and not writable by its group (unless that's the build user's private group) or by others,
# unless it's a sticky directory.
only_ours() {
    local owner group mode m
    read -r owner group mode < <(stat -c '%U %G %a' -- "$1")
    m=$((8#$mode))
    [ "$owner" = root ] || [ "$owner" = "$build_user" ] || return 1
    sticky_dir "$1" && return 0
    if ((m & 8#20)) && [ "$group" != "$(private_group)" ]; then return 1; fi
    if ((m & 8#2)); then return 1; fi
    return 0
}

# Root runs these scripts from the checkout and builds images from it, so no one but the build user
# and root may be able to change any of it, or any directory above it. (ACLs aren't checked.)
# worker/.venv/.lock is the exception: uv makes it 0666, and nothing reads it.
check_checkout() {
    local d found pg gw
    check_build_user
    d=$repo
    while :; do
        only_ours "$d" || die "$d can be changed by someone other than $build_user and root; run from a checkout only they can change"
        [ "$d" = / ] && break
        d=$(dirname "$d")
    done
    pg=$(private_group)
    gw=(-perm -g+w)
    [ -z "$pg" ] || gw=(-perm -g+w ! -group "$pg")
    found=$(find "$repo" ! -type l ! -path '*/.venv/.lock' \
        \( \( ! -user "$build_user" ! -user root \) -o -perm -o+w -o \( "${gw[@]}" \) \) -print -quit)
    [ -z "$found" ] || die "$found can be changed by someone other than $build_user and root"
}

# Temporary directories, removed on exit. make_tmp VAR [OWNER] makes one (root's, mode 0700, unless
# OWNER is given) and sets VAR to its path; it runs in the caller's shell, so the trap sees it.
tmps=()
# A release being staged; removed on exit unless the caller cleared this or current points at it.
pending_release=
cleanup() {
    rm -rf "${tmps[@]}"
    if [ -n "$pending_release" ] && [ "$(readlink "$root/current" 2> /dev/null)" != "releases/$pending_release" ]; then
        echo "removing the unfinished release $pending_release" >&2
        remove_release "$pending_release"
    fi
}
trap cleanup EXIT
make_tmp() {
    local t
    t=$(mktemp -d)
    tmps+=("$t")
    if [ -n "${2:-}" ]; then chown "$2" "$t"; fi
    printf -v "$1" %s "$t"
}

# Whether $1 is reached without symlinks, through directories (and to a file) owned by root and
# writable by no one else (sticky directories allowed).
root_path() {
    local p d owner mode m
    p=$(realpath -e -- "$1") && [ "$p" = "$(realpath -es -- "$1")" ] || return 1
    d=$p
    while :; do
        read -r owner mode < <(stat -c '%U %a' -- "$d")
        m=$((8#$mode))
        [ "$owner" = root ] || return 1
        if ! sticky_dir "$d" && ((m & 8#22)); then return 1; fi
        [ "$d" = / ] && return 0
        d=$(dirname "$d")
    done
}

# Copies file $1 to $2 (in root's directory) as $1's owner sees it, so a symlink or a file in someone
# else's directory can't make root copy something that user couldn't read. A root-owned file is
# read directly, but only through root-owned directories (root_path).
# shellcheck disable=SC2024
import_file() {
    local src=$1 owner
    [ -e "$src" ] || die "$src doesn't exist"
    owner=$(stat -c %U -- "$src")
    if [ "$owner" = root ]; then
        root_path "$src" || die "$src is root's, but reached through a symlink or a directory others can change; copy it somewhere root-owned"
        [ -f "$src" ] || die "$src isn't a regular file"
        cat -- "$src" > "$2"
    else
        sudo -u "$owner" -- cat -- "$src" > "$2" || die "can't read $src as $owner"
    fi
}

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
    local runsc want
    runsc=$(command -v runsc)
    want=$(jq -cn --arg p "$runsc" '{path: $p, runtimeArgs: ["--directfs=false", "--host-uds=all"]}')
    [ -f "$daemon_json" ] || echo '{}' > "$daemon_json"
    # Any runsc path will do, as long as the flags match.
    if jq -e --argjson want "$want" '.runtimes["runsc-kokoro"].runtimeArgs == $want.runtimeArgs' "$daemon_json" > /dev/null; then
        return
    fi
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
# Settings for deploy/compose.yaml, copied into each release's .env by install.sh and update.sh.
# Run update.sh after a change. Unset settings take the defaults shown.
# The GPU for the worker: an index, a UUID or a MIG device from nvidia-smi -L.
#KOKORO_GPU=0
# Where the server listens (IPv4:port). 0.0.0.0 would expose it to every network the host is on.
#KOKORO_LISTEN=127.0.0.1:8880
# Where the demo page listens (IPv4:port).
#KOKORO_DEMO_LISTEN=127.0.0.1:25436
# The worker's --max-batch (8 for many listeners: worker/README.md). The cache seed depends on it.
#KOKORO_MAX_BATCH=1
EOF
        chmod 0644 "$root/settings.env"
    fi
    load_settings
}

# Reads settings.env into KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN and KOKORO_MAX_BATCH (with
# defaults); anything else in it is an error, not dropped.
load_settings() {
    local line n=0 key value addr='^([0-9]{1,3}\.){3}[0-9]{1,3}:[0-9]{1,5}$'
    KOKORO_GPU=0 KOKORO_LISTEN=127.0.0.1:8880 KOKORO_DEMO_LISTEN=127.0.0.1:25436 KOKORO_MAX_BATCH=1
    while IFS= read -r line || [ -n "$line" ]; do
        n=$((n + 1))
        [[ $line =~ ^[[:space:]]*(#.*)?$ ]] && continue
        key=${line%%=*}
        value=${line#*=}
        case $key in
        KOKORO_GPU) [[ $value =~ ^[A-Za-z0-9._:/-]+$ ]] ;;
        KOKORO_LISTEN | KOKORO_DEMO_LISTEN) [[ $value =~ $addr ]] ;;
        KOKORO_MAX_BATCH) [[ $value =~ ^[1-9][0-9]{0,2}$ ]] ;;
        *) false ;;
        esac || die "$root/settings.env line $n isn't a valid KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN or KOKORO_MAX_BATCH: $line"
        printf -v "$key" %s "$value"
    done < "$root/settings.env"
    [ "$KOKORO_LISTEN" != "$KOKORO_DEMO_LISTEN" ] || die "KOKORO_LISTEN and KOKORO_DEMO_LISTEN are the same"
}

settings_env() {
    echo "KOKORO_GPU=$KOKORO_GPU"
    echo "KOKORO_LISTEN=$KOKORO_LISTEN"
    echo "KOKORO_DEMO_LISTEN=$KOKORO_DEMO_LISTEN"
    echo "KOKORO_MAX_BATCH=$KOKORO_MAX_BATCH"
}

health_url() {
    local host=${KOKORO_LISTEN%:*}
    [ "$host" != 0.0.0.0 ] || host=127.0.0.1
    echo "http://$host:${KOKORO_LISTEN##*:}/health"
}

# Fails if something already listens on the port of address $1 (any address on that port, which is
# stricter than needed but never wrong), naming it.
check_port_free() {
    local port=${1##*:} lines
    lines=$(ss -Hltnp "sport = :$port" 2> /dev/null || true)
    [ -z "$lines" ] || die "$1: port $port is already in use, by:
$lines
Stop that first, or choose another address in $root/settings.env."
}

# The value of KOKORO_$2 in release $1's .env.
release_setting() {
    sed -n "s/^KOKORO_$2=//p" "$root/releases/$1/.env"
}

# The checksum of the UniDic this checkout wants (fetch_unidic.sh's OUT_SHA256).
unidic_sha() {
    local sha
    sha=$(sed -n 's/^OUT_SHA256=//p' "$repo/crates/g2p-ja/tools/fetch_unidic.sh")
    [[ $sha =~ ^[0-9a-f]{64}$ ]] || die "no OUT_SHA256 in fetch_unidic.sh"
    echo "$sha"
}

unidic_ok() {
    [ -f "$1" ] && echo "$(unidic_sha)  $1" | sha256sum -c --status -
}

# UniDic, as unidic/SHA.dic: kept if it's there; otherwise imported from $1 (with $1.LICENSE next to
# it) if given, or fetched and converted by the build user. Checked once root has the copy.
ensure_unidic() {
    local from=${1:-} sha dest rt bt
    sha=$(unidic_sha)
    dest=$root/unidic/$sha.dic
    unidic_ok "$dest" && return
    echo "== UniDic"
    make_tmp rt
    if [ -n "$from" ]; then
        import_file "$from" "$rt/unidic.dic"
        import_file "$from.LICENSE" "$rt/LICENSE"
        unidic_ok "$rt/unidic.dic" || die "$from isn't the UniDic this checkout wants (sha256 $sha)"
    else
        make_tmp bt "$build_user"
        as_build_user "$repo" crates/g2p-ja/tools/fetch_unidic.sh "$bt/unidic.dic"
        import_file "$bt/unidic.dic" "$rt/unidic.dic"
        import_file "$bt/unidic.dic.LICENSE" "$rt/LICENSE"
        unidic_ok "$rt/unidic.dic" || die "UniDic checksum mismatch"
    fi
    install -m 0644 "$rt/LICENSE" "$dest.LICENSE"
    install -m 0644 "$rt/unidic.dic" "$dest.new"
    mv -f "$dest.new" "$dest"
}

# Removes the dictionaries no release names, except the one this checkout wants.
prune_unidic() {
    local f want
    want=$(unidic_sha).dic
    for f in "$root"/unidic/*.dic; do
        [ -f "$f" ] && [ "$(basename "$f")" != "$want" ] || continue
        if ! grep -qxF "KOKORO_UNIDIC_FILE=$(basename "$f")" "$root"/releases/*/.env 2> /dev/null; then
            rm -f "$f" "$f.LICENSE"
        fi
    done
}

# Models: imported from directory $1 if given, or converted once by the build user
# (kokoro_worker.convert downloads a pinned revision). Only the three files the converter writes.
ensure_models() {
    local from=${1:-} files=(config.json model.safetensors voices.safetensors) src rt bt f
    [ -f "$root/models/voices.safetensors" ] && return
    echo "== models"
    if [ -n "$from" ]; then
        src=$from
    else
        make_tmp bt "$build_user"
        src=$bt/models
        as_build_user "$repo/worker" uv sync --frozen
        as_build_user "$repo/worker" uv run --no-sync python -m kokoro_worker.convert "$src"
    fi
    make_tmp rt
    for f in "${files[@]}"; do import_file "$src/$f" "$rt/$f"; done
    rm -rf "$root/models.new"
    install -d -m 0755 "$root/models.new"
    for f in "${files[@]}"; do install -m 0644 "$rt/$f" "$root/models.new/$f"; done
    rm -rf "$root/models"
    mv "$root/models.new" "$root/models"
}

# secrets/server.toml (the clients' API keys, imported from $1 or one new one, plus the demo's) and
# secrets/demo.key. Written once; root:65534 0640, so the containers (nobody) read them and no
# other host user does. The demo's key may use 4 streams at most, which keeps capacity for clients.
# $1 holds either a KOKORO_API_KEYS=a,b line or one key per line (# comments allowed).
ensure_secrets() {
    local from=${1:-} dir=$root/secrets rt line k keys=() demo
    if [ -f "$dir/server.toml" ] && [ -f "$dir/demo.key" ]; then
        [ -z "$from" ] || echo "keeping the keys already in $dir (not importing $from)"
        return
    fi
    [ ! -e "$dir/server.toml" ] && [ ! -e "$dir/demo.key" ] || die "$dir has only one of server.toml and demo.key; remove it to start over"
    echo "== API keys"
    if [ -n "$from" ]; then
        make_tmp rt
        import_file "$from" "$rt/keys"
        line=$(sed -n 's/^KOKORO_API_KEYS=//p' "$rt/keys" | head -n 1)
        if [ -n "$line" ]; then
            IFS=, read -ra keys <<< "$line"
        else
            while IFS= read -r line || [ -n "$line" ]; do
                [[ $line =~ ^[[:space:]]*(#.*)?$ ]] || keys+=("$line")
            done < "$rt/keys"
        fi
        for k in "${keys[@]}"; do
            # Characters that are safe inside a TOML basic string: base64 keys (+ / =) included.
            [[ $k =~ ^[A-Za-z0-9._~+/=-]+$ ]] || die "a key in $from has characters other than letters, digits and ._~+/=-"
        done
        [ ${#keys[@]} -gt 0 ] || die "no keys in $from"
        echo "imported ${#keys[@]} key(s) from $from"
    else
        keys=("$(new_key)")
        echo "made a new API key for clients: sudo cat $dir/server.toml"
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
# Sets $rel, and $pending_release until the caller has started it.
stage_release() {
    local dir
    rel=$(date -u +%Y%m%dT%H%M%SZ)
    dir=$root/releases/$rel
    [ ! -e "$dir" ] || die "$dir exists"
    pending_release=$rel
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
        echo "KOKORO_UNIDIC_FILE=$(unidic_sha).dic"
        settings_env
    } > "$dir/.env"
    chmod 0644 "$dir/.env"
    docker compose -f "$dir/compose.yaml" config --quiet

    echo "== compile cache (warms up in gVisor next to the running worker, then exits without listening)"
    # Written by nobody into a directory only it can write, then handed to root and mounted
    # read-only from then on. Same image and flags as the worker that will load it.
    install -d -m 0700 "$dir/cache-seed"
    chown "$nobody:$nobody" "$dir/cache-seed"
    docker compose -f "$dir/compose.yaml" run --rm --no-deps -T -e KOKORO_CACHE_SEED= \
        -v "$dir/cache-seed:/cache-export" worker \
        --max-batch "$KOKORO_MAX_BATCH" --export-cache /cache-export
    chown -hR root:root "$dir/cache-seed"
    chmod -R u=rwX,go=rX "$dir/cache-seed"
    [ -n "$(ls -A "$dir/cache-seed")" ] || die "the compile cache is empty"
}

# Points current at releases/$1 (by rename).
point_current() {
    ln -sfn "releases/$1" "$root/current.new" && mv -T "$root/current.new" "$root/current"
}

# Waits for the server of the release current points at.
wait_healthy() {
    local url
    url=$(health_url)
    # The worker takes about 3.5 minutes to start in gVisor with directfs off, even with a seed.
    echo "== waiting for $url"
    for _ in $(seq 600); do
        curl -sf -o /dev/null "$url" && return 0
        sleep 1
    done
    echo "not healthy after 10 minutes" >&2
    return 1
}

# Starts the release current points at (recreating containers whose image or config changed), with
# that release's settings.
start_current() {
    local r
    r=$(basename "$(readlink "$root/current")")
    KOKORO_LISTEN=$(release_setting "$r" LISTEN)
    compose up -d --remove-orphans && wait_healthy
}

# Removes a release's images and directory, and the dictionaries no release names any more.
remove_release() {
    local i
    for i in "${images[@]}"; do docker image rm "optimized-kokoro-$i:$1" > /dev/null 2>&1 || true; done
    rm -rf "${root:?}/releases/$1"
    prune_unidic
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
    prune_unidic
}

show_logs() {
    echo "--- last log lines of the failed release" >&2
    compose logs --no-color --tail 40 >&2 || true
}
