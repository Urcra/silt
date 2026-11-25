#!/bin/sh
# Builds the bubblewrap and pasta that Silt's Linux release archives carry beside
# `silt`, as `silt-bwrap` and `silt-pasta`, linked statically so they run on
# any distribution (ADR 0093). Their licenses and where their sources are go
# in `licenses/`. Needs curl, meson, ninja, make and libcap's static library
# (Debian's and Ubuntu's `libcap-dev`).
# Usage: scripts/build-sandbox-tools.sh <out-dir>
set -eu

BWRAP_VERSION=0.13.1
BWRAP_SHA256=0e85c8702248b8dd64144f594147553650cdd0e4c951690345faace79c217c4d
PASST_VERSION=2026_10_02.cba3570
PASST_SHA256=aa75616cc43925f0bd6c05bcfbbaecd77397c03d7bd23591b34ef535be5dfcdc

[ $# -eq 1 ] || { echo "usage: $0 <out-dir>" >&2; exit 2; }
mkdir -p "$1/licenses"
out=$(cd "$1" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

bwrap_url=https://github.com/containers/bubblewrap/releases/download/v$BWRAP_VERSION/bubblewrap-$BWRAP_VERSION.tar.xz
passt_url=https://passt.top/passt/snapshot/passt-$PASST_VERSION.tar.xz
curl -fsSL -o bwrap.tar.xz "$bwrap_url"
curl -fsSL -o passt.tar.xz "$passt_url"
printf '%s  %s\n%s  %s\n' "$BWRAP_SHA256" bwrap.tar.xz "$PASST_SHA256" passt.tar.xz | sha256sum -c -
tar -xf bwrap.tar.xz
tar -xf passt.tar.xz

(
    cd "bubblewrap-$BWRAP_VERSION"
    meson setup _build --prefer-static -Dc_link_args=-static -Dselinux=disabled -Dman=disabled -Dtests=false \
        -Dbash_completion=disabled -Dzsh_completion=disabled
    ninja -C _build bwrap
)
# pasta is passt under another name: it reads which from its own.
make -C "passt-$PASST_VERSION" static VERSION="$PASST_VERSION"

cp "bubblewrap-$BWRAP_VERSION/_build/bwrap" "$out/silt-bwrap"
cp "passt-$PASST_VERSION/passt" "$out/silt-pasta"
tools="silt-bwrap silt-pasta"
# On x86-64 it runs its AVX2 build, beside it by its name, where it can.
if [ -f "passt-$PASST_VERSION/passt.avx2" ]; then
    cp "passt-$PASST_VERSION/passt.avx2" "$out/silt-pasta.avx2"
    tools="$tools silt-pasta.avx2"
fi
for tool in $tools; do
    strip "$out/$tool"
    file "$out/$tool" | grep -q 'statically linked' || { echo "$tool isn't statically linked" >&2; exit 1; }
done
"$out/silt-bwrap" --version
"$out/silt-pasta" --version | head -1

cp "bubblewrap-$BWRAP_VERSION/LICENSE" "$out/licenses/bubblewrap-LGPL-2.1-or-later.txt"
cp "passt-$PASST_VERSION/LICENSES/GPL-2.0-or-later.txt" "$out/licenses/passt-GPL-2.0-or-later.txt"
cp "passt-$PASST_VERSION/LICENSES/BSD-3-Clause.txt" "$out/licenses/passt-BSD-3-Clause.txt"
cat >"$out/licenses/README" <<EOF
silt-bwrap is bubblewrap $BWRAP_VERSION, under the LGPL 2.1 or later; its source:
$bwrap_url
silt-pasta is passt $PASST_VERSION (run as pasta), under the GPL 2.0 or later; its source:
$passt_url
Both are unmodified, built by scripts/build-sandbox-tools.sh in Silt's repository.
EOF
