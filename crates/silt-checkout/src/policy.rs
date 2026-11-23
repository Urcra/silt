//! Which paths of a thread Silt may write into a checkout. Threads hold
//! other people's work, so a path is untrusted input: it must stay inside
//! the checkout and out of git's and Silt's own data (ADR 0016).

use silt_crdt::is_git_dir_name;

/// Files larger than this are kept whole, as blobs, rather than as text
/// merged character by character: a long file's text history was most of
/// a family's memory (ADR 0115). Whoever writes a delta applies it, never
/// whoever applies one, so every replica projects the same.
pub const TEXT_LIMIT: usize = 5 * 1024 * 1024;

/// Whether `path`, relative to the checkout, may be written or removed. On
/// Windows it must also be a path Windows can hold as named (ADR 0045).
pub fn is_writable(path: &str) -> bool {
    is_writable_anywhere(path) && (!cfg!(windows) || windows_can_hold(path))
}

fn is_writable_anywhere(path: &str) -> bool {
    let normal_outside_git = path.split('/').all(|component| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && !component.contains('\0')
            && !is_git_dir_name(component)
            && !is_git_dir_stream(component)
    });
    let mut components = path.split('/');
    let in_silt_data = matches!((components.next(), components.next()), (Some(first), Some(second))
        if first.eq_ignore_ascii_case(".silt") && second.eq_ignore_ascii_case("checkouts"));
    normal_outside_git && !in_silt_data && !path.starts_with('/')
}

/// The paths of a thread that a checkout here leaves unwritten: those the
/// write policy refuses, and where the file system folds case, each path
/// but the first, in path order, of those differing only in case, as they
/// would be one file (ADR 0045).
pub fn unwritten<'a>(paths: impl IntoIterator<Item = &'a str>, folds_case: bool) -> Vec<String> {
    let mut paths: Vec<&str> = paths.into_iter().collect();
    paths.sort_unstable();
    paths.dedup();
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| !is_writable(path) || (folds_case && !seen.insert(path.to_lowercase())))
        .map(str::to_owned)
        .collect()
}

/// `.git` named through an NTFS stream, which Windows opens as `.git`
/// itself; git's `core.protectNTFS` refuses it everywhere, and so does Silt.
fn is_git_dir_stream(component: &str) -> bool {
    const STREAM: &str = "::$index_allocation";
    component.len() > STREAM.len()
        && component.is_char_boundary(component.len() - STREAM.len())
        && component[component.len() - STREAM.len()..].eq_ignore_ascii_case(STREAM)
        && is_git_dir_name(&component[..component.len() - STREAM.len()])
}

/// Whether Windows can hold `path` under that name: no component with a
/// character it forbids, ending in a dot or space, which it strips, or
/// naming a device, which it opens instead of a file.
pub fn windows_can_hold(path: &str) -> bool {
    path.split('/').all(|component| {
        !component.chars().any(|char| matches!(char, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || char < ' ')
            && !component.ends_with(['.', ' '])
            && !is_device_name(component)
    })
}

/// `CON`, `NUL`, `COM1` and the rest, in any case and with any extension:
/// Windows reserves them in every directory.
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let (port, number) = upper.split_at(upper.char_indices().nth(3).map_or(upper.len(), |(index, _)| index));
            matches!(port, "COM" | "LPT")
                && matches!(number, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "\u{b9}" | "\u{b2}" | "\u{b3}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_git_data_silt_checkouts_and_escapes() {
        for path in [
            ".git/hooks/pre-commit",
            ".GIT/config",
            "vendor/lib/.git/config",
            "git~1/HEAD",
            ".silt/checkouts/x/a",
            "../outside",
            "a/../../b",
            "/etc/passwd",
            "a//b",
            "",
        ] {
            assert!(!is_writable(path), "{path}");
        }
        for path in ["src/main.rs", ".gitignore", ".github/workflows/ci.yml", ".silt/skills/x/SKILL.md", ".agents/prepare"] {
            assert!(is_writable(path), "{path}");
        }
    }

    #[test]
    fn git_dir_streams_are_refused_everywhere() {
        for path in [".git::$INDEX_ALLOCATION/hooks/pre-commit", "a/.GIT::$index_allocation", "git~1::$INDEX_ALLOCATION/config"] {
            assert!(!is_writable(path), "{path}");
        }
        assert!(is_writable_anywhere("notes/git::$INDEX_ALLOCATION"), "only a .git name counts as one");
    }

    #[test]
    fn windows_holds_only_names_it_can_open_as_files() {
        for path in [
            "aux.txt",
            "src/CON",
            "a/nul.tar.gz",
            "COM1",
            "lpt9.log",
            "com\u{b9}",
            "Con .txt",
            "a:b",
            "a\\b",
            "what?",
            "star*",
            "pipe|d",
            "quote\"d",
            "<angle>",
            "trailing.",
            "trailing ",
            "dir./file",
            "tab\there",
        ] {
            assert!(!windows_can_hold(path), "{path}");
        }
        for path in ["src/main.rs", "console.log", "auxiliary/x", "COM10", "lpt", "com/x", "nul_safe", ".env", "a b/c d.txt"] {
            assert!(windows_can_hold(path), "{path}");
        }
    }

    #[test]
    fn what_windows_cant_hold_is_written_elsewhere_only() {
        assert_eq!(is_writable("aux.txt"), !cfg!(windows));
        assert_eq!(is_writable("a:b"), !cfg!(windows));
        assert!(is_writable("src/main.rs"));
    }

    #[test]
    fn unwritten_paths_are_the_refused_and_the_later_of_a_case_clash() {
        let paths = ["src/Main.rs", "src/main.rs", ".git/config", "README.md", "aux.txt"];
        let mut expected = vec![".git/config", "src/main.rs"];
        if cfg!(windows) {
            expected.push("aux.txt");
        }
        expected.sort_unstable();
        assert_eq!(unwritten(paths, true), expected);
        assert_eq!(unwritten(paths, false), expected.iter().copied().filter(|path| *path != "src/main.rs").collect::<Vec<_>>());
    }
}
