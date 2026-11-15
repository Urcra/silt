//! Skills Silt ships for the agents the web UI runs (ADR 0071). They live
//! in the binary and are written out once per content, so an agent can
//! read and run the files they name.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

pub struct BuiltinSkill {
    pub name: &'static str,
    /// Each file by its path in the skill's folder; `SKILL.md` first.
    pub files: &'static [(&'static str, &'static str)],
}

pub const BUILTIN_SKILLS: &[BuiltinSkill] = &[
    BuiltinSkill { name: "artifact-design", files: &[("SKILL.md", include_str!("../skills/artifact-design/SKILL.md"))] },
    BuiltinSkill {
        name: "artifact-dataviz",
        files: &[
            ("SKILL.md", include_str!("../skills/artifact-dataviz/SKILL.md")),
            ("references/form.md", include_str!("../skills/artifact-dataviz/references/form.md")),
            ("references/colour.md", include_str!("../skills/artifact-dataviz/references/colour.md")),
            ("references/marks.md", include_str!("../skills/artifact-dataviz/references/marks.md")),
            ("references/interaction.md", include_str!("../skills/artifact-dataviz/references/interaction.md")),
            ("references/anti-patterns.md", include_str!("../skills/artifact-dataviz/references/anti-patterns.md")),
        ],
    },
    BuiltinSkill {
        name: "artifact-diagramming",
        files: &[("SKILL.md", include_str!("../skills/artifact-diagramming/SKILL.md"))],
    },
    BuiltinSkill { name: "design-system", files: &[("SKILL.md", include_str!("../skills/design-system/SKILL.md"))] },
];

pub fn builtin_skill(name: &str) -> Option<&'static BuiltinSkill> {
    BUILTIN_SKILLS.iter().find(|skill| skill.name == name)
}

/// The folder holding the built-in skills under `data_dir`, written there
/// first if this build's are missing. Each build's content gets its own
/// folder, so an agent never reads a half-written or older copy.
pub fn builtin_skills_root(data_dir: &Path) -> io::Result<PathBuf> {
    let mut hash = Sha256::new();
    for skill in BUILTIN_SKILLS {
        for (path, text) in skill.files {
            hash.update(skill.name.as_bytes());
            hash.update([0]);
            hash.update(path.as_bytes());
            hash.update([0]);
            hash.update(text.as_bytes());
            hash.update([0]);
        }
    }
    let digest: String = hash.finalize().iter().take(6).map(|byte| format!("{byte:02x}")).collect();
    let parent = data_dir.join("skills").join("builtin");
    let root = parent.join(&digest);
    if root.is_dir() {
        return Ok(root);
    }
    fs::create_dir_all(&parent)?;
    let staging = tempfile::tempdir_in(&parent)?;
    for skill in BUILTIN_SKILLS {
        let dir = staging.path().join(skill.name);
        fs::create_dir_all(&dir)?;
        for (path, text) in skill.files {
            let file = dir.join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(file, text)?;
        }
    }
    match fs::rename(staging.path(), &root) {
        Ok(()) => {
            let _ = staging.keep();
            Ok(root)
        }
        // Another process wrote the same content first.
        Err(_) if root.is_dir() => Ok(root),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conventions::{SkillScope, discover_skills_with};

    #[test]
    fn the_built_in_skills_are_written_once_and_found_last() {
        let data = tempfile::tempdir().expect("temp dir");
        let root = builtin_skills_root(data.path()).expect("written");
        assert_eq!(builtin_skills_root(data.path()).expect("again"), root);
        assert!(root.join("artifact-dataviz/references/colour.md").is_file());

        let checkout = tempfile::tempdir().expect("temp dir");
        let own = checkout.path().join(".silt/skills/artifact-design");
        fs::create_dir_all(&own).expect("dir");
        fs::write(own.join("SKILL.md"), "---\nname: artifact-design\ndescription: The project's own.\n---\nOurs.")
            .expect("write");
        let skills = discover_skills_with(checkout.path(), None, Some(&root));
        let design = skills.iter().find(|skill| skill.name == "artifact-design").expect("design");
        assert_eq!((design.scope, design.description.as_str()), (SkillScope::Silt, "The project's own."));
        let dataviz = skills.iter().find(|skill| skill.name == "artifact-dataviz").expect("dataviz");
        assert_eq!(dataviz.scope, SkillScope::Builtin);
        for skill in BUILTIN_SKILLS {
            assert!(skills.iter().any(|found| found.name == skill.name && found.model_invocable), "{}", skill.name);
        }
    }
}
