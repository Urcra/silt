#!/bin/sh
# Installs the `silt` binary from a GitHub release (ADR 0043):
#   curl -fsSL https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.sh | sh
#
#   SILT_VERSION       a release tag such as v0.1.0; the latest by default
#   SILT_INSTALL_DIR   where `silt` goes; ~/.local/bin by default
#   SILT_DOWNLOAD_URL  where the archives and SHA256SUMS are, in place of
#                      the release's own URL
set -eu

repo=https://github.com/Urcra/silt
dir=${SILT_INSTALL_DIR:-$HOME/.local/bin}

fail() {
    echo "silt install: $*" >&2
    exit 1
}

case "$(uname -s)" in
    Linux) ;;
    MINGW* | MSYS* | CYGWIN*) fail "on Windows, run in PowerShell: irm https://raw.githubusercontent.com/Urcra/silt/main/scripts/install.ps1 | iex" ;;
    *) fail "there are release binaries for Linux only so far; build from source: cargo install --locked --git $repo silt-cli" ;;
esac
case "$(uname -m)" in
    x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
    aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
    *) fail "no release binary for $(uname -m)" ;;
esac

if [ -n "${SILT_DOWNLOAD_URL:-}" ]; then
    url=$SILT_DOWNLOAD_URL
elif [ -n "${SILT_VERSION:-}" ]; then
    url=$repo/releases/download/$SILT_VERSION
else
    url=$repo/releases/latest/download
fi

if command -v curl >/dev/null; then
    fetch() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null; then
    fetch() { wget -q "$1" -O "$2"; }
else
    fail "needs curl or wget"
fi
if command -v sha256sum >/dev/null; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    fail "needs sha256sum or shasum to check the download"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
archive=silt-$target.tar.gz
echo "silt install: downloading $url/$archive"
fetch "$url/$archive" "$work/$archive" || fail "could not download $url/$archive"
fetch "$url/SHA256SUMS" "$work/SHA256SUMS" || fail "could not download $url/SHA256SUMS"
expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' "$work/SHA256SUMS")
[ -n "$expected" ] || fail "SHA256SUMS lists no $archive"
[ "$(sha256 "$work/$archive")" = "$expected" ] || fail "$archive doesn't match its checksum; nothing installed"

tar -xzf "$work/$archive" -C "$work"
[ -f "$work/silt" ] || fail "$archive holds no silt binary"
mkdir -p "$dir"
# Renamed into place, so a running `silt` keeps its old binary.
cp "$work/silt" "$dir/.silt.new"
chmod 755 "$dir/.silt.new"
mv "$dir/.silt.new" "$dir/silt"
echo "silt install: installed $("$dir/silt" --version) to $dir/silt"

case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "silt install: $dir isn't on your PATH; add it, for example: echo 'export PATH=\"$dir:\$PATH\"' >> ~/.profile" ;;
esac
echo "silt install: next, import the sessions you already have: silt import"
