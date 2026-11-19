#!/bin/sh
# The probe plugin as a third party's repository, with Silt's SDK inside it
# as a build sandbox sees only the repository: `probe-repo.sh <dir>` makes
# it at v1.0.0, and `probe-repo.sh <dir> bump` tags v1.1.0, which reaches
# one more host.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
dir=$1
commit() { git -C "$dir" -c user.name=t -c user.email=t@example.com "$@"; }
if [ "${2:-}" = bump ]; then
    sed -i 's/^version = "1.0.0"/version = "1.1.0"/; s/^hosts = \["api.probe.test"\]/hosts = ["api.probe.test", "elsewhere.test"]/' "$dir/silt-plugin.toml"
    commit commit -qam "Reach elsewhere too"
    commit tag v1.1.0
    exit 0
fi
mkdir -p "$dir/sdk"
cp -r "$root/plugins/probe" "$dir/probe"
for sdk in silt-plugin-api silt-plugin silt-access; do
    cp -r "$root/crates/$sdk" "$dir/sdk/$sdk"
done
sed -i 's#../../crates/silt-plugin-api#../sdk/silt-plugin-api#' "$dir/probe/Cargo.toml"
cp "$root/plugins/Cargo.lock" "$dir/Cargo.lock"
cat > "$dir/Cargo.toml" <<'TOML'
[workspace]
resolver = "3"
members = ["probe", "sdk/*"]

[workspace.package]
version = "1.0.0"
edition = "2024"
rust-version = "1.99"
license = "AGPL-3.0-only"
publish = false

[workspace.lints.rust]
unsafe_code = "deny"

[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
strip = true
panic = "abort"
TOML
sed -n '/^pub const PROBE_MANIFEST: &str = r#"/,/^"#;/p' "$root/crates/silt-plugin-host/src/testing.rs" | sed '1d;$d' > "$dir/silt-plugin.toml"
printf '# Probe\n\nDoes what a host must allow or refuse.\n' > "$dir/README.md"
git init -q -b main "$dir"
commit add .
commit commit -qm "The probe"
commit tag v1.0.0
