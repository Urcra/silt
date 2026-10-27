#!/usr/bin/env bash
# From WSL, builds this tree with Windows' own cargo and installs `silt.exe`
# and `silt-desktop.exe` where the desktop installer puts them (ADR 0045).
# Run by `just install-wsl`, after `web/dist` is built.
#
# Windows' cargo builds a copy on its own drive: from \\wsl.localhost it
# would read every source over 9P, and build scripts would see UNC paths.
set -euo pipefail

root=$(git rev-parse --show-toplevel)
cd /mnt/c

[ -f "$root/web/dist/index.html" ] || { echo "install-windows: build the web UI first: just build-web" >&2; exit 1; }
command -v powershell.exe >/dev/null || { echo "install-windows: needs WSL with Windows interop" >&2; exit 1; }

windows() {
    powershell.exe -NoProfile -NonInteractive -Command "$1" | tr -d '\r'
}

local_app_data=$(windows '[Environment]::GetFolderPath("LocalApplicationData")')
src=$(wslpath "$local_app_data")/silt-build
app=$(wslpath "$local_app_data")/Silt

windows 'if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { exit 1 }' \
    || { echo "install-windows: Windows has no cargo; install Rust there from https://rustup.rs" >&2; exit 1; }

files=$(mktemp)
trap 'rm -f "$files"' EXIT
{
    git -C "$root" ls-files -co --exclude-standard
    (cd "$root" && find web/dist -type f)
} | sort -u >"$files"

echo "install-windows: copying the tree to $src"
mkdir -p "$src"
rsync -rlt --files-from="$files" "$root/" "$src/"
# A file deleted here still in the copy, such as a migration, would be built in.
(cd "$src" && find . \( -path ./target -o -path ./crates/silt-desktop/target \) -prune -o -type f -print | sed 's|^\./||' | sort) \
    | comm -13 "$files" - \
    | (cd "$src" && xargs -r -d '\n' rm -f)

cd "$src"
echo "install-windows: building silt.exe and silt-desktop.exe"
windows 'cargo build --release --locked -p silt-cli; if ($LASTEXITCODE) { exit $LASTEXITCODE }
    cargo build --release --locked --manifest-path crates/silt-desktop/Cargo.toml --features tauri/custom-protocol; exit $LASTEXITCODE'

# A running program can't be overwritten on Windows, only renamed: the
# daemon and the app keep their old build until they restart.
windows '$app = Join-Path $env:LOCALAPPDATA "Silt"
    New-Item -ItemType Directory -Force $app | Out-Null
    Get-ChildItem $app -Filter *.old | Remove-Item -ErrorAction SilentlyContinue
    $built = @{ "silt.exe" = "target\release\silt.exe"; "silt-desktop.exe" = "crates\silt-desktop\target\release\silt-desktop.exe" }
    foreach ($name in $built.Keys) {
        $to = Join-Path $app $name
        if (Test-Path $to) { Move-Item $to "$to.$(Get-Date -Format yyyyMMddHHmmss).old" }
        Copy-Item $built[$name] $to
    }'

"$app/silt.exe" daemon start
echo "install-windows: installed $("$app/silt.exe" --version) to $local_app_data\\Silt"
windows 'reg query HKCU\Software\Classes\silt /ve *> $null; exit $LASTEXITCODE' \
    || echo "install-windows: silt:// links and the PATH need the desktop installer, run once: https://github.com/Urcra/silt/releases"
if windows 'if (Get-Process silt-desktop -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }'; then
    echo "install-windows: the desktop app still runs the old build; quit and reopen it"
fi
