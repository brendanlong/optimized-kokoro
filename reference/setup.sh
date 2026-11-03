#!/bin/sh
# Installs the deployed Kokoro-FastAPI at a pinned commit; its text pipeline is the reference the port must match.
set -eu
KFA_SHA=a3508fa0826286073289371de284d17f3d6d88d1
cd "$(dirname "$0")"
mkdir -p .cache
if [ ! -d .cache/kfa ]; then
  git clone -q https://github.com/brendanlong/Kokoro-FastAPI .cache/kfa
fi
git -C .cache/kfa fetch -q origin "$KFA_SHA"
git -C .cache/kfa checkout -q "$KFA_SHA"
cd .cache/kfa
uv sync --no-config --frozen --extra cpu --python 3.12
# misaki's Japanese G2P (cutlet) needs the full UniDic dictionary: the version the deployed image has.
UNIDIC=3.1.0+2021-08-31
DICDIR="$(.venv/bin/python -I -c 'import unidic; print(unidic.DICDIR)')"
if [ "$(cat "$DICDIR/version" 2>/dev/null)" != "unidic-$UNIDIC" ]; then
  .venv/bin/python -I -m unidic download "$UNIDIC"
  [ "$(cat "$DICDIR/version")" = "unidic-$UNIDIC" ]
fi
# espeak-ng keeps its data path in a fixed-size buffer that the venv's site-packages path overflows,
# so dump.py points it at a copy of the same data with a shorter path.
rm -rf ../espeak-ng-data
cp -r "$(.venv/bin/python -I -c 'import espeakng_loader; print(espeakng_loader.get_data_path())')" ../espeak-ng-data
