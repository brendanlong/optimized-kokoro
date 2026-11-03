#!/bin/sh
# Downloads UniDic 3.1.0 (2021-08-31; the release `python -m unidic download` installs, which the
# reference uses), checks it, and converts its MeCab dictionary into crates/g2p-ja/data/unidic.dic
# (or $1), the file kokoro-g2p-ja memory-maps. Never overwrite that file in place while a server
# runs: this script (and deploy/deploy.sh) replaces it by rename. UniDic is used under its BSD-3-Clause license (it is
# triple-licensed BSD-3 / GPL-2 / LGPL-2.1); the license travels with the converted file as
# data/unidic.dic.LICENSE.
#
#   crates/g2p-ja/tools/fetch_unidic.sh [OUT]
#
# Needs curl, unzip, sha256sum and cargo. The download is 526 MB; the output is 516 MB.
set -eu

URL=https://cotonoha-dic.s3-ap-northeast-1.amazonaws.com/unidic-3.1.0.zip
ZIP_SHA256=638718c4c63625ab300de4c92c67925d54c0e9e3830009eaa992f29819d59c43
OUT_SHA256=b1134a847fb17921a7cf5625bf69a745ad356636b8b1294f1f96bc97bb8c256f

here=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$here/data/unidic.dic}
if [ -f "$out" ] && echo "$OUT_SHA256  $out" | sha256sum -c --status -; then
  echo "$out is already this dictionary" >&2
  exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "downloading $URL" >&2
curl -fsSL -o "$tmp/unidic.zip" "$URL"
echo "$ZIP_SHA256  $tmp/unidic.zip" | sha256sum -c - >&2
mkdir "$tmp/zip"
unzip -q "$tmp/unidic.zip" 'unidic/sys.dic' 'unidic/unk.dic' 'unidic/char.bin' 'unidic/matrix.bin' 'unidic/licenses/BSD' -d "$tmp/zip"
cargo run --quiet --release --manifest-path "$here/../../Cargo.toml" -p kokoro-g2p-ja --example build_dict -- \
  "$tmp/zip/unidic" "$tmp/unidic.dic" "$here/data/context_order.txt"
echo "$OUT_SHA256  $tmp/unidic.dic" | sha256sum -c - >&2
# The server memory-maps the dictionary, so a running one must never see it change: copy it next
# to the destination, then rename it over the old one (the old inode stays intact while mapped).
cp "$tmp/zip/unidic/licenses/BSD" "$out.LICENSE"
cp "$tmp/unidic.dic" "$out.new.$$"
mv -f "$out.new.$$" "$out"
echo "wrote $out" >&2
