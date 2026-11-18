# Building and installing Silt: the `silt` CLI and the desktop app.
# Prerequisites are in docs/contributing.md.

bin_dir := env("CARGO_INSTALL_ROOT", env("CARGO_HOME", home_directory() / ".cargo")) / "bin"
data_dir := env("XDG_DATA_HOME", home_directory() / ".local/share")

# The desktop app is its own workspace (ADR 0041). `custom-protocol` is what
# `tauri build` turns on: the app's own pages are embedded, not a dev server's.
desktop := "--manifest-path crates/silt-desktop/Cargo.toml --features tauri/custom-protocol"

[private]
default:
    @just --list

# Build the CLI and the desktop app
build: build-cli build-desktop

# Install the CLI, the desktop app and, where it builds, the speech helper
install: install-cli install-desktop try-install-voice

# Build the web UI, which the CLI embeds
build-web: check-pnpm
    pnpm -C web install --frozen-lockfile
    pnpm -C web build

# Build target/release/silt
build-cli: build-web
    cargo build --release --locked -p silt-cli

# Build crates/silt-desktop/target/release/silt-desktop
build-desktop: check-webkit
    cargo build --release --locked {{desktop}}

# Install `silt` into ~/.cargo/bin, and restart its daemon on the new build
install-cli: build-web
    cargo install --locked --path crates/silt-cli
    "{{bin_dir}}/silt" log > /dev/null

# Install `silt-desktop` beside `silt`, with a launcher entry for the silt:// links.
# Wayland matches a window to its entry by file name, and the window's app id is
# the binary's name, so the entry is `silt-desktop.desktop`.
install-desktop: check-webkit
    cargo install --locked --path crates/silt-desktop --features tauri/custom-protocol
    install -Dm644 crates/silt-desktop/icons/32x32.png "{{data_dir}}/icons/hicolor/32x32/apps/silt.png"
    install -Dm644 crates/silt-desktop/icons/128x128.png "{{data_dir}}/icons/hicolor/128x128/apps/silt.png"
    install -Dm644 crates/silt-desktop/icons/128x128@2x.png "{{data_dir}}/icons/hicolor/256x256/apps/silt.png"
    install -Dm644 crates/silt-desktop/icons/icon.png "{{data_dir}}/icons/hicolor/512x512/apps/silt.png"
    install -d "{{data_dir}}/applications"
    rm -f "{{data_dir}}/applications/silt.desktop"
    printf '%s\n' \
        '[Desktop Entry]' \
        'Type=Application' \
        'Name=Silt' \
        'Comment=Coding with agents, resumed anywhere' \
        'Exec={{bin_dir}}/silt-desktop %u' \
        'Icon=silt' \
        'Categories=Development;' \
        'MimeType=x-scheme-handler/silt;' \
        'StartupWMClass=silt-desktop' \
        > "{{data_dir}}/applications/silt-desktop.desktop"
    -update-desktop-database "{{data_dir}}/applications" 2>/dev/null

# Install `silt-voice` beside `silt`, for dictation in the web UI (ADR 0061)
install-voice:
    cargo install --locked --path crates/silt-voice

# Dictation is optional, and its build downloads the speech engine, which
# fails offline and on platforms it has no build for.
[private]
try-install-voice:
    @{{just_executable()}} install-voice || echo "Skipped the speech helper, so the web UI has no dictation. \`just install-voice\` tries again." >&2

# From WSL: install the CLI and the speech helper here, and those and the desktop app on Windows
install-wsl: install-cli try-install-voice install-windows

# From WSL: build with Windows' cargo and install the CLI, the desktop app and the speech helper into %LOCALAPPDATA%\Silt
install-windows: build-web
    scripts/install-windows.sh

# What updating from the app runs (ADR 0100): `install`, with the desktop app
# only where WebKitGTK is; in WSL, `install-wsl`, building for Windows only
# where Silt is installed there
update:
    #!/usr/bin/env sh
    set -eu
    just="{{just_executable()}}"
    if [ -n "${WSL_DISTRO_NAME:-}" ]; then
        "$just" install-cli
        "$just" try-install-voice
        if command -v powershell.exe > /dev/null && powershell.exe -NoProfile -NonInteractive -Command 'Test-Path (Join-Path $env:LOCALAPPDATA "Silt\silt.exe")' | grep -q True; then
            "$just" install-windows
        fi
        exit 0
    fi
    "$just" install-cli
    if pkg-config --exists webkit2gtk-4.1 2> /dev/null; then
        "$just" install-desktop
    else
        echo "Skipped the desktop app, which needs WebKitGTK 4.1 (see docs/contributing.md)" >&2
    fi
    "$just" try-install-voice

# Remove what `just install` installed
uninstall:
    -cargo uninstall silt-cli
    -cargo uninstall silt-desktop
    -cargo uninstall silt-voice
    rm -f "{{data_dir}}/applications/silt-desktop.desktop" "{{data_dir}}/applications/silt.desktop"
    for size in 32x32 128x128 256x256 512x512; do rm -f "{{data_dir}}/icons/hicolor/$size/apps/silt.png"; done

[private]
check-pnpm:
    @command -v node > /dev/null || { echo "Building the web UI needs Node.js (see docs/contributing.md)" >&2; exit 1; }
    @command -v pnpm > /dev/null || { echo "Building the web UI needs pnpm: \`corepack enable pnpm\`, or \`npm install -g pnpm\` (see docs/contributing.md)" >&2; exit 1; }

[private]
check-webkit:
    @pkg-config --exists webkit2gtk-4.1 || { echo "The desktop app needs WebKitGTK 4.1 (see docs/contributing.md)" >&2; exit 1; }
