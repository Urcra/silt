//! The docs site's command-line reference is what `silt reference` prints.
//! `UPDATE_CLI_REFERENCE=1 cargo test -p silt-cli --test reference`
//! writes it again after the commands change.

use std::path::Path;
use std::process::Command;

#[test]
fn the_docs_hold_the_command_line_reference_as_it_is() {
    let output = Command::new(env!("CARGO_BIN_EXE_silt")).arg("reference").output().expect("silt runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let printed = String::from_utf8(output.stdout).expect("UTF-8");
    let kept = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/cli.md");
    if std::env::var_os("UPDATE_CLI_REFERENCE").is_some() {
        std::fs::create_dir_all(kept.parent().expect("a parent")).expect("mkdir");
        std::fs::write(&kept, &printed).expect("write");
    }
    let on_disk = std::fs::read_to_string(&kept).unwrap_or_default();
    assert!(
        on_disk == printed,
        "docs/reference/cli.md is out of date; run UPDATE_CLI_REFERENCE=1 cargo test -p silt-cli --test reference"
    );
}
