//! A sandbox for commands an agent runs (ADR 0014), built from unprivileged
//! Linux namespaces: the command sees the whole system read-only, writes
//! only to the directories it is given and its own `/tmp`, and has no
//! network unless asked.

use std::path::PathBuf;

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
    /// Directories the command may write to.
    pub writable: Vec<PathBuf>,
    pub network: bool,
}

impl Sandbox {
    /// Puts this process into the sandbox; whatever it runs next is
    /// confined. The process must still be single-threaded, since the
    /// kernel only moves those into a new user namespace.
    pub fn enter(&self) -> Result<(), SandboxError> {
        #[cfg(target_os = "linux")]
        return linux::enter(self);
        #[cfg(not(target_os = "linux"))]
        Err(SandboxError::Unsupported)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::ffi::CString;
    use std::fs;
    use std::os::fd::OwnedFd;
    use std::path::{Path, PathBuf};

    use rustix::fs::CWD;
    use rustix::mount::{
        MountFlags, MountPropagationFlags, MoveMountFlags, OpenTreeFlags, mount, mount_change, move_mount, open_tree,
    };
    use rustix::thread::UnshareFlags;

    use super::{Sandbox, SandboxError};

    fn setup(what: impl Into<String>) -> impl FnOnce(std::io::Error) -> SandboxError {
        let what = what.into();
        move |source| SandboxError::Setup { what, source }
    }

    pub(super) fn enter(sandbox: &Sandbox) -> Result<(), SandboxError> {
        let cwd = std::env::current_dir().map_err(setup("cannot read the working directory"))?;
        let writable: Vec<PathBuf> = sandbox
            .writable
            .iter()
            .map(|dir| dir.canonicalize().map_err(setup(format!("cannot use {}", dir.display()))))
            .collect::<Result<_, _>>()?;
        let (uid, gid) = (rustix::process::getuid().as_raw(), rustix::process::getgid().as_raw());

        let mut flags = UnshareFlags::NEWUSER | UnshareFlags::NEWNS;
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
            .map_err(|error| setup("cannot make the mounts private")(error.into()))?;
        // Copies taken before everything turns read-only stay writable when
        // put back on top.
        let trees: Vec<(PathBuf, OwnedFd)> = writable
            .iter()
            .map(|dir| {
                let flags = OpenTreeFlags::OPEN_TREE_CLONE | OpenTreeFlags::OPEN_TREE_CLOEXEC | OpenTreeFlags::AT_RECURSIVE;
                open_tree(CWD, dir, flags)
                    .map(|tree| (dir.clone(), tree))
                    .map_err(|error| setup(format!("cannot copy the mount of {}", dir.display()))(error.into()))
            })
            .collect::<Result<_, _>>()?;
        read_only_everywhere()?;
        mount("tmpfs", "/tmp", "tmpfs", MountFlags::NOSUID | MountFlags::NODEV, c"mode=1777")
            .map_err(|error| setup("cannot mount a private /tmp")(error.into()))?;
        for (dir, tree) in trees {
            // Under /tmp the directory is gone with the old /tmp.
            let _ = fs::create_dir_all(&dir);
            move_mount(&tree, "", CWD, &dir, MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH)
                .map_err(|error| setup(format!("cannot make {} writable", dir.display()))(error.into()))?;
        }
        // The old working directory is the read-only mount underneath.
        let fallback = writable.first().map_or(Path::new("/"), PathBuf::as_path);
        std::env::set_current_dir(&cwd)
            .or_else(|_| std::env::set_current_dir(fallback))
            .map_err(setup("cannot enter the working directory"))?;
        Ok(())
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

    fn read_only_everywhere() -> Result<(), SandboxError> {
        let root = CString::new("/").expect("no NUL");
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
                root.as_ptr(),
                AT_RECURSIVE,
                &raw const attr,
                size_of::<MountAttr>(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(setup("cannot make the system read-only")(std::io::Error::last_os_error()))
        }
    }
}
