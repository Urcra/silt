//! A sandbox for commands an agent runs (ADR 0014, ADR 0016), built from
//! unprivileged Linux namespaces:
//! - the command sees the system read-only, and writes only to the
//!   directories it is given and its own `/tmp`;
//! - git's and Silt's data inside those directories stays read-only;
//! - places that hold sockets or credentials are hidden;
//! - it has no network unless asked, gets a clean environment, and runs in
//!   its own process namespace, so everything it starts ends with it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error(
        "this system does not let unprivileged users create namespaces ({0}); enable them (sysctl kernel.unprivileged_userns_clone=1, or allow them in AppArmor) to run commands sandboxed"
    )]
    Unavailable(std::io::Error),
    #[error("the sandbox only works on Linux")]
    Unsupported,
    #[error("{what}: {source}")]
    Setup { what: String, source: std::io::Error },
}

#[derive(Debug, Clone, Default)]
pub struct Sandbox {
    /// Directories the command may write to. Their `.git` and `.silt` stay
    /// read-only: hooks and config there run later, outside the sandbox.
    pub writable: Vec<PathBuf>,
    /// More directories or files to hide, besides [`default_hidden`].
    pub hidden: Vec<PathBuf>,
    pub network: bool,
    /// Pass the whole environment rather than [`safe_environment`].
    pub keep_env: bool,
}

/// Directories and files a sandboxed command never sees: where sockets to
/// other programs live (a socket can be connected to on a read-only
/// mount, and some lead to programs that run commands outside the sandbox),
/// and where credentials are kept.
pub fn default_hidden() -> Vec<PathBuf> {
    let mut hidden = vec![PathBuf::from("/run"), PathBuf::from("/var/run")];
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        hidden.push(PathBuf::from(runtime));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for name in [".ssh", ".gnupg", ".aws", ".docker", ".kube", ".netrc", ".config/gh", ".config/gcloud", ".azure"] {
            hidden.push(home.join(name));
        }
    }
    hidden
}

/// The variables a sandboxed command gets: what tools need to find
/// themselves, nothing that could carry a credential or a socket's address.
pub fn safe_environment() -> Vec<(OsString, OsString)> {
    const KEPT: [&str; 22] = [
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LANGUAGE", "TERM", "TZ", "COLORTERM", "CARGO_HOME",
        "RUSTUP_HOME", "GOPATH", "GOROOT", "JAVA_HOME", "VIRTUAL_ENV", "CONDA_PREFIX", "NVM_DIR", "PNPM_HOME",
        "NODE_PATH", "PYTHONPATH", "DENO_DIR",
    ];
    std::env::vars_os()
        .filter(|(name, _)| {
            name.to_str()
                .is_some_and(|name| KEPT.contains(&name) || name.starts_with("LC_"))
        })
        .collect()
}

impl Sandbox {
    /// Runs `command` in the sandbox and returns its exit code. `init` is
    /// the command line that calls [`init`] (for Silt, `silt sandbox-init`):
    /// it becomes the first process of the sandbox's process namespace and
    /// starts the command. Call this while the process is single-threaded,
    /// since the kernel only moves those into a new user namespace.
    pub fn run(&self, init: &[OsString], command: &[String]) -> Result<i32, SandboxError> {
        #[cfg(target_os = "linux")]
        return linux::run(self, init, command);
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (init, command);
            Err(SandboxError::Unsupported)
        }
    }
}

/// The sandbox's first process: mounts its own `/proc`, forbids gaining
/// privileges and becomes `command`. Returns only when it can't.
pub fn init(command: &[String]) -> SandboxError {
    #[cfg(target_os = "linux")]
    return linux::init(command);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = command;
        SandboxError::Unsupported
    }
}

/// `paths` without those inside another of them, which the outer one hides
/// already.
fn outermost(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    let mut kept: Vec<PathBuf> = Vec::new();
    for path in paths {
        if !kept.iter().any(|outer| path.starts_with(outer)) {
            kept.push(path);
        }
    }
    kept
}

/// Whether hiding `hidden` would also hide one of the `writable` directories.
fn covers_any(hidden: &Path, writable: &[PathBuf]) -> bool {
    writable.iter().any(|dir| dir.starts_with(hidden))
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::{CString, OsString};
    use std::fs;
    use std::os::fd::OwnedFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::path::{Path, PathBuf};

    use rustix::fs::CWD;
    use rustix::mount::{
        MountFlags, MountPropagationFlags, MoveMountFlags, OpenTreeFlags, mount, mount_bind, mount_change, move_mount,
        open_tree,
    };
    use rustix::thread::UnshareFlags;

    use super::{Sandbox, SandboxError, covers_any, default_hidden, outermost, safe_environment};

    fn setup(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> SandboxError {
        let what = what.into();
        move |source| SandboxError::Setup { what, source }
    }

    fn mount_error(what: impl Into<String>) -> impl FnOnce(rustix::io::Errno) -> SandboxError {
        let what = what.into();
        move |error| SandboxError::Setup {
            what,
            source: error.into(),
        }
    }

    pub(super) fn run(sandbox: &Sandbox, init: &[OsString], command: &[String]) -> Result<i32, SandboxError> {
        let cwd = std::env::current_dir().map_err(setup("cannot read the working directory"))?;
        let writable: Vec<PathBuf> = sandbox
            .writable
            .iter()
            .map(|dir| dir.canonicalize().map_err(setup(format!("cannot use {}", dir.display()))))
            .collect::<Result<_, _>>()?;
        let hidden = outermost(
            default_hidden()
                .into_iter()
                .chain(sandbox.hidden.iter().cloned())
                .filter_map(|path| path.canonicalize().ok())
                .filter(|path| !covers_any(path, &writable))
                .collect(),
        );
        let environment = (!sandbox.keep_env).then(safe_environment);
        let (uid, gid) = (rustix::process::getuid().as_raw(), rustix::process::getgid().as_raw());

        let mut flags = UnshareFlags::NEWUSER | UnshareFlags::NEWNS | UnshareFlags::NEWPID;
        if !sandbox.network {
            flags |= UnshareFlags::NEWNET;
        }
        #[expect(unsafe_code, reason = "unshare has no safe wrapper")]
        // SAFETY: the file descriptor table is not unshared, which is the
        // only way this call could invalidate descriptors in use.
        unsafe { rustix::thread::unshare_unsafe(flags) }.map_err(|error| SandboxError::Unavailable(error.into()))?;

        fs::write("/proc/self/setgroups", "deny").map_err(setup("cannot drop supplementary groups"))?;
        fs::write("/proc/self/uid_map", format!("{uid} {uid} 1")).map_err(setup("cannot map the user"))?;
        fs::write("/proc/self/gid_map", format!("{gid} {gid} 1")).map_err(setup("cannot map the group"))?;

        mount_change("/", MountPropagationFlags::PRIVATE | MountPropagationFlags::REC)
            .map_err(mount_error("cannot make the mounts private"))?;
        // Copies taken before everything turns read-only stay writable when
        // put back on top.
        let trees: Vec<(PathBuf, OwnedFd)> = writable
            .iter()
            .map(|dir| {
                let flags = OpenTreeFlags::OPEN_TREE_CLONE | OpenTreeFlags::OPEN_TREE_CLOEXEC | OpenTreeFlags::AT_RECURSIVE;
                open_tree(CWD, dir, flags)
                    .map(|tree| (dir.clone(), tree))
                    .map_err(mount_error(format!("cannot copy the mount of {}", dir.display())))
            })
            .collect::<Result<_, _>>()?;
        read_only(Path::new("/"))?;
        mount("tmpfs", "/tmp", "tmpfs", MountFlags::NOSUID | MountFlags::NODEV, c"mode=1777")
            .map_err(mount_error("cannot mount a private /tmp"))?;
        for path in &hidden {
            hide(path)?;
        }
        for (dir, tree) in trees {
            // Under /tmp the directory is gone with the old /tmp.
            let _ = fs::create_dir_all(&dir);
            move_mount(&tree, "", CWD, &dir, MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH)
                .map_err(mount_error(format!("cannot make {} writable", dir.display())))?;
            for name in [".git", ".silt"] {
                let inner = dir.join(name);
                if fs::symlink_metadata(&inner).is_ok() {
                    mount_bind(&inner, &inner).map_err(mount_error(format!("cannot protect {}", inner.display())))?;
                    read_only(&inner)?;
                }
            }
        }
        // The old working directory is the read-only mount underneath.
        let fallback = writable.first().map_or(Path::new("/"), PathBuf::as_path);
        std::env::set_current_dir(&cwd)
            .or_else(|_| std::env::set_current_dir(fallback))
            .map_err(setup("cannot enter the working directory"))?;

        let (program, args) = init.split_first().ok_or_else(|| setup("no init command")(std::io::ErrorKind::InvalidInput.into()))?;
        let mut child = std::process::Command::new(program);
        child.args(args).arg("--").args(command);
        if let Some(environment) = environment {
            child.env_clear().envs(environment);
        }
        let status = child
            .status()
            .map_err(setup(format!("cannot start {}", program.to_string_lossy())))?;
        Ok(status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)))
    }

    pub(super) fn init(command: &[String]) -> SandboxError {
        // The sandbox ends when the process that set it up does, wherever
        // the command's own children got to.
        let _ = rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL));
        // Its own /proc shows only the sandbox's processes. Some kernels
        // refuse a new one while parts of the old are covered; the process
        // namespace still keeps signals inside then.
        let _ = mount(
            "proc",
            "/proc",
            "proc",
            MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC,
            c"",
        );
        if let Err(error) = rustix::thread::set_no_new_privs(true) {
            return mount_error("cannot forbid gaining privileges")(error);
        }
        let Some((program, args)) = command.split_first() else {
            return setup("no command to run")(std::io::ErrorKind::InvalidInput.into());
        };
        let error = std::process::Command::new(program).args(args).exec();
        setup(format!("cannot run {program}"))(error)
    }

    /// Covers `path` with something empty and read-only. A path that is
    /// gone already, such as one under the old `/tmp`, is hidden.
    fn hide(path: &Path) -> Result<(), SandboxError> {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(setup(format!("cannot hide {}", path.display()))(error)),
        };
        let hidden = if metadata.is_dir() {
            mount(
                "tmpfs",
                path,
                "tmpfs",
                MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC,
                c"mode=0555",
            )
        } else {
            mount_bind("/dev/null", path)
        };
        hidden.map_err(mount_error(format!("cannot hide {}", path.display())))
    }

    /// `struct mount_attr` from `linux/mount.h`.
    #[repr(C)]
    struct MountAttr {
        attr_set: u64,
        attr_clr: u64,
        propagation: u64,
        userns_fd: u64,
    }

    const MOUNT_ATTR_RDONLY: u64 = 0x1;
    const AT_RECURSIVE: libc::c_uint = 0x8000;

    /// Makes the mount at `path`, and every mount under it, read-only.
    fn read_only(path: &Path) -> Result<(), SandboxError> {
        let target = CString::new(path.as_os_str().as_bytes()).map_err(|error| setup("bad path")(error.into()))?;
        let attr = MountAttr {
            attr_set: MOUNT_ATTR_RDONLY,
            attr_clr: 0,
            propagation: 0,
            userns_fd: 0,
        };
        #[expect(unsafe_code, reason = "mount_setattr has no safe wrapper")]
        // SAFETY: the path is NUL-terminated and `attr` is a valid
        // `mount_attr` whose size is passed along.
        let result = unsafe {
            libc::syscall(
                libc::SYS_mount_setattr,
                libc::AT_FDCWD,
                target.as_ptr(),
                AT_RECURSIVE,
                &raw const attr,
                size_of::<MountAttr>(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(setup(format!("cannot make {} read-only", path.display()))(std::io::Error::last_os_error()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_outermost_hidden_places_are_covered() {
        let kept = outermost(vec![PathBuf::from("/run/user/1000"), PathBuf::from("/run"), PathBuf::from("/home/a/.ssh")]);
        assert_eq!(kept, vec![PathBuf::from("/home/a/.ssh"), PathBuf::from("/run")]);
    }

    #[test]
    fn hidden_places_never_cover_writable_ones() {
        let writable = vec![PathBuf::from("/run/media/usb/repo")];
        assert!(covers_any(Path::new("/run"), &writable));
        assert!(!covers_any(Path::new("/var/run"), &writable));
    }

    #[test]
    fn the_environment_keeps_tools_and_drops_credentials() {
        let names: Vec<String> = safe_environment()
            .into_iter()
            .filter_map(|(name, _)| name.into_string().ok())
            .collect();
        assert!(names.iter().all(|name| !name.contains("KEY") && !name.contains("TOKEN") && name != "SSH_AUTH_SOCK"));
    }
}
