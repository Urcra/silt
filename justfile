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

# Install the CLI, the desktop app and the speech helper
install: install-cli install-desktop install-voice

# Build the web UI, which the CLI embeds
build-web:
    pnpm -C web install
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

# Install `silt-desktop` beside `silt`, with a launcher entry for the silt:// links
install-desktop: check-webkit
    cargo install --locked --path crates/silt-desktop --features tauri/custom-protocol
    install -Dm644 crates/silt-desktop/icons/128x128.png "{{data_dir}}/icons/hicolor/128x128/apps/silt.png"
    install -d "{{data_dir}}/applications"
    printf '%s\n' \
        '[Desktop Entry]' \
        'Type=Application' \
        'Name=Silt' \
        'Comment=Coding with agents, resumed anywhere' \
        'Exec={{bin_dir}}/silt-desktop %u' \
        'Icon=silt' \
        'Categories=Development;' \
        'MimeType=x-scheme-handler/silt;' \
        > "{{data_dir}}/applications/silt.desktop"
    -update-desktop-database "{{data_dir}}/applications" 2>/dev/null

# Install `silt-voice` beside `silt`, for dictation in the web UI (ADR 0059)
install-voice:
    cargo install --locked --path crates/silt-voice

# From WSL: install the CLI here, and the CLI and the desktop app on Windows
install-wsl: install-cli install-windows

# From WSL: build with Windows' cargo and install into %LOCALAPPDATA%\Silt
install-windows: build-web
    scripts/install-windows.sh

# Remove what `just install` installed
uninstall:
    -cargo uninstall silt-cli
    -cargo uninstall silt-desktop
    -cargo uninstall silt-voice
    rm -f "{{data_dir}}/applications/silt.desktop" "{{data_dir}}/icons/hicolor/128x128/apps/silt.png"

[private]
check-webkit:
    @pkg-config --exists webkit2gtk-4.1 || { echo "The desktop app needs WebKitGTK 4.1 (see docs/contributing.md)" >&2; exit 1; }
