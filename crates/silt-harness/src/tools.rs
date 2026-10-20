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
    /// The content hash of each file as the agent last saw it, by path in
    /// the checkout.
    seen: HashMap<String, String>,
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

/// File access confined to one directory. Every step is taken from a
/// descriptor of the checkout without following symlinks, so neither a
/// link placed by the repository or a command nor one swapped in between
/// a check and a write can lead outside it (ADR 0016).
mod beneath {
    use std::fs::File;
    use std::io::{self, Read as _, Write as _};
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use rustix::fs::{AtFlags, FileType, Mode, OFlags, fchmod, mkdirat, openat, renameat, statat};

    fn open_dir(root: &Path, dirs: &[&str], create: bool) -> io::Result<OwnedFd> {
        let flags = OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let mut dir = openat(rustix::fs::CWD, root, flags, Mode::empty())?;
        for name in dirs {
            dir = match openat(&dir, *name, flags, Mode::empty()) {
                Ok(next) => next,
                Err(rustix::io::Errno::NOENT) if create => {
                    mkdirat(&dir, *name, Mode::from_raw_mode(0o755))?;
                    openat(&dir, *name, flags, Mode::empty())?
                }
                Err(error) => return Err(error.into()),
            };
        }
        Ok(dir)
    }

    pub fn read(root: &Path, path: &[&str]) -> io::Result<String> {
        let (name, dirs) = path.split_last().ok_or(io::ErrorKind::InvalidInput)?;
        let dir = open_dir(root, dirs, false)?;
        let fd = openat(&dir, *name, OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())?;
        let mut text = String::new();
        File::from(fd).read_to_string(&mut text)?;
        Ok(text)
    }

    /// Replaces the file with `text`: written beside it, then renamed over
    /// it, keeping its permissions.
    pub fn write(root: &Path, path: &[&str], text: &str) -> io::Result<()> {
        let (name, dirs) = path.split_last().ok_or(io::ErrorKind::InvalidInput)?;
        let dir = open_dir(root, dirs, true)?;
        let mode = match statat(&dir, *name, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) if FileType::from_raw_mode(stat.st_mode) == FileType::RegularFile => Mode::from_raw_mode(stat.st_mode & 0o7777),
            Ok(_) => return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a regular file")),
            Err(rustix::io::Errno::NOENT) => Mode::from_raw_mode(0o644),
            Err(error) => return Err(error.into()),
        };
        let temporary = format!(".{name}.silt-tmp-{}", std::process::id());
        let flags = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let fd = openat(&dir, temporary.as_str(), flags, mode)?;
        fchmod(&fd, mode)?;
        File::from(fd).write_all(text.as_bytes())?;
        renameat(&dir, temporary.as_str(), &dir, *name)?;
        Ok(())
    }

    /// Whether `path` names a directory reached without symlinks.
    pub fn is_dir(root: &Path, path: &[&str]) -> bool {
        open_dir(root, path, false).is_ok()
    }
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

    /// `path` as a path in the checkout, or why it is not allowed: outside
    /// the checkout, or git's or Silt's own data.
    fn resolve(&self, path: &str) -> Result<String, String> {
        let given = Path::new(path);
        let relative = if given.is_absolute() {
            given.strip_prefix(&self.root).map_err(|_| format!("{path} is outside the checkout"))?
        } else {
            given
        };
        let mut parts: Vec<String> = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                Component::ParentDir => {
                    parts.pop().ok_or_else(|| format!("{path} is outside the checkout"))?;
                }
                Component::CurDir => {}
                Component::RootDir | Component::Prefix(_) => return Err(format!("{path} is outside the checkout")),
            }
        }
        let joined = parts.join("/");
        if !joined.is_empty() && !silt_checkout::is_writable(&joined) {
            return Err("git's and Silt's own data is not for editing".into());
        }
        Ok(joined)
    }

    fn parts(path: &str) -> Vec<&str> {
        path.split('/').filter(|part| !part.is_empty()).collect()
    }

    pub(crate) fn read_text(&self, path: &str) -> std::io::Result<String> {
        beneath::read(&self.root, &Self::parts(path))
    }

    pub fn read_file(&mut self, input: &Value) -> Outcome {
        let path = self.resolve(field(input, "path")?)?;
        let text = self.read_text(&path).map_err(|error| format!("cannot read {path}: {error}"))?;
        self.seen.insert(path, hash(&text));
        Ok(truncate(text))
    }

    /// Refuses to change a file that exists but is not as last read. Only a
    /// file that isn't there counts as new.
    fn check_fresh(&self, path: &str) -> Result<Option<String>, String> {
        let current = match self.read_text(path) {
            Ok(current) => current,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("cannot read {path}: {error}")),
        };
        match self.seen.get(path) {
            Some(seen) if *seen == hash(&current) => Ok(Some(current)),
            Some(_) => Err(format!(
                "{path} changed since you last read it, probably edited by someone else; read it again before changing it"
            )),
            None => Err(format!("read {path} before changing it")),
        }
    }

    fn store(&mut self, path: String, text: &str) -> Outcome {
        beneath::write(&self.root, &Self::parts(&path), text).map_err(|error| format!("cannot write {path}: {error}"))?;
        let done = format!("wrote {path}");
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
        let current = self.check_fresh(&path)?.ok_or_else(|| format!("{path} does not exist"))?;
        let edited = match current.matches(old).count() {
            1 => current.replacen(old, new, 1),
            0 => return Err("`old` is not in the file".into()),
            count => return Err(format!("`old` is in the file {count} times; give more context")),
        };
        self.store(path, &edited)
    }

    pub fn list_files(&self, input: &Value) -> Outcome {
        let start = self.resolve(input["path"].as_str().unwrap_or("."))?;
        if !beneath::is_dir(&self.root, &Self::parts(&start)) {
            return Err(format!("{start} is not a directory of the checkout"));
        }
        let mut found = Vec::new();
        let mut stack = vec![self.root.join(&start)];
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
                // `file_type` doesn't follow symlinks, so links are listed,
                // never walked into.
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    stack.push(path);
                } else {
                    found.push(path.strip_prefix(&self.root).unwrap_or(&path).to_string_lossy().into_owned());
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
        let outside = tempfile::tempdir().expect("tempdir");
        let target = outside.path().join("planted");
        std::os::unix::fs::symlink(&target, dir.path().join("dangling")).expect("symlink");
        assert!(workspace.write_file(&json!({"path": "dangling", "content": "x"})).is_err());
        assert!(!target.exists(), "a dangling link must not create its target");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("linkdir")).expect("symlink");
        assert!(workspace.write_file(&json!({"path": "linkdir/new", "content": "x"})).is_err());
        assert!(!outside.path().join("new").exists());
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
