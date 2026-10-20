//! The harness's tools on a checkout. Paths never leave it, and a file is
//! only changed when it is as the agent last read it: a collaborator's
//! edit merged in meanwhile makes the agent read it again rather than
//! overwrite it.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// What a tool returns to the model; errors are for the model too.
pub type Outcome = Result<String, String>;

const MAX_OUTPUT: usize = 30_000;

pub struct Workspace {
    root: PathBuf,
    /// The content hash of each file as the agent last saw it.
    seen: HashMap<PathBuf, String>,
}

fn hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn field<'a>(input: &'a Value, name: &str) -> Result<&'a str, String> {
    input[name].as_str().ok_or_else(|| format!("`{name}` is missing"))
}

pub(crate) fn truncate(mut text: String) -> String {
    if text.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n… (cut)");
    }
    text
}

impl Workspace {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            root: root.canonicalize()?,
            seen: HashMap::new(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The path inside the checkout that `path` names, or why it is not
    /// allowed. Symlinks may not lead out either.
    fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        let mut resolved = self.root.clone();
        for component in Path::new(path).components() {
            match component {
                Component::Normal(part) => resolved.push(part),
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => resolved = PathBuf::from("/"),
            }
        }
        let inside = |candidate: &Path| candidate.starts_with(&self.root);
        if !inside(&resolved) {
            return Err(format!("{path} is outside the checkout"));
        }
        if resolved.strip_prefix(&self.root).is_ok_and(|rest| rest.starts_with(".git")) {
            return Err("the git directory is not for editing".into());
        }
        let existing = resolved.ancestors().find(|ancestor| ancestor.exists()).unwrap_or(&self.root);
        match existing.canonicalize() {
            Ok(real) if inside(&real) => Ok(resolved),
            _ => Err(format!("{path} leads outside the checkout")),
        }
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).to_string_lossy().into_owned()
    }

    pub fn read_file(&mut self, input: &Value) -> Outcome {
        let path = self.resolve(field(input, "path")?)?;
        let text = std::fs::read_to_string(&path).map_err(|error| format!("cannot read {}: {error}", self.relative(&path)))?;
        self.seen.insert(path, hash(&text));
        Ok(truncate(text))
    }

    /// Refuses to change a file that exists but is not as last read.
    fn check_fresh(&self, path: &Path) -> Result<Option<String>, String> {
        let Ok(current) = std::fs::read_to_string(path) else {
            return Ok(None);
        };
        match self.seen.get(path) {
            Some(seen) if *seen == hash(&current) => Ok(Some(current)),
            Some(_) => Err(format!(
                "{} changed since you last read it, probably edited by someone else; read it again before changing it",
                self.relative(path)
            )),
            None => Err(format!("read {} before changing it", self.relative(path))),
        }
    }

    fn store(&mut self, path: PathBuf, text: &str) -> Outcome {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&path, text).map_err(|error| format!("cannot write {}: {error}", self.relative(&path)))?;
        let done = format!("wrote {}", self.relative(&path));
        self.seen.insert(path, hash(text));
        Ok(done)
    }

    pub fn write_file(&mut self, input: &Value) -> Outcome {
        let path = self.resolve(field(input, "path")?)?;
        let content = field(input, "content")?;
        self.check_fresh(&path)?;
        self.store(path, content)
    }

    pub fn edit_file(&mut self, input: &Value) -> Outcome {
        let path = self.resolve(field(input, "path")?)?;
        let (old, new) = (field(input, "old")?, field(input, "new")?);
        let current = self
            .check_fresh(&path)?
            .ok_or_else(|| format!("{} does not exist", self.relative(&path)))?;
        let edited = match current.matches(old).count() {
            1 => current.replacen(old, new, 1),
            0 => return Err("`old` is not in the file".into()),
            count => return Err(format!("`old` is in the file {count} times; give more context")),
        };
        self.store(path, &edited)
    }

    pub fn list_files(&self, input: &Value) -> Outcome {
        let start = self.resolve(input["path"].as_str().unwrap_or("."))?;
        let mut found = Vec::new();
        let mut stack = vec![start];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if name == ".git" || name == "node_modules" || name == "target" {
                    continue;
                }
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    stack.push(path);
                } else {
                    found.push(self.relative(&path));
                }
            }
        }
        found.sort();
        Ok(truncate(found.join("\n")))
    }
}

/// The tools as the model is told about them: name, description and the
/// JSON schema of the input.
pub fn specs() -> Vec<(&'static str, &'static str, Value)> {
    let path = json!({"type": "string", "description": "relative to the checkout"});
    vec![
        (
            "read_file",
            "Read a file of the checkout.",
            json!({"type": "object", "properties": {"path": path}, "required": ["path"]}),
        ),
        (
            "write_file",
            "Create a file, or replace one you have read, with new content.",
            json!({"type": "object", "properties": {"path": path, "content": {"type": "string"}}, "required": ["path", "content"]}),
        ),
        (
            "edit_file",
            "Replace the one occurrence of `old` in a file you have read with `new`.",
            json!({"type": "object", "properties": {"path": path, "old": {"type": "string"}, "new": {"type": "string"}}, "required": ["path", "old", "new"]}),
        ),
        (
            "list_files",
            "List the files under a directory of the checkout, by default all of them.",
            json!({"type": "object", "properties": {"path": path}}),
        ),
        (
            "run",
            "Run a shell command in the checkout, sandboxed: it can write only the checkout and /tmp, and has no network.",
            json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("src")).expect("dir");
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").expect("file");
        let workspace = Workspace::new(dir.path()).expect("workspace");
        (dir, workspace)
    }

    #[test]
    fn paths_stay_in_the_checkout() {
        let (dir, mut workspace) = workspace();
        assert!(workspace.read_file(&json!({"path": "../../etc/passwd"})).is_err());
        assert!(workspace.read_file(&json!({"path": "/etc/passwd"})).is_err());
        assert!(workspace.write_file(&json!({"path": ".git/config", "content": "x"})).is_err());
        std::os::unix::fs::symlink("/etc", dir.path().join("out")).expect("symlink");
        assert!(workspace.read_file(&json!({"path": "out/passwd"})).is_err());
        assert_eq!(workspace.read_file(&json!({"path": "./src/../src/main.rs"})), Ok("fn main() {}\n".into()));
    }

    #[test]
    fn edits_need_the_file_as_last_read() {
        let (dir, mut workspace) = workspace();
        let edit = json!({"path": "src/main.rs", "old": "fn main() {}", "new": "fn main() { run() }"});
        assert_eq!(workspace.edit_file(&edit), Err("read src/main.rs before changing it".into()));
        workspace.read_file(&json!({"path": "src/main.rs"})).expect("reads");
        std::fs::write(dir.path().join("src/main.rs"), "// a teammate\nfn main() {}\n").expect("teammate edits");
        let refused = workspace.edit_file(&edit).expect_err("stale");
        assert!(refused.contains("changed since you last read it"), "{refused}");
        workspace.read_file(&json!({"path": "src/main.rs"})).expect("reads again");
        assert_eq!(workspace.edit_file(&edit), Ok("wrote src/main.rs".into()));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/main.rs")).expect("reads"),
            "// a teammate\nfn main() { run() }\n"
        );
        assert_eq!(workspace.edit_file(&edit), Err("`old` is not in the file".into()));
    }

    #[test]
    fn new_files_need_no_read_and_are_listed() {
        let (_dir, mut workspace) = workspace();
        assert_eq!(
            workspace.write_file(&json!({"path": "tests/it.rs", "content": "#[test] fn it() {}\n"})),
            Ok("wrote tests/it.rs".into())
        );
        assert_eq!(workspace.list_files(&json!({})), Ok("src/main.rs\ntests/it.rs".into()));
    }
}
