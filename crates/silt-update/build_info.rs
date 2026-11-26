// What a build says of itself (ADR 0100, ADR 0129): its commit, branch,
// whether tracked files had changes, the clone it was built from and its
// channel. Shared by silt-update's build script and silt-desktop's, which is
// its own workspace. Each `SILT_BUILD_*` variable set while building wins
// over git: the release workflow names the channel, stable or nightly, and
// install-windows.sh, which builds a copy without `.git`, names the WSL clone.

fn emit_build_info() {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const VARS: [&str; 5] = ["SILT_BUILD_COMMIT", "SILT_BUILD_BRANCH", "SILT_BUILD_DIRTY", "SILT_BUILD_SOURCE", "SILT_BUILD_CHANNEL"];

    let dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let git = |args: &[&str]| -> Option<String> {
        let output = Command::new("git").arg("-C").arg(&dir).args(args).output().ok()?;
        let text = String::from_utf8(output.stdout).ok()?;
        output.status.success().then(|| text.trim().to_owned())
    };
    let given = |name: &str| std::env::var(name).ok().filter(|value| !value.trim().is_empty());
    let one_line = |value: String| (!value.contains(['\n', '\r'])).then_some(value);

    println!("cargo:rerun-if-changed=build.rs");
    for name in VARS {
        println!("cargo:rerun-if-env-changed={name}");
    }

    let toplevel = git(&["rev-parse", "--show-toplevel"]);
    // Only what a commit or a checkout moves: the index changes with every
    // `git status`, and rebuilding the daemon each time isn't worth knowing
    // of an unstaged edit, which a fresh `cargo install` sees anyway.
    if toplevel.is_some() {
        let mut watched = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
        watched.extend(git(&["symbolic-ref", "--quiet", "HEAD"]));
        for name in watched {
            if let Some(path) = git(&["rev-parse", "--git-path", &name]) {
                let path = dir.join(Path::new(&path));
                if path.exists() {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
    }

    let commit = given("SILT_BUILD_COMMIT").or_else(|| toplevel.as_ref().and_then(|_| git(&["rev-parse", "HEAD"])));
    let branch = given("SILT_BUILD_BRANCH").or_else(|| toplevel.as_ref().and_then(|_| git(&["symbolic-ref", "--quiet", "--short", "HEAD"])));
    let dirty = match given("SILT_BUILD_DIRTY") {
        Some(value) => matches!(value.trim(), "1" | "true" | "yes"),
        None => toplevel.is_some() && git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|status| !status.is_empty()),
    };
    let source = given("SILT_BUILD_SOURCE").or(toplevel).and_then(one_line);
    let dev = commit.is_some() && source.is_some();
    let channel = given("SILT_BUILD_CHANNEL").unwrap_or_else(|| if dev { "dev" } else { "stable" }.to_owned());
    // A release is built in CI's checkout, which no one updates (ADR 0129).
    let source = source.filter(|_| channel == "dev");
    let commit = commit.and_then(one_line).unwrap_or_default();
    let branch = branch.and_then(one_line).unwrap_or_default();

    let short: String = commit.chars().take(7).collect();
    let label = match (channel.as_str(), short.is_empty(), dirty) {
        (channel, true, _) => channel.to_owned(),
        (channel, false, false) => format!("{channel} {short}"),
        (channel, false, true) => format!("{channel} {short}, with changes"),
    };

    println!("cargo:rustc-env=SILT_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=SILT_BUILD_BRANCH={branch}");
    println!("cargo:rustc-env=SILT_BUILD_DIRTY={}", if dirty { "1" } else { "" });
    println!("cargo:rustc-env=SILT_BUILD_SOURCE={}", source.unwrap_or_default());
    println!("cargo:rustc-env=SILT_BUILD_CHANNEL={channel}");
    println!("cargo:rustc-env=SILT_BUILD_LABEL={label}");
}
