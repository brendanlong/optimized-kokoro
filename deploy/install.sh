#!/bin/bash
# Installs the server, worker and demo page on the host's system Docker (deploy/compose.yaml). Run
# once, with sudo from your own account, from a checkout only you and root can change:
#
#   sudo deploy/install.sh [--keys-from FILE] [--models-from DIR] [--unidic-from FILE]
#
#   --keys-from FILE    the clients' API keys: a KOKORO_API_KEYS=a,b line or one key per line
#                       (default: one new key)
#   --models-from DIR   config.json, model.safetensors and voices.safetensors from kokoro_worker.convert
#                       (default: convert them as you with uv)
#   --unidic-from FILE  a converted unidic.dic, with FILE.LICENSE next to it, checked against
#                       fetch_unidic.sh's checksum (default: fetch and convert it as you with cargo)
#
# Each file is read as its owner (sudo -u OWNER cat), so it can be anyone's. Settings (GPU, listen
# addresses) are in /srv/optimized-kokoro/settings.env, made on the first run: edit it and run again
# to change them before the first start. Later updates: deploy/update.sh. See deploy/README.md.
set -euo pipefail
# shellcheck source=deploy/lib.sh
. "$(dirname "$0")/lib.sh"

usage() {
    sed -n '2,17p' "$0"
    exit 2
}

keys_from='' models_from='' unidic_from=''
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || usage
    case $1 in
    --keys-from) keys_from=$2 ;;
    --models-from) models_from=$2 ;;
    --unidic-from) unidic_from=$2 ;;
    *) usage ;;
    esac
    shift 2
done

require_root
take_lock
check_checkout
[ ! -e "$root/current" ] || die "already installed ($root/current); use deploy/update.sh"
check_driver
ensure_dirs
check_cdi
check_port_free "$KOKORO_LISTEN"
check_port_free "$KOKORO_DEMO_LISTEN"
ensure_runtimes
# Leftovers of a failed install.
sweep_projects
prune_releases
ensure_unidic "$unidic_from"
ensure_models "$models_from"
ensure_secrets "$keys_from"
stage_release

# Takes a release that didn't come up (or was interrupted) down again; cleanup then removes it.
take_down() {
    if [ -e "$root/current" ]; then
        show_logs "$rel"
        compose down --volumes || true
    fi
    rm -f "$root/current"
}

echo "== start $rel"
on_abort=take_down
if ! { point_current "$rel" && start_current; }; then die "install failed"; fi
on_abort=
pending_release=

cat << EOF

Up: the server on $KOKORO_LISTEN, the demo page on $KOKORO_DEMO_LISTEN.
  sudo docker compose -f $root/current/compose.yaml ps        # or logs -f
The clients' API keys are in $root/secrets/server.toml (sudo cat it). Check the deployment with the
list in deploy/README.md. The demo page needs HTTPS (a secure context) for WebCodecs; over plain
HTTP it falls back to a WASM encoder.
EOF
