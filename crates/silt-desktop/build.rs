// The same build info as `silt`'s, so the page can tell whether an update
// changed this app (ADR 0100).
include!("../silt-update/build_info.rs");

fn main() {
    emit_build_info();
    tauri_build::build();
}
