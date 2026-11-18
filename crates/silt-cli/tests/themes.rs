//! `silt theme` on its own data dir: importing, starting, listing and
//! checking themes for Silt's UI (ADR 0098).

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn silt(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_silt"))
        .args(args)
        .env("SILT_HOME", home)
        .env_remove("SILT_DAEMON")
        .output()
        .expect("silt runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

const NIGHT: &str = include_str!("../../silt-theme/tests/fixtures/vscode-night.jsonc");
const HARBOUR: &str = include_str!("../../silt-theme/tests/fixtures/zed-family.json");

#[test]
fn an_imported_theme_is_listed_checked_and_fixed() {
    let home = tempfile::tempdir().expect("home");
    let source = home.path().join("night.jsonc");
    fs::write(&source, NIGHT).expect("source");

    let imported = silt(home.path(), &["theme", "import", source.to_str().expect("path")]);
    assert!(imported.status.success(), "{}{}", stdout(&imported), String::from_utf8_lossy(&imported.stderr));
    let report = stdout(&imported);
    assert!(report.contains("Read a VS Code theme: Fixture Night (dark)"), "{report}");
    assert!(report.contains("ignored  semanticTokenColors"), "{report}");
    let file = home.path().join("themes/fixture-night.json");
    assert!(file.is_file());

    let again = silt(home.path(), &["theme", "import", source.to_str().expect("path")]);
    assert!(!again.status.success(), "a second import doesn't replace the file without --force");

    let listed = stdout(&silt(home.path(), &["theme", "list"]));
    assert!(listed.lines().any(|line| line.starts_with("fixture-night") && line.contains("yours")), "{listed}");
    assert!(listed.lines().any(|line| line.starts_with("nous-light") && line.contains("● light theme")), "{listed}");

    let check = |extra: &[&str]| silt(home.path(), &[&["theme", "check", file.to_str().expect("path")][..], extra].concat());
    let failed = check(&[]);
    assert!(!failed.status.success(), "the fixture's comment is too faint: {}", stdout(&failed));
    assert!(stdout(&failed).contains("syntax comment on bg"), "{}", stdout(&failed));
    let fixed = check(&["--fix-contrast"]);
    assert!(fixed.status.success(), "{}", stdout(&fixed));
    assert!(stdout(&fixed).contains("nudged   fixture-night syntax comment"), "{}", stdout(&fixed));
    assert!(check(&[]).status.success(), "the fix was saved");
}

#[test]
fn a_zed_family_imports_under_a_chosen_id() {
    let home = tempfile::tempdir().expect("home");
    let source = home.path().join("harbour.json");
    fs::write(&source, HARBOUR).expect("source");
    let imported = silt(home.path(), &["theme", "import", source.to_str().expect("path"), "--to", "harbour"]);
    assert!(imported.status.success(), "{}", String::from_utf8_lossy(&imported.stderr));
    let listed = stdout(&silt(home.path(), &["theme", "list", "--json"]));
    let catalog: serde_json::Value = serde_json::from_str(&listed).expect("JSON");
    let ids: Vec<&str> = catalog["themes"].as_array().expect("themes").iter().filter_map(|theme| theme["id"].as_str()).collect();
    assert!(ids.contains(&"harbour-light") && ids.contains(&"harbour-dark"), "{ids:?}");
}

#[test]
fn a_new_theme_starts_from_another_and_a_broken_file_is_named() {
    let home = tempfile::tempdir().expect("home");
    let made = silt(home.path(), &["theme", "new", "my-dark", "--from", "nous-dark"]);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let file = fs::read_to_string(home.path().join("themes/my-dark.json")).expect("written");
    assert!(file.contains("\"name\": \"My Dark\"") && file.contains("\"appearance\": \"dark\""), "{file}");
    assert!(!silt(home.path(), &["theme", "new", "x", "--from", "no-such-theme"]).status.success());

    fs::write(home.path().join("themes/broken.json"), "{ nope").expect("broken");
    let listed = stdout(&silt(home.path(), &["theme", "list"]));
    assert!(listed.contains("unreadable: broken.json"), "{listed}");
    assert!(listed.lines().any(|line| line.starts_with("my-dark")), "{listed}");
    let path = stdout(&silt(home.path(), &["theme", "path"]));
    assert_eq!(Path::new(path.trim()), home.path().join("themes"));
}

#[test]
fn a_folder_theme_is_checked_with_its_fonts_and_a_new_one_takes_them_along() {
    let home = tempfile::tempdir().expect("home");
    let root = home.path().join("themes/harbour");
    fs::create_dir_all(root.join("fonts")).expect("fonts");
    fs::write(
        root.join("theme.json"),
        r##"{ "name": "Harbour", "themes": [{ "id": "harbour", "name": "Harbour", "appearance": "light",
            "colors": { "bg": "#ffffff", "fg-1": "#111111", "accent": "#0055cc" },
            "fonts": { "ui": ["Harbour Sans", "system-ui"], "faces": [
              { "family": "Harbour Sans", "file": "fonts/sans.woff2" },
              { "family": "Harbour Sans", "file": "fonts/not-a-font.woff2", "weight": 700 }
            ] } }] }"##,
    )
    .expect("theme");
    fs::write(root.join("fonts/sans.woff2"), b"wOF2 a font").expect("font");
    fs::write(root.join("fonts/not-a-font.woff2"), b"GIF89a").expect("not a font");

    let checked = silt(home.path(), &["theme", "check", root.to_str().expect("path")]);
    assert!(checked.status.success(), "a face left out is a warning: {}", stdout(&checked));
    assert!(stdout(&checked).contains("not-a-font.woff2 isn't a woff2 font"), "{}", stdout(&checked));

    let made = silt(home.path(), &["theme", "new", "harbour-dusk", "--from", "harbour"]);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let copy = home.path().join("themes/harbour-dusk");
    assert!(copy.join("theme.json").is_file() && copy.join("fonts/sans.woff2").is_file());
    assert!(!copy.join("fonts/not-a-font.woff2").exists(), "only the faces it ships");
}
