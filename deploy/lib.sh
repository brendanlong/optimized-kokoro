# shellcheck shell=bash
# Shared by install.sh and update.sh (sourced, not run). Runs as root.
#
# Layout of $root (/srv/optimized-kokoro, everything root-owned):
#   settings.env            KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN, KOKORO_MAX_BATCH, KOKORO_CLOUDFLARED
#   models/                 what kokoro_worker.convert wrote (0644)
#   unidic/SHA256.dic       UniDic for Japanese, named by its checksum (fetch_unidic.sh's OUT_SHA256);
#                           each release's .env names the one it uses, so a new dictionary never
#                           replaces the one a running server maps
#   secrets/                0700; keys (named client keys, root 0600, see keys.sh), demo.key,
#                           server.toml made from them, and, with the tunnel,
#                           cloudflared.token (put there by the admin), root:65534 0640
#   resolv.conf             for cloudflared (Docker's DNS doesn't work under runsc)
#   releases/REL/           compose.yaml, .env and the compile-cache seed made with REL's worker image
#   current -> releases/REL the release that runs; swapped by rename
#   .lock                   held by install.sh and update.sh while they run
# Images are tagged optimized-kokoro-{worker,server,demo}:REL.

# Compose gives the environment priority over a release's .env, so nothing inherited (sudo -E, say)
# may set what the scripts decide. The only inputs are KOKORO_BUILD_USER and SUDO_USER (and DESTDIR,
# for dry runs).
for _v in $(compgen -e); do
    case $_v in
    KOKORO_BUILD_USER) ;;
    KOKORO_* | COMPOSE_* | DOCKER_*) unset "$_v" ;;
    esac
done
unset _v

root=${DESTDIR:-}/srv/optimized-kokoro
daemon_json=${DESTDIR:-}/etc/docker/daemon.json
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
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
    flock -n 9 || die "another install.sh, update.sh or keys.sh is running"
}

# nvproxy supports exact driver versions only.
check_driver() {
    local driver supported
    command -v nvidia-smi > /dev/null || die "nvidia-smi isn't installed (the NVIDIA driver is required)"
    driver=$(nvidia-smi --query-gpu=driver_version --format=csv,noheader) || die "nvidia-smi failed"
    driver=${driver%%$'\n'*}
    supported=$(runsc nvproxy list-supported-drivers) || die "runsc nvproxy list-supported-drivers failed; is runsc too old?"
    grep -qxF "$driver" <<< "$supported" ||
        die "this runsc ($(runsc --version | sed -n 1p)) doesn't support NVIDIA driver $driver; see runsc nvproxy list-supported-drivers"
}

# The worker's GPU comes through Docker's CDI support, not --gpus (compose.yaml says why): the NVIDIA
# Container Toolkit's CDI spec must name KOKORO_GPU, and Docker must have CDI enabled.
check_cdi() {
    local devices dirs
    command -v nvidia-ctk > /dev/null || die "nvidia-ctk isn't installed (the NVIDIA Container Toolkit is required)"
    devices=$(nvidia-ctk cdi list 2> /dev/null) || die "nvidia-ctk cdi list failed"
    grep -qxF "nvidia.com/gpu=$KOKORO_GPU" <<< "$devices" || die "no CDI device nvidia.com/gpu=$KOKORO_GPU; nvidia-ctk cdi list shows:
$devices
Generate the spec (sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml) or set KOKORO_GPU to one of those names in $root/settings.env."
    dirs=$(docker info --format '{{json .CDISpecDirs}}' 2> /dev/null) || die "docker info failed"
    case $dirs in
    "" | null | "[]") die "Docker's CDI support is off: add \"features\": {\"cdi\": true} to $daemon_json and restart Docker (it's on by default from Docker 28)" ;;
    esac
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

# The group whose write bit is harmless: the build user's private group (its own name, no members,
# and no other user's primary group).
private_group() {
    local g gid members users
    g=$(id -gn "$build_user")
    gid=$(id -g "$build_user")
    members=$(getent group "$g" | cut -d: -f4)
    users=$(getent passwd | awk -F: -v gid="$gid" '$4 == gid' | wc -l)
    if [ "$g" = "$build_user" ] && [ -z "$members" ] && [ "$users" = 1 ]; then echo "$g"; fi
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
# A command that undoes a half-done swap (going back to the previous release, or taking a failed
# install down), run on exit until the caller clears it: on failure, but also on Ctrl-C.
on_abort=
cleanup() {
    local abort=$on_abort
    on_abort=
    if [ -n "$abort" ]; then
        echo "== undoing: $abort" >&2
        $abort || echo "undoing failed; see sudo docker compose -f $root/current/compose.yaml ps" >&2
    fi
    rm -rf "${tmps[@]}"
    if [ -n "$pending_release" ] && [ "$(readlink "$root/current" 2> /dev/null)" != "releases/$pending_release" ]; then
        echo "removing the unfinished release $pending_release" >&2
        remove_release "$pending_release"
    fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM
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
# The GPU for the worker: a CDI device from nvidia-ctk cdi list (nvidia.com/gpu=NAME), by index or UUID.
#KOKORO_GPU=0
# Where the server listens (IPv4:port). 0.0.0.0 would expose it to every network the host is on.
#KOKORO_LISTEN=127.0.0.1:8880
# Where the demo page listens (IPv4:port).
#KOKORO_DEMO_LISTEN=127.0.0.1:25436
# The worker's --max-batch (8 for many listeners: worker/README.md). The cache seed depends on it.
#KOKORO_MAX_BATCH=1
# 1 runs a Cloudflare Tunnel to the server, with the tunnel token in secrets/cloudflared.token
# (deploy/README.md, "Cloudflare Tunnel").
#KOKORO_CLOUDFLARED=0
EOF
        chmod 0644 "$root/settings.env"
    fi
    load_settings
}

# Reads settings.env into KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN, KOKORO_MAX_BATCH and
# KOKORO_CLOUDFLARED (with
# defaults); anything else in it is an error, not dropped.
load_settings() {
    local line n=0 key value
    KOKORO_GPU=0 KOKORO_LISTEN=127.0.0.1:8880 KOKORO_DEMO_LISTEN=127.0.0.1:25436 KOKORO_MAX_BATCH=1 KOKORO_CLOUDFLARED=0
    while IFS= read -r line || [ -n "$line" ]; do
        n=$((n + 1))
        [[ $line =~ ^[[:space:]]*(#.*)?$ ]] && continue
        key=${line%%=*}
        value=${line#*=}
        case $key in
        # One GPU, never all of them (as worker/run.sh).
        KOKORO_GPU) [[ $value =~ ^[A-Za-z0-9._:/-]+$ ]] && [ "$value" != all ] ;;
        KOKORO_LISTEN | KOKORO_DEMO_LISTEN) valid_address "$value" ;;
        # The worker's own range.
        KOKORO_MAX_BATCH) [[ $value =~ ^[0-9]{1,2}$ ]] && ((10#$value >= 1 && 10#$value <= 64)) ;;
        KOKORO_CLOUDFLARED) [[ $value =~ ^[01]$ ]] ;;
        *) false ;;
        esac || die "$root/settings.env line $n isn't a valid KOKORO_GPU (a CDI GPU name, not all), KOKORO_LISTEN or KOKORO_DEMO_LISTEN (IPv4:port), KOKORO_MAX_BATCH (1-64) or KOKORO_CLOUDFLARED (0 or 1): $line"
        printf -v "$key" %s "$value"
    done < "$root/settings.env"
    [ "$(port_of "$KOKORO_LISTEN")" != "$(port_of "$KOKORO_DEMO_LISTEN")" ] || die "KOKORO_LISTEN and KOKORO_DEMO_LISTEN use the same port"
}

# Whether $1 is IPv4:port, with octets 0-255 and a port 1-65535.
valid_address() {
    local o
    [[ $1 =~ ^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3}):([0-9]{1,5})$ ]] || return 1
    for o in "${BASH_REMATCH[@]:1:4}"; do ((10#$o <= 255)) || return 1; done
    ((10#${BASH_REMATCH[5]} >= 1 && 10#${BASH_REMATCH[5]} <= 65535))
}

port_of() {
    echo $((10#${1##*:}))
}

settings_env() {
    echo "KOKORO_GPU=$KOKORO_GPU"
    echo "KOKORO_LISTEN=$KOKORO_LISTEN"
    echo "KOKORO_DEMO_LISTEN=$KOKORO_DEMO_LISTEN"
    echo "KOKORO_MAX_BATCH=$KOKORO_MAX_BATCH"
    [ "$KOKORO_CLOUDFLARED" != 1 ] || echo "COMPOSE_PROFILES=cloudflared"
}

# With KOKORO_CLOUDFLARED=1: checks the tunnel token the admin put in secrets/, gives it the same
# owner and mode as the other secrets (root:nobody 0640, bind-mounted as is), and writes the
# resolv.conf cloudflared needs under runsc.
prepare_cloudflared() {
    local token=$root/secrets/cloudflared.token
    [ "$KOKORO_CLOUDFLARED" = 1 ] || return 0
    [ -f "$token" ] && [ ! -L "$token" ] && [ -s "$token" ] ||
        die "KOKORO_CLOUDFLARED=1 needs the tunnel token in $token (a regular, non-empty file): see deploy/README.md"
    chown root:"$nobody" "$token"
    chmod 0640 "$token"
    printf 'nameserver 1.1.1.1\nnameserver 1.0.0.1\n' > "$root/resolv.conf.new"
    chmod 0644 "$root/resolv.conf.new"
    mv "$root/resolv.conf.new" "$root/resolv.conf"
}

health_url() {
    local host=${KOKORO_LISTEN%:*}
    [ "$host" != 0.0.0.0 ] || host=127.0.0.1
    echo "http://$host:${KOKORO_LISTEN##*:}/health"
}

# Fails if something already listens on the port of address $1 (any address on that port, which is
# stricter than needed but never wrong), naming it. Ports given after it are skipped: the running
# release's own, which it gives up when it's replaced.
check_port_free() {
    local port lines p
    port=$(port_of "$1")
    for p in "${@:2}"; do [ "$port" != "$p" ] || return 0; done
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
    if unidic_ok "$dest"; then
        [ -z "$from" ] || echo "keeping the UniDic already in $root/unidic (not importing $from)"
        return
    fi
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
    if [ -f "$root/models/voices.safetensors" ]; then
        [ -z "$from" ] || echo "keeping the models already in $root/models (not importing $from)"
        return
    fi
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

# secrets/keys (the clients' named API keys, imported from $1 as imported-N or one new one named
# default; root 0600, edited with deploy/keys.sh), secrets/demo.key, and secrets/server.toml made
# from them. Written once; demo.key and server.toml are root:65534 0640, so the containers (nobody)
# read them and no other host user does. The demo's key may use 4 streams at most, which keeps capacity for clients.
# $1 holds either a KOKORO_API_KEYS=a,b line or one key per line (# comments allowed; other NAME=...
# lines are refused, since they're config, not keys). Keys have at least 16 characters.
ensure_secrets() {
    local from=${1:-} dir=$root/secrets rt line k keys=() demo
    if [ -f "$dir/server.toml" ] && [ -f "$dir/demo.key" ]; then
        ensure_keys_file
        [ -z "$from" ] || echo "keeping the keys already in $dir (not importing $from)"
        return
    fi
    [ ! -e "$dir/server.toml" ] && [ ! -e "$dir/demo.key" ] || die "$dir has only one of server.toml and demo.key; remove it to start over"
    echo "== API keys"
    if [ -n "$from" ]; then
        make_tmp rt
        import_file "$from" "$rt/keys"
        line=$(sed -n '/^KOKORO_API_KEYS=/{s///p;q}' "$rt/keys")
        if [ -n "$line" ]; then
            IFS=, read -ra keys <<< "$line"
        else
            while IFS= read -r line || [ -n "$line" ]; do
                [[ $line =~ ^[[:space:]]*(#.*)?$ ]] && continue
                # NAME=value, but not a base64 key's trailing = padding.
                [[ ! $line =~ ^[A-Za-z_][A-Za-z0-9_]*=.*[^=] ]] || die "$from has a ${line%%=*}= line but no KOKORO_API_KEYS=; give it a KOKORO_API_KEYS=a,b line or one key per line"
                keys+=("$line")
            done < "$rt/keys"
        fi
        for k in "${keys[@]}"; do
            # Characters that are safe inside a TOML basic string: base64 keys (+ / =) included.
            [[ $k =~ ^[A-Za-z0-9._~+/=-]+$ ]] || die "a key in $from has characters other than letters, digits and ._~+/=-"
            [ ${#k} -ge 16 ] || die "a key in $from is shorter than 16 characters"
        done
        [ ${#keys[@]} -gt 0 ] || die "no keys in $from"
        echo "imported ${#keys[@]} key(s) from $from"
    else
        keys=("$(new_key)")
        echo "made a new API key for clients, named default (sudo deploy/keys.sh list)"
    fi
    demo=$(new_key)
    (
        umask 077
        local i=0 name
        for k in "${keys[@]}"; do
            i=$((i + 1))
            name=default
            [ -z "$from" ] || name=imported-$i
            printf '%s\t%s\t-\n' "$name" "$k"
        done > "$dir/keys.new"
        echo "$demo" > "$dir/demo.key.new"
    )
    chgrp "$nobody" "$dir/demo.key.new"
    chmod 0640 "$dir/demo.key.new"
    mv "$dir/keys.new" "$dir/keys"
    mv "$dir/demo.key.new" "$dir/demo.key"
    render_server_toml
    mv "$dir/server.toml.new" "$dir/server.toml"
}

# Makes secrets/keys for an install from before it existed: each plain key in server.toml becomes
# imported-N (the demo's key is the table entry, and stays in demo.key).
ensure_keys_file() {
    local dir=$root/secrets i=0 k other
    if [ -f "$dir/keys" ]; then
        [ -s "$dir/keys" ] || die "$dir/keys is empty"
        return 0
    fi
    # Only what install.sh ever wrote, which render_server_toml makes again; anything else (a hand
    # edit) would be lost.
    other=$(grep -vE '^(#.*|listen = "0\.0\.0\.0:8880"|worker_socket = "/run/kokoro/worker\.sock"|api_keys = \[|    "[^"]*",|    # .*|    \{ key = "[^"]*", max_streams = 4 \},|\])$' "$dir/server.toml" || true)
    [ -z "$other" ] || die "$dir/server.toml has lines install.sh didn't write, which keys.sh would drop:
$other
Move them out (or into secrets/keys by hand: NAME<tab>KEY<tab>MAX_STREAMS or -) first."
    (
        umask 077
        while IFS= read -r k; do
            i=$((i + 1))
            printf 'imported-%s\t%s\t-\n' "$i" "$k"
        done < <(sed -n 's/^    "\([^"]*\)",$/\1/p' "$dir/server.toml") > "$dir/keys.new"
    )
    [ -s "$dir/keys.new" ] || die "found no client keys in $dir/server.toml"
    mv "$dir/keys.new" "$dir/keys"
    echo "named the existing client keys imported-1.. in $dir/keys (deploy/keys.sh rename renames them)"
}

# Writes secrets/server.toml.new from a keys file ($1, default secrets/keys) and secrets/demo.key,
# root:65534 0640; the caller renames it into place.
render_server_toml() {
    local dir=$root/secrets name key streams keys=${1:-$root/secrets/keys}
    (
        umask 077
        {
            echo '# Written by deploy/install.sh and deploy/keys.sh from keys and demo.key; edit those instead.'
            echo '# listen and worker_socket match deploy/compose.yaml.'
            echo 'listen = "0.0.0.0:8880"'
            echo 'worker_socket = "/run/kokoro/worker.sock"'
            echo 'api_keys = ['
            while IFS=$'\t' read -r name key streams || [ -n "$name" ]; do
                # Whatever edited the file, nothing but these may reach the TOML.
                [[ $name =~ ^[a-z0-9][a-z0-9-]{0,31}$ && $key =~ ^[A-Za-z0-9._~+/=-]{16,}$ && $streams =~ ^(-|[1-9][0-9]{0,3})$ ]] ||
                    die "$keys has a line that isn't NAME<tab>KEY<tab>MAX_STREAMS (or -): $name"
                echo "    # $name"
                if [ "$streams" = - ]; then
                    echo "    \"$key\","
                else
                    echo "    { key = \"$key\", max_streams = $streams },"
                fi
            done < "$keys"
            echo "    # demo (demo.key)"
            echo "    { key = \"$(cat "$dir/demo.key")\", max_streams = 4 },"
            echo ']'
        } > "$dir/server.toml.new"
    )
    chgrp "$nobody" "$dir/server.toml.new"
    chmod 0640 "$dir/server.toml.new"
}

# Builds the images and the compile cache for a new release, while whatever runs keeps running.
# Sets $rel, and $pending_release until the caller has started it.
stage_release() {
    local dir
    rel=$(date -u +%Y%m%dT%H%M%SZ)
    dir=$root/releases/$rel
    [ ! -e "$dir" ] || die "$dir exists"
    pending_release=$rel
    prepare_cloudflared
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
    # --remove-orphans keeps containers of services that are only switched off by a profile, so a
    # tunnel turned off in settings.env would keep running.
    if ! grep -qx 'COMPOSE_PROFILES=cloudflared' "$root/current/.env"; then
        compose rm -sf cloudflared > /dev/null || return 1
    fi
    compose up -d --remove-orphans && wait_healthy && wait_tunnel
}

# With the tunnel on, waits for cloudflared to register a connection with Cloudflare, so a bad or
# revoked token fails the install or update (and rolls back) instead of restarting forever.
wait_tunnel() {
    grep -qx 'COMPOSE_PROFILES=cloudflared' "$root/current/.env" || return 0
    echo "== waiting for the Cloudflare tunnel to connect"
    for _ in $(seq 120); do
        compose logs --no-log-prefix cloudflared 2> /dev/null | grep -q 'Registered tunnel connection' && return 0
        sleep 1
    done
    echo "cloudflared didn't connect in 2 minutes:" >&2
    compose logs --tail 20 cloudflared >&2 || true
    return 1
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
