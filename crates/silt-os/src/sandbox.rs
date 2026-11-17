//! Silt's sandbox on Linux (ADR 0086): bubblewrap for what a run sees and
//! which processes it can touch, pasta for full internet, routes it can't
//! change for what full internet never reaches, and a seccomp filter for
//! the few calls no agent needs.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read as _};
use std::os::fd::AsFd as _;
use std::path::{Path, PathBuf};

use rustix::fd::AsRawFd as _;
use silt_workspace::{Cidr, MountKind, Network, SandboxPlan};

/// Where the sandbox's DNS goes in full internet: an address pasta answers
/// itself, outside every range the sandbox blocks.
pub const DNS_ADDRESS: &str = "198.51.100.53";

/// The programs the sandbox is made with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub bwrap: PathBuf,
    /// Needed for full internet only.
    pub pasta: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("Silt's sandbox needs bubblewrap: Silt ships it beside `silt`, or install it (`bwrap`)")]
    NoBubblewrap,
    #[error("full internet in Silt's sandbox needs pasta: Silt ships it beside `silt`, or install it (`passt`)")]
    NoPasta,
    #[error(
        "this system doesn't let bubblewrap make a sandbox: {0}. On Ubuntu 24.04 and later, bubblewrap needs its AppArmor profile; `silt access doctor` says how"
    )]
    Refused(String),
    #[error("could not set up the sandbox's network: {0}")]
    Routes(io::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A program shipped beside `silt`, named by its variable, else on the PATH.
fn shipped(name: &str, variable: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(variable).filter(|path| !path.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(format!("silt-{name}"))));
    beside.filter(|path| path.is_file()).or_else(|| which::which(name).ok())
}

/// Finds bubblewrap, and pasta when `network` needs it.
pub fn tools(network: Network) -> Result<Tools, SandboxError> {
    let bwrap = shipped("bwrap", "SILT_BWRAP").ok_or(SandboxError::NoBubblewrap)?;
    let pasta = shipped("pasta", "SILT_PASTA");
    if network == Network::Full && pasta.is_none() {
        return Err(SandboxError::NoPasta);
    }
    Ok(Tools { bwrap, pasta })
}

/// Whether bubblewrap can make a sandbox here, with what it said when not.
pub fn check(tools: &Tools) -> Result<(), SandboxError> {
    let output = std::process::Command::new(&tools.bwrap)
        .args(["--unshare-all", "--die-with-parent", "--ro-bind", "/", "/", "--", "true"])
        .stdin(std::process::Stdio::null())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(SandboxError::Refused(if said.is_empty() { format!("bwrap exited with {}", output.status) } else { said }))
}

/// pasta's arguments to run `then` in a namespace of its own with full
/// internet: no ports forwarded either way, the gateway address not mapped
/// to this machine, and DNS answered at [`DNS_ADDRESS`].
pub fn pasta_args(then: &[OsString]) -> Vec<OsString> {
    // pasta otherwise drops to `nobody` when started as root, which can't
    // make the namespace.
    let runas = format!("{}:{}", rustix::process::getuid().as_raw(), rustix::process::getgid().as_raw());
    let mut args: Vec<OsString> = vec!["--runas".into(), runas.into()];
    args.extend(
        [
            "--config-net",
            "--quiet",
            "--no-map-gw",
            "--dns-forward",
            DNS_ADDRESS,
            "--tcp-ports",
            "none",
            "--udp-ports",
            "none",
            "--tcp-ns",
            "none",
            "--udp-ns",
            "none",
            "--",
        ]
        .into_iter()
        .map(OsString::from),
    );
    args.extend(then.iter().cloned());
    args
}

/// bubblewrap's arguments for `plan`, running `command`. `seccomp` is the
/// open file of [`seccomp_program`], kept open and inherited; `resolv`, a
/// resolver file for full internet.
pub fn bwrap_args(plan: &SandboxPlan, ids: Ids, seccomp: &File, resolv: Option<&Path>, command: &[OsString]) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |items: &[&std::ffi::OsStr]| args.extend(items.iter().map(|item| item.to_os_string()));
    push(&["--die-with-parent".as_ref(), "--unshare-user".as_ref(), "--unshare-pid".as_ref(), "--unshare-ipc".as_ref()]);
    // The person's own ids, even inside pasta's namespace, where they're 0.
    let (uid, gid) = (ids.uid.to_string(), ids.gid.to_string());
    push(&["--uid".as_ref(), uid.as_ref(), "--gid".as_ref(), gid.as_ref()]);
    push(&["--unshare-uts".as_ref(), "--unshare-cgroup-try".as_ref(), "--hostname".as_ref(), "silt-sandbox".as_ref()]);
    match plan.network {
        // Full internet runs in pasta's namespace already.
        Network::None | Network::Gateway => push(&["--unshare-net".as_ref()]),
        Network::Full | Network::Host => {}
    }
    if !plan.nested {
        push(&["--disable-userns".as_ref()]);
    }
    push(&["--proc".as_ref(), "/proc".as_ref(), "--dev".as_ref(), "/dev".as_ref()]);
    for mount in &plan.mounts {
        let path = mount.path.as_os_str();
        if let Some(source) = &mount.source {
            // Something else shown in its place, read-only.
            push(&["--ro-bind".as_ref(), source.as_os_str(), path]);
            continue;
        }
        match mount.kind {
            MountKind::ReadOnly => push(&["--ro-bind-try".as_ref(), path, path]),
            MountKind::ReadWrite => push(&["--bind-try".as_ref(), path, path]),
            MountKind::Device => push(&["--dev-bind-try".as_ref(), path, path]),
            MountKind::Tmpfs => push(&["--tmpfs".as_ref(), path]),
            // Only over what exists: a missing one has nothing to hide.
            // An empty folder of the sandbox's own; writable, so what's
            // mounted inside it later finds somewhere to go.
            MountKind::Hide if mount.path.is_dir() => push(&["--tmpfs".as_ref(), path]),
            MountKind::Hide if mount.path.exists() => push(&["--ro-bind".as_ref(), "/dev/null".as_ref(), path]),
            MountKind::Hide => {}
        }
    }
    if let Some(resolv) = resolv {
        push(&["--ro-bind".as_ref(), resolv.as_os_str(), "/etc/resolv.conf".as_ref()]);
    }
    let fd = seccomp.as_fd().as_raw_fd().to_string();
    push(&["--seccomp".as_ref(), fd.as_ref(), "--chdir".as_ref(), plan.cwd.as_os_str(), "--".as_ref()]);
    args.extend(command.iter().cloned());
    args
}

/// The user and group a sandbox runs as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ids {
    pub uid: u32,
    pub gid: u32,
}

impl Ids {
    /// This process's, or those `SILT_SANDBOX_IDS` carried in from before
    /// pasta's namespace made them 0.
    pub fn here() -> Self {
        let carried = std::env::var("SILT_SANDBOX_IDS").ok().and_then(|ids| {
            let (uid, gid) = ids.split_once(':')?;
            Some(Self { uid: uid.parse().ok()?, gid: gid.parse().ok()? })
        });
        carried.unwrap_or_else(|| Self { uid: rustix::process::getuid().as_raw(), gid: rustix::process::getgid().as_raw() })
    }

    /// The variable carrying them into pasta's namespace.
    pub fn variable(self) -> (&'static str, String) {
        ("SILT_SANDBOX_IDS", format!("{}:{}", self.uid, self.gid))
    }
}

/// Keeps `file` open across `exec`, for bubblewrap to read.
pub fn inherit(file: &File) -> io::Result<()> {
    rustix::io::fcntl_setfd(file, rustix::io::FdFlags::empty()).map_err(io::Error::from)
}

/// The seccomp filter every sandbox runs under, as bubblewrap reads it: it
/// refuses pushing input into the terminal (`TIOCSTI`, `TIOCLINUX`), which
/// would type into the person's shell, `io_uring`, which slips past other
/// filters, and vsock sockets, which reach virtual machines' hosts.
pub fn seccomp_program() -> io::Result<Vec<u8>> {
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter, SeccompRule, TargetArch,
    };
    let invalid = |error: &dyn std::fmt::Display| io::Error::new(io::ErrorKind::InvalidInput, error.to_string());
    let arg = |index: u8, value: u64| SeccompCondition::new(index, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, value);
    let arch = if cfg!(target_arch = "aarch64") { TargetArch::aarch64 } else { TargetArch::x86_64 };
    let mut rules = std::collections::BTreeMap::new();
    let ioctls = [0x5412_u64, 0x541c]
        .into_iter()
        .map(|request| arg(1, request).and_then(|condition| SeccompRule::new(vec![condition])))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| invalid(&error))?;
    rules.insert(rustix_sys::SYS_IOCTL, ioctls);
    for call in [rustix_sys::SYS_IO_URING_SETUP, rustix_sys::SYS_IO_URING_ENTER, rustix_sys::SYS_IO_URING_REGISTER] {
        rules.insert(call, Vec::new());
    }
    let vsock = arg(0, 40).and_then(|condition| SeccompRule::new(vec![condition])).map_err(|error| invalid(&error))?;
    rules.insert(rustix_sys::SYS_SOCKET, vec![vsock]);
    let filter =
        SeccompFilter::new(rules, SeccompAction::Allow, SeccompAction::Errno(1), arch).map_err(|error| invalid(&error))?;
    let program: BpfProgram = filter.try_into().map_err(|error: seccompiler::BackendError| invalid(&error))?;
    let mut bytes = Vec::with_capacity(program.len() * 8);
    for instruction in program {
        bytes.extend_from_slice(&instruction.code.to_ne_bytes());
        bytes.push(instruction.jt);
        bytes.push(instruction.jf);
        bytes.extend_from_slice(&instruction.k.to_ne_bytes());
    }
    Ok(bytes)
}

/// System call numbers the filter names, per architecture.
mod rustix_sys {
    #[cfg(target_arch = "x86_64")]
    pub const SYS_IOCTL: i64 = 16;
    #[cfg(target_arch = "x86_64")]
    pub const SYS_SOCKET: i64 = 41;
    #[cfg(target_arch = "aarch64")]
    pub const SYS_IOCTL: i64 = 29;
    #[cfg(target_arch = "aarch64")]
    pub const SYS_SOCKET: i64 = 198;
    pub const SYS_IO_URING_SETUP: i64 = 425;
    pub const SYS_IO_URING_ENTER: i64 = 426;
    pub const SYS_IO_URING_REGISTER: i64 = 427;
}

/// Makes each of `ranges` unreachable from this network namespace, with
/// routes that win over the default one by being more specific. Run before
/// the sandbox drops the right to change them.
pub fn block_routes(ranges: &[Cidr]) -> Result<(), SandboxError> {
    use rustix::net::{AddressFamily, RecvFlags, SendFlags, SocketType, netlink::SocketAddrNetlink};
    let socket =
        rustix::net::socket(AddressFamily::NETLINK, SocketType::RAW, None).map_err(|error| SandboxError::Routes(error.into()))?;
    let kernel = SocketAddrNetlink::new(0, 0);
    for (sequence, range) in ranges.iter().enumerate() {
        let message = route_message(*range, u32::try_from(sequence).unwrap_or(u32::MAX));
        rustix::net::sendto(&socket, &message, SendFlags::empty(), &kernel)
            .map_err(|error| SandboxError::Routes(error.into()))?;
        let mut answer = [0u8; 1024];
        let (read, _) =
            rustix::net::recv(&socket, &mut answer, RecvFlags::empty()).map_err(|error| SandboxError::Routes(error.into()))?;
        let answer = &answer[..read];
        // NLMSG_ERROR carries the outcome; 0 is success.
        let kind = answer.get(4..6).map(|bytes| u16::from_ne_bytes([bytes[0], bytes[1]]));
        let code = answer.get(16..20).map(|bytes| i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
        match (kind, code) {
            (Some(2), Some(0)) => {}
            // A namespace without IPv6 has nothing to reach there anyway.
            (Some(2), Some(code))
                if range.v6
                    && [rustix::io::Errno::AFNOSUPPORT, rustix::io::Errno::OPNOTSUPP, rustix::io::Errno::NETDOWN]
                        .iter()
                        .any(|errno| errno.raw_os_error() == -code) => {}
            (Some(2), Some(code)) => {
                let error = io::Error::from_raw_os_error(-code);
                return Err(SandboxError::Routes(io::Error::new(error.kind(), format!("blocking {range}: {error}"))));
            }
            _ => return Err(SandboxError::Routes(io::Error::other("the kernel's answer wasn't understood"))),
        }
    }
    Ok(())
}

/// An `RTM_NEWROUTE` making `range` unreachable.
fn route_message(range: Cidr, sequence: u32) -> Vec<u8> {
    const RTM_NEWROUTE: u16 = 24;
    const FLAGS: u16 = 0x1 | 0x4 | 0x400 | 0x200; // request, ack, create, exclusive
    let address = range.octets();
    let attribute_len = 4 + address.len();
    let padded = attribute_len.div_ceil(4) * 4;
    let total = 16 + 12 + padded;
    let mut message = Vec::with_capacity(total);
    message.extend_from_slice(&u32::try_from(total).unwrap_or(0).to_ne_bytes());
    message.extend_from_slice(&RTM_NEWROUTE.to_ne_bytes());
    message.extend_from_slice(&FLAGS.to_ne_bytes());
    message.extend_from_slice(&sequence.to_ne_bytes());
    message.extend_from_slice(&0u32.to_ne_bytes());
    // rtmsg: family, destination length, source length, tos, table (main),
    // protocol (static), scope (universe), type (unreachable), flags.
    message.extend_from_slice(&[if range.v6 { 10 } else { 2 }, range.prefix, 0, 0, 254, 4, 0, 7]);
    message.extend_from_slice(&0u32.to_ne_bytes());
    // RTA_DST
    message.extend_from_slice(&u16::try_from(attribute_len).unwrap_or(0).to_ne_bytes());
    message.extend_from_slice(&1u16.to_ne_bytes());
    message.extend_from_slice(&address);
    message.resize(total, 0);
    message
}

/// The resolver file full internet's sandbox reads.
pub fn resolver_file() -> String {
    format!("nameserver {DNS_ADDRESS}\n")
}

/// Reads a sandbox plan written for `silt sandbox run`.
pub fn read_plan(path: &Path) -> io::Result<SandboxPlan> {
    let mut text = String::new();
    File::open(path)?.read_to_string(&mut text)?;
    serde_json::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use silt_workspace::{Mount, Policy, Reach, RunPaths, plan};

    #[test]
    fn a_route_message_is_well_formed() {
        let message = route_message(Cidr::parse("10.0.0.0/8").unwrap(), 3);
        assert_eq!(message.len(), 16 + 12 + 8);
        assert_eq!(u32::from_ne_bytes(message[0..4].try_into().unwrap()) as usize, message.len());
        assert_eq!(&message[16..24], &[2, 8, 0, 0, 254, 4, 0, 7]);
        assert_eq!(&message[32..36], &[10, 0, 0, 0]);
        let v6 = route_message(Cidr::parse("fc00::/7").unwrap(), 0);
        assert_eq!(v6.len(), 16 + 12 + 20);
    }

    #[test]
    fn the_seccomp_program_compiles() {
        let program = seccomp_program().expect("a program");
        assert!(!program.is_empty() && program.len().is_multiple_of(8));
    }

    #[test]
    fn bwrap_gets_every_mount_in_order() {
        let paths = RunPaths {
            checkout: "/work/app".into(),
            silt: "/usr/bin/silt".into(),
            scratch: "/tmp/run".into(),
            ..RunPaths::default()
        };
        let mut plan = plan(&paths, &Reach::of(&Policy { network: Some(Network::None), no_nested: true, ..Policy::default() }));
        plan.mounts.push(Mount { kind: MountKind::Hide, path: "/nonexistent/secret".into(), source: None });
        let seccomp = File::open("/dev/null").unwrap();
        let args: Vec<String> = bwrap_args(&plan, Ids { uid: 1000, gid: 1000 }, &seccomp, None, &["claude".into()])
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let joined = args.join(" ");
        assert!(joined.contains("--unshare-net") && joined.contains("--disable-userns"), "{joined}");
        assert!(joined.contains("--bind-try /work/app /work/app"));
        assert!(joined.contains("--ro-bind-try /work/app/.git/config /work/app/.git/config"));
        assert!(joined.find("--bind-try /work/app ").unwrap() < joined.find("/work/app/.git/config").unwrap());
        assert!(!joined.contains("/nonexistent/secret"), "nothing to hide");
        assert!(joined.ends_with("--chdir /work/app -- claude"), "{joined}");
    }

    #[test]
    fn a_sandbox_sees_only_its_plan() {
        let Some(tools) = tools(Network::None).ok().filter(|tools| check(tools).is_ok()) else {
            eprintln!("skipped: bubblewrap can't make a sandbox here");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("app");
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        std::fs::write(checkout.join(".git/config"), "[core]\n").unwrap();
        let secret = dir.path().join("secret");
        std::fs::write(&secret, "token").unwrap();
        let paths = RunPaths {
            checkout: checkout.clone(),
            silt: "/bin/sh".into(),
            scratch: dir.path().join("run"),
            ..RunPaths::default()
        };
        let plan = plan(&paths, &Reach::of(&Policy { network: Some(Network::None), ..Policy::default() }));
        let program = seccomp_program().unwrap();
        let file_path = dir.path().join("seccomp.bpf");
        std::fs::write(&file_path, program).unwrap();
        let seccomp = File::open(&file_path).unwrap();
        inherit(&seccomp).unwrap();
        let script = format!(
            "cat {} 2>/dev/null && echo SEEN; echo ok > {}/made && echo WROTE; echo x >> .git/config 2>/dev/null || echo LOCKED",
            secret.display(),
            checkout.display()
        );
        let args = bwrap_args(&plan, Ids::here(), &seccomp, None, &["/bin/sh".into(), "-c".into(), script.into()]);
        let output = std::process::Command::new(&tools.bwrap).args(&args).output().unwrap();
        let said = String::from_utf8_lossy(&output.stdout);
        assert!(!said.contains("SEEN") && said.contains("WROTE") && said.contains("LOCKED"), "{said}");
        assert!(checkout.join("made").exists());
    }
}
