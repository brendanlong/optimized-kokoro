# shellcheck shell=bash
# Shared by install.sh and update.sh (sourced, not run). Runs as root.
#
# Layout of $root (/srv/optimized-kokoro, everything root-owned):
#   settings.env            KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN, KOKORO_MAX_BATCH,
#                           KOKORO_MAX_CHUNK_SECONDS, KOKORO_CLOUDFLARED
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
#   .lock                   held by install.sh, update.sh and keys.sh while they run
# Images are tagged optimized-kokoro-{worker,server,demo}:REL. Each release is its own Compose
# project, optimized-kokoro-rel (REL in lower case, COMPOSE_PROJECT_NAME in its .env), with its own
# socket volume and network, so update.sh can start a new release's worker next to the running
# release.

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
    # A second Ctrl-C would cut the undoing short and leave nothing serving. Ignored signals stay
    # ignored in what this runs (docker compose too), so it finishes.
    trap '' INT TERM
    gpu_watch_stop
    if [ -n "$abort" ]; then
        echo "== undoing: $abort (Ctrl-C is ignored until it's done)" >&2
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

# The release current points at.
current_release() {
    basename "$(readlink "$root/current")"
}

# The Compose project stage_release gives release $1 (Compose wants lower case).
new_project() {
    echo "optimized-kokoro-${1,,}"
}

# The Compose project of release $1: COMPOSE_PROJECT_NAME in its .env, or new_project before it
# has a .env.
project_of() {
    local env=$root/releases/$1/.env p
    [ -f "$env" ] || {
        new_project "$1"
        return
    }
    p=$(sed -n 's/^COMPOSE_PROJECT_NAME=//p' "$env")
    [ -n "$p" ] || die "$env has no COMPOSE_PROJECT_NAME"
    echo "$p"
}

# docker compose for release $1. The project is in its .env too, so a plain
# docker compose -f .../compose.yaml finds it as well; -p only makes it explicit.
compose_rel() {
    local r=$1
    shift
    docker compose -p "$(project_of "$r")" -f "$root/releases/$r/compose.yaml" "$@"
}

# docker compose for the release current points at.
compose() {
    compose_rel "$(current_release)" "$@"
}

# Removes Compose project $1's containers, networks and volumes, by their labels (so it needs no
# compose file, and also finds what an interrupted run left). Never fails: it also runs in cleanup.
remove_project() {
    local kind ids
    for kind in container network volume; do
        case $kind in
        container) ids=$(docker ps -aq --filter "label=com.docker.compose.project=$1" 2> /dev/null) || ids= ;;
        *) ids=$(docker "$kind" ls -q --filter "label=com.docker.compose.project=$1" 2> /dev/null) || ids= ;;
        esac
        [ -n "$ids" ] || continue
        # shellcheck disable=SC2086
        docker "$kind" rm -f $ids > /dev/null || echo "couldn't remove all of $1's ${kind}s" >&2
    done
    return 0
}

# Removes every project of ours (optimized-kokoro-REL) except $1: what an update or install that
# was killed before it could clean up left, such as a new worker holding GPU memory, or a network
# holding the subnet.
sweep_projects() {
    local p
    for p in $({
        docker ps -a --format '{{.Label "com.docker.compose.project"}}'
        docker network ls --format '{{.Label "com.docker.compose.project"}}'
        docker volume ls --format '{{.Label "com.docker.compose.project"}}'
    } | grep -xE 'optimized-kokoro-[0-9]{8}t[0-9]{6}z' | sort -u); do
        [ "$p" != "${1:-}" ] || continue
        echo "== removing the Compose project $p, left by an interrupted install or update"
        remove_project "$p"
    done
}

new_key() {
    # 32 URL-safe characters (192 bits).
    head -c 24 /dev/urandom | base64 | tr '+/' '-_'
}

# The gVisor runtimes compose.yaml uses (see there for the flags), so each container gets only the
# host Unix socket access it needs.
kokoro_runtimes='{
    "runsc-kokoro-uds-create": ["--directfs=false", "--host-uds=create"],
    "runsc-kokoro-uds-open": ["--directfs=false", "--host-uds=open"],
    "runsc-kokoro-no-uds": ["--directfs=false"]
}'

# Adds $kokoro_runtimes to daemon.json and removes runsc-kokoro (the single runtime, with
# --host-uds=all, of releases before the split) once no kept release names it, then reloads Docker,
# which picks up runtimes without restarting any container. The backup is the daemon.json from
# before the first change.
ensure_runtimes() {
    local runsc clash keep_old=false
    runsc=$(command -v runsc)
    [ -f "$daemon_json" ] || echo '{}' > "$daemon_json"
    # Any runsc path will do, as long as the flags match.
    clash=$(jq -r --argjson want "$kokoro_runtimes" \
        '.runtimes // {} | to_entries[] | select($want[.key] and .value.runtimeArgs != $want[.key]) | .key' "$daemon_json")
    [ -z "$clash" ] || die "$daemon_json already has different runtimes named ${clash//$'\n'/ }; their runtimeArgs must be: $(jq -c . <<< "$kokoro_runtimes")"
    ! grep -qsx '    runtime: runsc-kokoro' "$root"/releases/*/compose.yaml || keep_old=true
    jq --arg path "$runsc" --argjson want "$kokoro_runtimes" --argjson keep_old "$keep_old" '
        .runtimes = ($want | map_values({path: $path, runtimeArgs: .}))
            + (.runtimes // {} | if $keep_old then . else del(.["runsc-kokoro"]) end)' \
        "$daemon_json" > "$daemon_json.new"
    if jq -e --slurpfile old "$daemon_json" '. == $old[0]' "$daemon_json.new" > /dev/null; then
        rm "$daemon_json.new"
        return
    fi
    [ -e "$daemon_json.bak-kokoro" ] || cp -p "$daemon_json" "$daemon_json.bak-kokoro"
    echo "== changing the runtimes in $daemon_json (backup: $daemon_json.bak-kokoro)"
    jq -r --slurpfile old "$daemon_json" '(.runtimes | keys) as $now | ($old[0].runtimes // {} | keys) as $was
        | ($now - $was | map("adding " + .)) + ($was - $now | map("removing " + .)) | .[]' "$daemon_json.new"
    chmod --reference="$daemon_json" "$daemon_json.new"
    mv "$daemon_json.new" "$daemon_json"
    systemctl reload docker
    for _ in $(seq 30); do
        docker info --format '{{json .Runtimes}}' 2> /dev/null |
            jq -e --argjson want "$kokoro_runtimes" '($want | keys) - keys == []' > /dev/null && return
        sleep 1
    done
    die "Docker didn't pick up the runtimes after a reload; check journalctl -u docker"
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
# The worker's --max-batch, 1-64. 8 serves many listeners faster than 1 at about the same latency for
# one, in no more GPU memory (worker/README.md). The cache seed depends on it.
#KOKORO_MAX_BATCH=8
# The worker's --max-chunk-seconds, 25-60: the most audio one request to the worker may make, at
# any speed. It sets the worker's peak GPU memory (deploy/README.md); the server splits longer chunks.
#KOKORO_MAX_CHUNK_SECONDS=60
# 1 runs a Cloudflare Tunnel to the server, with the tunnel token in secrets/cloudflared.token
# (deploy/README.md, "Cloudflare Tunnel").
#KOKORO_CLOUDFLARED=0
EOF
        chmod 0644 "$root/settings.env"
    fi
    load_settings
}

# Reads settings.env into KOKORO_GPU, KOKORO_LISTEN, KOKORO_DEMO_LISTEN, KOKORO_MAX_BATCH,
# KOKORO_MAX_CHUNK_SECONDS and KOKORO_CLOUDFLARED (with defaults); anything else in it is an
# error, not dropped.
load_settings() {
    local line n=0 key value
    KOKORO_GPU=0 KOKORO_LISTEN=127.0.0.1:8880 KOKORO_DEMO_LISTEN=127.0.0.1:25436 KOKORO_MAX_BATCH=8 KOKORO_MAX_CHUNK_SECONDS=60
    KOKORO_CLOUDFLARED=0
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
        # Up to the server's ceiling, worker_max_chunk_samples (60 s). Below 25 s, requests at speed
        # 0.25 would be split into windows of fewer than 32 ids (stitch.rs max_render_ids).
        KOKORO_MAX_CHUNK_SECONDS) [[ $value =~ ^[0-9]{1,2}$ ]] && ((10#$value >= 25 && 10#$value <= 60)) ;;
        KOKORO_CLOUDFLARED) [[ $value =~ ^[01]$ ]] ;;
        *) false ;;
        esac || die "$root/settings.env line $n isn't a valid KOKORO_GPU (a CDI GPU name, not all), KOKORO_LISTEN or KOKORO_DEMO_LISTEN (IPv4:port), KOKORO_MAX_BATCH (1-64), KOKORO_MAX_CHUNK_SECONDS (25-60) or KOKORO_CLOUDFLARED (0 or 1): $line"
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
    echo "KOKORO_MAX_CHUNK_SECONDS=$KOKORO_MAX_CHUNK_SECONDS"
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
# read them and no other host user does. The demo's key may use 4 streams at most, at priority 64 at most,
# which keeps capacity for clients and puts them first.
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
            printf '%s\t%s\t-\t-\n' "$name" "$k"
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
    other=$(grep -vE '^(#.*|listen = "0\.0\.0\.0:8880"|worker_socket = "/run/kokoro/worker\.sock"|api_keys = \[|    "[^"]*",|    # .*|    \{ key = "[^"]*", max_streams = 4(, max_priority = 64)? \},|\])$' "$dir/server.toml" || true)
    [ -z "$other" ] || die "$dir/server.toml has lines install.sh didn't write, which keys.sh would drop:
$other
Move them out (or into secrets/keys by hand: NAME<tab>KEY<tab>MAX_STREAMS<tab>MAX_PRIORITY, each a number or -) first."
    (
        umask 077
        while IFS= read -r k; do
            i=$((i + 1))
            printf 'imported-%s\t%s\t-\t-\n' "$i" "$k"
        done < <(sed -n 's/^    "\([^"]*\)",$/\1/p' "$dir/server.toml") > "$dir/keys.new"
    )
    [ -s "$dir/keys.new" ] || die "found no client keys in $dir/server.toml"
    mv "$dir/keys.new" "$dir/keys"
    echo "named the existing client keys imported-1.. in $dir/keys (deploy/keys.sh rename renames them)"
}

# Writes secrets/server.toml.new from a keys file ($1, default secrets/keys) and secrets/demo.key,
# root:65534 0640; the caller renames it into place.
render_server_toml() {
    local dir=$root/secrets name key streams priority fields keys_file=${1:-$root/secrets/keys}
    (
        umask 077
        {
            echo '# Written by deploy/install.sh and deploy/keys.sh from keys and demo.key; edit those instead.'
            echo '# listen and worker_socket match deploy/compose.yaml.'
            echo 'listen = "0.0.0.0:8880"'
            echo 'worker_socket = "/run/kokoro/worker.sock"'
            echo 'api_keys = ['
            while IFS=$'\t' read -r name key streams priority || [ -n "$name" ]; do
                # Lines written before MAX_PRIORITY existed have three fields: the server's default.
                priority=${priority:--}
                # Whatever edited the file, nothing but these may reach the TOML.
                if ! [[ $name =~ ^[a-z0-9][a-z0-9-]{0,31}$ && $key =~ ^[A-Za-z0-9._~+/=-]{16,}$ && $streams =~ ^(-|[1-9][0-9]{0,3})$ &&
                    ($priority = - || ($priority =~ ^(0|[1-9][0-9]{0,2})$ && $priority -le 255)) ]]; then
                    die "$keys_file has a line that isn't NAME<tab>KEY<tab>MAX_STREAMS<tab>MAX_PRIORITY (each limit a number or -; MAX_PRIORITY 0-255, or left out): $name"
                fi
                echo "    # $name"
                fields=
                [ "$streams" = - ] || fields+=", max_streams = $streams"
                [ "$priority" = - ] || fields+=", max_priority = $priority"
                if [ -z "$fields" ]; then
                    echo "    \"$key\","
                else
                    echo "    { key = \"$key\"$fields },"
                fi
            done < "$keys_file"
            # The demo page gets 4 streams, and priority 64 at most (its requests run at 64), so it
            # can neither take the clients' capacity nor go ahead of their default priority.
            echo "    # demo (demo.key)"
            echo "    { key = \"$(cat "$dir/demo.key")\", max_streams = 4, max_priority = 64 },"
            echo ']'
        } > "$dir/server.toml.new"
    )
    chgrp "$nobody" "$dir/server.toml.new"
    chmod 0640 "$dir/server.toml.new"
}

# Builds the images and the compile cache for a new release, while whatever runs keeps running.
# Sets $rel, and $pending_release until the caller has started it, and $gpu_need: the GPU memory
# (MiB) the new worker's warmup took, if nvidia-smi could tell.
stage_release() {
    local dir before peak
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
        echo "COMPOSE_PROJECT_NAME=$(new_project "$rel")"
        settings_env
    } > "$dir/.env"
    chmod 0644 "$dir/.env"
    compose_rel "$rel" config --quiet

    echo "== compile cache (warms up in gVisor next to the running worker, then exits without listening)"
    # Written by nobody into a directory only it can write, then handed to root and mounted
    # read-only from then on. Same image and flags as the worker that will load it.
    install -d -m 0700 "$dir/cache-seed"
    chown "$nobody:$nobody" "$dir/cache-seed"
    before=$(gpu_used)
    gpu_watch_start export
    compose_rel "$rel" run --rm --no-deps -T -e KOKORO_CACHE_SEED= \
        -v "$dir/cache-seed:/cache-export" worker \
        --max-batch "$KOKORO_MAX_BATCH" --max-chunk-seconds "$KOKORO_MAX_CHUNK_SECONDS" --export-cache /cache-export
    gpu_watch_stop
    peak=$(gpu_peak export)
    gpu_need=
    if [ -n "$before" ] && [ -n "$peak" ]; then
        gpu_need=$((peak > before ? peak - before : 0))
        echo "GPU memory during the export: $before MiB before, $peak MiB at the peak (the warmup took $gpu_need MiB)"
    fi
    chown -hR root:root "$dir/cache-seed"
    chmod -R u=rwX,go=rX "$dir/cache-seed"
    [ -n "$(ls -A "$dir/cache-seed")" ] || die "the compile cache is empty"
}

# Points current at releases/$1 (by rename).
point_current() {
    ln -sfn "releases/$1" "$root/current.new" && mv -T "$root/current.new" "$root/current"
}

# Waits for the server of the release current points at, and sets $healthy_at ($EPOCHREALTIME)
# when it answers. Polls often: after update.sh's swap, this wait is the downtime.
healthy_at=
# shellcheck disable=SC2034 # healthy_at is update.sh's
wait_healthy() {
    local url end=$((SECONDS + 600))
    url=$(health_url)
    # The worker takes about 3.5 minutes to start in gVisor with directfs off, even with a seed.
    echo "== waiting for $url"
    for _ in $(seq 3000); do
        if curl -sf -o /dev/null --max-time 2 "$url"; then
            healthy_at=$EPOCHREALTIME
            return 0
        fi
        ((SECONDS < end)) || break
        sleep 0.2
    done
    echo "not healthy after 10 minutes" >&2
    return 1
}

# Seconds from $1 to $2 (both $EPOCHREALTIME), to a tenth.
elapsed() {
    local d=$((10#${2//[.,]/} - 10#${1//[.,]/}))
    d=$((d / 100000))
    echo "$((d / 10)).$((d % 10))"
}

# Waits for release $1's worker to listen on its socket, which it does only once it has warmed up.
# Fails at once if the container stops or restarts (out of GPU memory, say), or after 10 minutes.
wait_worker() {
    local id logs status=missing restarts=0
    echo "== waiting for $1's worker to warm up (about as long as the cache export took)"
    for _ in $(seq 300); do
        logs=$(compose_rel "$1" logs --no-log-prefix worker 2> /dev/null || true)
        if grep -q 'listening on /run/kokoro/worker.sock' <<< "$logs"; then
            echo "$1's worker is ready"
            return 0
        fi
        id=$(compose_rel "$1" ps -aq worker 2> /dev/null || true)
        status=missing restarts=0
        if [ -n "$id" ]; then
            read -r status restarts < <(docker inspect -f '{{.State.Status}} {{.RestartCount}}' "$id" 2> /dev/null || echo missing 0)
        fi
        if [ "$status" != running ] || [ "$restarts" != 0 ]; then
            echo "$1's worker stopped before it was ready ($status, restarted $restarts times)" >&2
            break
        fi
        sleep 2
    done
    [ "$status" != running ] || [ "$restarts" != 0 ] || echo "$1's worker wasn't ready after 10 minutes" >&2
    compose_rel "$1" logs --no-color --tail 40 worker >&2 || true
    return 1
}

# Removes release $1's server, demo and tunnel, and then its network, which frees the published
# ports and the subnet (172.30.88.0/24) for the next release's network. Its worker keeps running,
# so the release comes back in seconds (start_current) if the next one doesn't.
stop_front() {
    local net
    compose_rel "$1" rm -sf server demo cloudflared || return 1
    net="$(project_of "$1")_kokoro"
    if docker network inspect "$net" > /dev/null 2>&1 && ! docker network rm "$net" > /dev/null; then
        echo "couldn't remove the network $net; still attached: $(docker network inspect -f '{{range .Containers}}{{.Name}} {{end}}' "$net" 2> /dev/null)" >&2
        return 1
    fi
}

# KOKORO_GPU's memory in MiB: in use (gpu_used) and in all (gpu_total). Nothing if nvidia-smi
# can't name the CDI device (a MIG device, say).
gpu_query() {
    nvidia-smi -i "$KOKORO_GPU" --query-gpu="$1" --format=csv,noheader,nounits 2> /dev/null | sed -n '1{/^[0-9][0-9]*$/p}' || true
}
gpu_used() { gpu_query memory.used; }
gpu_total() { gpu_query memory.total; }

# Samples KOKORO_GPU's memory in use about twice a second into log $1 until gpu_watch_stop;
# gpu_peak $1 is the highest sample (MiB), or nothing. A loop rather than nvidia-smi -lms, whose
# output to a file may sit in its buffer until it's killed.
gpu_dir='' gpu_pid=''
gpu_watch_start() {
    [ -n "$gpu_dir" ] || make_tmp gpu_dir
    # Without the lock's descriptor, so nothing it leaves behind could hold the lock.
    (
        while :; do
            gpu_used
            sleep 0.5
        done
    ) > "$gpu_dir/$1" 9>&- &
    gpu_pid=$!
}
gpu_watch_stop() {
    [ -n "$gpu_pid" ] || return 0
    kill "$gpu_pid" 2> /dev/null || true
    wait "$gpu_pid" 2> /dev/null || true
    gpu_pid=
}
gpu_peak() {
    { grep -xE '[0-9]+' "$gpu_dir/$1" 2> /dev/null || true; } | sort -n | tail -1
}

# Before a second worker starts next to the running one: fails if KOKORO_GPU hasn't the memory the
# new worker's warmup took during the export, with 256 MiB to spare (running out could also fail
# the running worker's requests).
check_gpu_room() {
    local used total
    used=$(gpu_used) total=$(gpu_total)
    if [ -z "${gpu_need:-}" ] || [ -z "$used" ] || [ -z "$total" ]; then
        echo "(nvidia-smi -i $KOKORO_GPU doesn't show the GPU's memory, so not checking that a second worker fits)"
        return 0
    fi
    echo "GPU memory: $used of $total MiB in use; the new worker's warmup took $gpu_need MiB"
    ((total - used >= gpu_need + 256)) ||
        die "not enough free GPU memory for a second worker: $((total - used)) MiB, against the $gpu_need MiB its warmup took and 256 to spare. Nothing was changed; try again when there's room"
}

# Starts the release current points at (recreating containers whose image or config changed), with
# that release's settings.
start_current() {
    local r
    r=$(current_release)
    KOKORO_LISTEN=$(release_setting "$r" LISTEN)
    # --remove-orphans keeps containers of services that are only switched off by a profile, so a
    # tunnel turned off in settings.env would keep running.
    if ! grep -qx 'COMPOSE_PROFILES=cloudflared' "$root/current/.env"; then
        compose rm -sf cloudflared > /dev/null || return 1
    fi
    compose up -d --remove-orphans && wait_healthy && wait_tunnel
}

# With the tunnel on, waits for cloudflared to register a connection with Cloudflare, so a bad or
# revoked token fails the install or update (and rolls back) instead of restarting forever. Sets
# $tunnel_at ($EPOCHREALTIME) when it has.
tunnel_at=
# shellcheck disable=SC2034 # tunnel_at is update.sh's
wait_tunnel() {
    local logs
    grep -qx 'COMPOSE_PROFILES=cloudflared' "$root/current/.env" || return 0
    echo "== waiting for the Cloudflare tunnel to connect"
    for _ in $(seq 240); do
        logs=$(compose logs --no-log-prefix cloudflared 2> /dev/null || true)
        if grep -q 'Registered tunnel connection' <<< "$logs"; then
            tunnel_at=$EPOCHREALTIME
            return 0
        fi
        sleep 0.5
    done
    echo "cloudflared didn't connect in 2 minutes:" >&2
    compose logs --tail 20 cloudflared >&2 || true
    return 1
}

# Removes a release's Compose project, images and directory, and the dictionaries no release names
# any more. The project only if it isn't the running one.
remove_release() {
    local i p
    p=$(project_of "$1")
    if [ ! -L "$root/current" ] || [ "$p" != "$(project_of "$(current_release)")" ]; then
        remove_project "$p"
    fi
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

# The last log lines of release $1.
show_logs() {
    echo "--- last log lines of $1" >&2
    compose_rel "$1" logs --no-color --tail 40 >&2 || true
}
