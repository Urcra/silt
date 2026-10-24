//! Skills and rules files a repository gives its agents. Silt finds them
//! the same way for every agent, so each agent gets the ones it doesn't
//! read natively from Silt instead.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Skill roots, highest precedence first: Silt's own, the cross-agent
/// convention, then the user's personal skills.
pub fn skill_roots(checkout: &Path, home: Option<&Path>) -> Vec<(SkillScope, PathBuf)> {
    let mut roots =
        vec![(SkillScope::Silt, checkout.join(".silt/skills")), (SkillScope::Project, checkout.join(".agents/skills"))];
    if let Some(home) = home {
        roots.push((SkillScope::Personal, home.join(".agents/skills")));
    }
    roots
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkillScope {
    Silt,
    Project,
    Personal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// Offered to the user as a slash command.
    pub user_invocable: bool,
    /// The agent may load it by itself.
    pub model_invocable: bool,
    pub metadata: BTreeMap<String, String>,
    /// The directory holding `SKILL.md` and any files it refers to.
    pub dir: PathBuf,
    pub scope: SkillScope,
}

impl Skill {
    pub fn body(&self) -> std::io::Result<String> {
        let text = fs::read_to_string(self.dir.join("SKILL.md"))?;
        Ok(split_frontmatter(&text).1.trim_start().to_owned())
    }
}

/// Every skill under the roots, nested category folders included. A name
/// found in an earlier root hides the same name in later ones.
pub fn discover_skills(checkout: &Path, home: Option<&Path>) -> Vec<Skill> {
    let mut skills: BTreeMap<String, Skill> = BTreeMap::new();
    for (scope, root) in skill_roots(checkout, home) {
        let mut found = Vec::new();
        find_skill_files(&root, 0, &mut found);
        found.sort();
        for file in found {
            let Some(skill) = read_skill(&file, scope) else {
                continue;
            };
            skills.entry(skill.name.clone()).or_insert(skill);
        }
    }
    skills.into_values().collect()
}

const MAX_DEPTH: usize = 6;

fn find_skill_files(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let skill = dir.join("SKILL.md");
    if skill.is_file() {
        found.push(skill);
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir() || kind.is_symlink()) && entry.path().is_dir() {
            find_skill_files(&entry.path(), depth + 1, found);
        }
    }
}

fn read_skill(file: &Path, scope: SkillScope) -> Option<Skill> {
    let text = fs::read_to_string(file).ok()?;
    let dir = file.parent()?.to_owned();
    let fields = parse_frontmatter(split_frontmatter(&text).0);
    let flag = |key: &str, default: bool| match fields.get(key).map(|value| value.as_str()) {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    };
    let name = fields
        .get("name")
        .cloned()
        .filter(|name| !name.is_empty())
        .or_else(|| dir.file_name().map(|name| name.to_string_lossy().into_owned()))?;
    let metadata = fields
        .iter()
        .filter_map(|(key, value)| key.strip_prefix("metadata.").map(|key| (key.to_owned(), value.clone())))
        .collect();
    Some(Skill {
        description: fields.get("description").cloned().unwrap_or_default(),
        user_invocable: flag("user-invocable", true),
        model_invocable: !flag("disable-model-invocation", false),
        metadata,
        name,
        dir,
        scope,
    })
}

/// The frontmatter between leading `---` lines, and the rest.
fn split_frontmatter(text: &str) -> (&str, &str) {
    let Some(rest) = text.strip_prefix("---").and_then(|rest| rest.strip_prefix('\n').or_else(|| rest.strip_prefix("\r\n")))
    else {
        return ("", text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return (&rest[..offset], &rest[offset + line.len()..]);
        }
        offset += line.len();
    }
    ("", text)
}

/// The subset of YAML that skill frontmatter uses: `key: value` with plain
/// or quoted values, `|` and `>` block scalars, and one level of nested
/// maps, flattened to `parent.key`.
fn parse_frontmatter(text: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    let mut parent: Option<String> = None;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let key = key.trim().to_owned();
        let value = value.trim();
        let full_key = match (&parent, indented) {
            (Some(parent), true) => format!("{parent}.{key}"),
            _ => key.clone(),
        };
        if !indented {
            parent = None;
        }
        if value.is_empty() {
            if !indented {
                parent = Some(key);
            }
            continue;
        }
        let value = if let Some(style) = value.strip_prefix(['|', '>']).map(|_| value.as_bytes()[0]) {
            let mut block = Vec::new();
            while index < lines.len() && (lines[index].trim().is_empty() || lines[index].starts_with([' ', '\t'])) {
                block.push(lines[index].trim());
                index += 1;
            }
            let joined = if style == b'|' { block.join("\n") } else { block.join(" ") };
            joined.trim().to_owned()
        } else {
            unquote(value)
        };
        fields.insert(full_key, value);
    }
    fields
}

fn unquote(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some((inner, _)) = value.strip_prefix(quote).and_then(|rest| rest.split_once(quote)) {
            return inner.to_owned();
        }
    }
    value.split(" #").next().unwrap_or(value).trim().to_owned()
}

/// Names agents use for a project's rules file, in the order the first
/// non-empty one is picked at the repository root.
pub const ROOT_RULES: [&str; 9] = [
    "AGENT.md",
    "AGENTS.md",
    "CLAUDE.md",
    "GEMINI.md",
    ".rules",
    ".cursorrules",
    ".windsurfrules",
    ".clinerules",
    ".github/copilot-instructions.md",
];

/// Names that apply in subdirectories, from the outermost inward.
const NESTED_RULES: [&str; 2] = ["AGENT.md", "AGENTS.md"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesFile {
    pub path: PathBuf,
    /// The file's name relative to its directory, such as `AGENTS.md`.
    pub name: String,
    pub text: String,
}

/// The rules that apply to an agent working in `cwd` inside the checkout:
/// the user's personal rules, the root's rules file, then nested rules
/// files on the way down to `cwd`.
pub fn discover_rules(checkout: &Path, cwd: &Path, home: Option<&Path>) -> Vec<RulesFile> {
    let mut rules = Vec::new();
    if let Some(home) = home
        && let Some(file) = read_rules(&home.join(".agents"), "AGENTS.md")
    {
        rules.push(file);
    }
    if let Some(file) = ROOT_RULES.iter().find_map(|name| read_rules(checkout, name)) {
        rules.push(file);
    }
    let Ok(relative) = cwd.strip_prefix(checkout) else {
        return rules;
    };
    let mut dir = checkout.to_owned();
    for component in relative.components() {
        dir.push(component);
        if let Some(file) = NESTED_RULES.iter().find_map(|name| read_rules(&dir, name)) {
            rules.push(file);
        }
    }
    rules
}

fn read_rules(dir: &Path, name: &str) -> Option<RulesFile> {
    let path = dir.join(name);
    let text = fs::read_to_string(&path).ok()?;
    (!text.trim().is_empty()).then(|| RulesFile { path, name: name.to_owned(), text })
}

/// What an agent CLI reads by itself, so Silt only adds the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeConventions {
    /// Rules file names it loads from the directories it works in.
    pub rules: &'static [&'static str],
    /// Skill roots it loads from the checkout, relative to its root.
    pub skill_dirs: &'static [&'static str],
    /// Whether it loads `~/.agents/skills` itself.
    pub personal_skills: bool,
    /// Whether Silt hands it the other skills natively at launch.
    pub given_skills: bool,
}

/// The instructions to give an agent at session start: the rules files it
/// won't read itself, and the skills it can load through Silt.
pub fn session_instructions(rules: &[RulesFile], skills: &[Skill], native: NativeConventions, checkout: &Path) -> Option<String> {
    let mut out = String::new();
    let missing: Vec<&RulesFile> = rules.iter().filter(|file| !native.rules.contains(&file.name.as_str())).collect();
    for file in missing {
        // As the repository names it, `/`-separated on Windows too.
        let shown = file.path.strip_prefix(checkout).map_or_else(
            |_| file.path.display().to_string(),
            |inside| inside.components().map(|part| part.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        );
        out.push_str(&format!("# Project rules from {shown}\n\n{}\n\n", file.text.trim()));
    }
    let offered: Vec<&Skill> =
        skills.iter().filter(|skill| skill.model_invocable && !natively_loaded(skill, native, checkout)).collect();
    if !offered.is_empty() {
        out.push_str("# Skills\n\nThese skills are available. When a task matches one, load it with the silt_use_skill tool and follow it.\n\n");
        for skill in offered {
            out.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
    }
    let out = out.trim().to_owned();
    (!out.is_empty()).then_some(out)
}

/// Skills the agent doesn't find by itself, to hand it at launch when it
/// takes skills that way.
pub fn skills_to_give<'a>(skills: &'a [Skill], native: NativeConventions, checkout: &Path) -> Vec<&'a Skill> {
    if !native.given_skills {
        return Vec::new();
    }
    skills.iter().filter(|skill| !found_natively(skill, native, checkout)).collect()
}

fn found_natively(skill: &Skill, native: NativeConventions, checkout: &Path) -> bool {
    (native.personal_skills && skill.scope == SkillScope::Personal)
        || native.skill_dirs.iter().any(|dir| skill.dir.starts_with(checkout.join(dir)))
}

fn natively_loaded(skill: &Skill, native: NativeConventions, checkout: &Path) -> bool {
    native.given_skills || found_natively(skill, native, checkout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, text).expect("write");
    }

    #[test]
    fn frontmatter_parses_the_yaml_skills_use() {
        let text = "name: land\ndescription: >\n  Lands the branch\n  after CI.\nuser-invocable: false\nmetadata:\n  silt-action: land\n  owner: 'infra' # team\nnotes: |\n  one\n  two\n";
        let fields = parse_frontmatter(text);
        assert_eq!(fields["name"], "land");
        assert_eq!(fields["description"], "Lands the branch after CI.");
        assert_eq!(fields["user-invocable"], "false");
        assert_eq!(fields["metadata.silt-action"], "land");
        assert_eq!(fields["metadata.owner"], "infra");
        assert_eq!(fields["notes"], "one\ntwo");
    }

    #[test]
    fn skills_are_found_by_precedence_and_in_nested_folders() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let home = tempfile::tempdir().expect("temp dir");
        let skill = |name: &str, description: &str| {
            format!("---\nname: {name}\ndescription: {description}\n---\n\nDo the {name} thing.\n")
        };
        write(&checkout.path().join(".agents/skills/review/SKILL.md"), &skill("review", "project review"));
        write(&checkout.path().join(".silt/skills/review/SKILL.md"), &skill("review", "silt review"));
        write(&checkout.path().join(".agents/skills/release/land/SKILL.md"), &skill("land", "land it"));
        write(&home.path().join(".agents/skills/notes/SKILL.md"), "---\ndisable-model-invocation: true\n---\nTake notes.\n");
        write(&home.path().join(".agents/skills/land/SKILL.md"), &skill("land", "personal land"));

        let skills = discover_skills(checkout.path(), Some(home.path()));
        let summary: Vec<(&str, &str, SkillScope, bool)> = skills
            .iter()
            .map(|skill| (skill.name.as_str(), skill.description.as_str(), skill.scope, skill.model_invocable))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("land", "land it", SkillScope::Project, true),
                ("notes", "", SkillScope::Personal, false),
                ("review", "silt review", SkillScope::Silt, true),
            ]
        );
        assert_eq!(skills[2].body().expect("body"), "Do the review thing.\n");
    }

    #[test]
    fn rules_come_from_the_root_and_nested_folders() {
        let checkout = tempfile::tempdir().expect("temp dir");
        let root = checkout.path();
        write(&root.join("AGENT.md"), "   \n");
        write(&root.join("AGENTS.md"), "Use tabs.\n");
        write(&root.join("CLAUDE.md"), "Claude only.\n");
        write(&root.join("crates/AGENTS.md"), "Crates rules.\n");
        write(&root.join("crates/core/src/x.rs"), "");
        let rules = discover_rules(root, &root.join("crates/core/src"), None);
        let names: Vec<String> =
            rules.iter().map(|file| file.path.strip_prefix(root).expect("inside").to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, vec!["AGENTS.md", "crates/AGENTS.md"]);

        let claude = NativeConventions {
            rules: &["CLAUDE.md"],
            skill_dirs: &[".claude/skills"],
            personal_skills: false,
            given_skills: true,
        };
        let instructions = session_instructions(&rules, &[], claude, root).expect("some");
        assert!(instructions.contains("# Project rules from AGENTS.md\n\nUse tabs."));
        assert!(instructions.contains("from crates/AGENTS.md"));
        let codex = NativeConventions {
            rules: &["AGENTS.md"],
            skill_dirs: &[".agents/skills"],
            personal_skills: true,
            given_skills: false,
        };
        assert_eq!(session_instructions(&rules, &[], codex, root), None);
    }
}
