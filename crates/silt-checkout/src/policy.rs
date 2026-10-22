//! Which paths of a thread Silt may write into a checkout. Threads hold
//! other people's work, so a path is untrusted input: it must stay inside
//! the checkout and out of git's and Silt's own data (ADR 0016).

use silt_crdt::is_git_dir_name;

/// Whether `path`, relative to the checkout, may be written or removed.
pub fn is_writable(path: &str) -> bool {
    let normal_outside_git = path.split('/').all(|component| {
        !component.is_empty() && component != "." && component != ".." && !component.contains('\0') && !is_git_dir_name(component)
    });
    let mut components = path.split('/');
    let in_silt_data = matches!((components.next(), components.next()), (Some(first), Some(second))
        if first.eq_ignore_ascii_case(".silt") && second.eq_ignore_ascii_case("checkouts"));
    normal_outside_git && !in_silt_data && !path.starts_with('/')
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
}
