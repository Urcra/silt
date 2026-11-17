//! What a sandboxed run sees of the machine (ADR 0093), worked out from the
//! run's paths and its reach. Its root starts empty, so this lists what goes
//! in: a credential store nobody named stays out.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::reach::{Devices, Network, Reach};

/// Address ranges a sandbox with full internet never reaches: this machine,
/// its networks and cloud metadata. A rule's ranges may open some again.
pub const ALWAYS_BLOCKED: [&str; 12] = [
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "224.0.0.0/4",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
    "ff00::/8",
];

/// How a path goes into the sandbox. Each is skipped when its path doesn't
/// exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountKind {
    ReadOnly,
    ReadWrite,
    /// A device node, or a folder of them.
    Device,
    /// An empty, writable folder of the sandbox's own.
    Tmpfs,
    /// Hidden under an empty file or folder, inside something mounted.
    Hide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub kind: MountKind,
    /// The same path inside and out, so paths in hooks and transcripts mean
    /// the same on both sides.
    pub path: PathBuf,
    /// What's shown at `path` instead of what's there, such as a trust
    /// store with the gateway's authority added.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
}

/// A port on the sandbox's own `127.0.0.1` passed to a socket outside it,
/// such as the gateway's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relay {
    pub port: u16,
    pub socket: PathBuf,
}

/// A folder of a run's `PATH`, with the links on the way to it resolved
/// outside, as the plan does no I/O.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathDir {
    pub path: PathBuf,
    /// Where it really is, every link followed.
    pub real: Option<PathBuf>,
    /// Its deepest ancestor that's a link, or itself, and where that link
    /// leads. Mounting the folder alone would leave a relative link in it
    /// such as `npx -> ../lib/…` dangling: `..` climbs out of the mount.
    pub link: Option<(PathBuf, PathBuf)>,
}

impl From<PathBuf> for PathDir {
    fn from(path: PathBuf) -> Self {
        Self { path, real: None, link: None }
    }
}

impl From<&str> for PathDir {
    fn from(path: &str) -> Self {
        PathBuf::from(path).into()
    }
}

/// Where a run's things are on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunPaths {
    pub home: Option<PathBuf>,
    /// The home folder with its links followed, where that differs, so a
    /// link can't bring it in under another name.
    pub real_home: Option<PathBuf>,
    /// The thread's checkout, which the agent works in.
    pub checkout: PathBuf,
    /// Its git dir when that's outside the checkout, as a managed
    /// checkout's is.
    pub git_dir: Option<PathBuf>,
    /// What git reads besides: a managed checkout's clone's objects.
    pub git_shared: Vec<PathBuf>,
    /// Where the agent keeps its sessions and settings.
    pub agent_homes: Vec<PathBuf>,
    /// Files inside the agent's homes that hold its sign-in, hidden after the
    /// homes are mounted so the agent's own credential never enters (ADR 0093).
    pub hide: Vec<PathBuf>,
    /// Files inside the agent's homes that configure what it runs (hooks, MCP
    /// commands), kept read-only after the homes are mounted so an agent can't
    /// plant one that runs on the host on a later unsandboxed start (ADR 0093).
    pub pin_config: Vec<PathBuf>,
    /// Paths under the git dir that configure what git runs — `config`,
    /// `hooks`, `commondir` and the same for submodules and worktrees — kept
    /// read-only so an agent can't make git run its code on the host later
    /// (B2.3, CVE-2026-25725). Worked out with I/O before the plan.
    pub git_protect: Vec<PathBuf>,
    /// Each folder of the run's `PATH`.
    pub path_dirs: Vec<PathDir>,
    /// The `silt` its hooks and MCP server run.
    pub silt: PathBuf,
    /// More it reads, such as an agent program Silt downloaded.
    pub read_only: Vec<PathBuf>,
    /// Silt's own data, hidden wherever it is.
    pub data_dir: Option<PathBuf>,
    /// The same with its links followed, where that differs.
    pub real_data_dir: Option<PathBuf>,
    /// Files Silt wrote for this run, such as the agent's settings.
    pub scratch: PathBuf,
    /// The socket that answers for this run only.
    pub run_socket: Option<PathBuf>,
    pub ssh_agent_socket: Option<PathBuf>,
}

/// Everything the sandbox is made of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPlan {
    /// In order: a later mount goes over an earlier one.
    pub mounts: Vec<Mount>,
    pub network: Network,
    /// The ranges full internet doesn't reach.
    pub blocked: Vec<String>,
    /// Ranges a rule opened past [`ALWAYS_BLOCKED`].
    pub allowed: Vec<String>,
    /// Ports on this machine reached as the sandbox's own `localhost`.
    pub ports: Vec<u16>,
    /// The agent's own sandbox may run inside.
    pub nested: bool,
    pub cwd: PathBuf,
    /// Ports inside passed to sockets outside.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<Relay>,
    /// Sockets outside the sandbox it connects to, such as the run's
    /// channel: what macOS's sandbox must name to allow.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sockets: Vec<PathBuf>,
    /// The gateway's port on this machine's loopback, where the sandbox has
    /// no network of its own to relay it into, as on macOS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_port: Option<u16>,
}

/// The system's own folders, read-only.
const SYSTEM: [&str; 11] = ["/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/nix", "/snap"];

/// Toolchains under the home folder, read-only.
const TOOLCHAINS: [&str; 18] = [
    ".cargo",
    ".rustup",
    ".nvm",
    ".volta",
    ".bun",
    ".deno",
    ".pyenv",
    ".rbenv",
    ".sdkman",
    "go",
    ".local/bin",
    ".local/lib",
    ".local/share/pnpm",
    ".local/share/mise",
    ".local/share/fnm",
    ".gitconfig",
    ".config/git",
    ".config/mise",
];

/// Caches builds write to, under the home folder.
const CACHES: [&str; 8] =
    [".cache", ".npm", ".cargo/registry", ".cargo/git", "go/pkg", ".pnpm-store", ".local/share/pnpm/store", ".gradle/caches"];

/// Credentials in the home folder, hidden even when something mounts what
/// holds them, such as a `PATH` folder the home is in.
const CREDENTIALS: [&str; 22] = [
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".config/gcloud",
    ".config/gh",
    ".config/hub",
    ".config/op",
    ".config/1Password",
    ".config/Bitwarden",
    ".password-store",
    ".local/share/keyrings",
    ".netrc",
    ".git-credentials",
    ".npmrc",
    ".pypirc",
    ".vault-token",
    ".cargo/credentials.toml",
    ".cargo/credentials",
    ".config/git/credentials",
];

/// What a run's sandbox is made of: the system and the toolchains read-only;
/// the checkout, the agent's home, caches and the run's own files writable;
/// what git or Silt would later run from read-only again; and what `reach`
/// hands over.
pub fn plan(paths: &RunPaths, reach: &Reach) -> SandboxPlan {
    let mut mounts = Mounts::default();
    for dir in SYSTEM {
        mounts.at(MountKind::ReadOnly, dir.into());
    }
    for dir in ["/tmp", "/var/tmp", "/run"] {
        mounts.at(MountKind::Tmpfs, dir.into());
    }
    let home = paths.home.as_deref();
    if let Some(home) = home {
        for dir in TOOLCHAINS {
            mounts.at(MountKind::ReadOnly, home.join(dir));
        }
    }
    // Folders the person's PATH names, wherever they are, so the tools the
    // agent calls are there; never one that holds the home folder or Silt's
    // data, by any name.
    let homes: Vec<&Path> = home.into_iter().chain(paths.real_home.as_deref()).collect();
    let data: Vec<&Path> = paths.data_dir.as_deref().into_iter().chain(paths.real_data_dir.as_deref()).collect();
    let private: Vec<&Path> = homes.iter().chain(&data).copied().collect();
    let refused =
        |dir: &Path| !dir.is_absolute() || dir.starts_with("/proc") || private.iter().any(|private| private.starts_with(dir));
    // A link's target is shown under the link's name, out of reach of the
    // credentials hidden below by their own names.
    let secret = |dir: &Path| {
        data.iter().any(|data| dir.starts_with(data))
            || homes.iter().any(|home| {
                CREDENTIALS.iter().map(|file| home.join(file)).any(|file| file.starts_with(dir) || dir.starts_with(&file))
            })
    };
    for dir in &paths.path_dirs {
        if refused(&dir.path) || dir.real.as_deref().is_some_and(refused) {
            continue;
        }
        match &dir.link {
            Some((link, target)) if !refused(link) && !refused(target) && !secret(target) => {
                mounts.add(Mount { kind: MountKind::ReadOnly, path: link.clone(), source: Some(target.clone()) });
            }
            _ => mounts.at(MountKind::ReadOnly, dir.path.clone()),
        }
    }
    mounts.at(MountKind::ReadOnly, paths.silt.clone());
    for path in &paths.read_only {
        mounts.at(MountKind::ReadOnly, path.clone());
    }
    if let Some(home) = home {
        for dir in CACHES {
            mounts.at(MountKind::ReadWrite, home.join(dir));
        }
        for file in CREDENTIALS {
            mounts.at(MountKind::Hide, home.join(file));
        }
    }
    // Then the run's own corner of it comes back, below.
    if let Some(data) = &paths.data_dir {
        mounts.at(MountKind::Hide, data.clone());
    }
    for dir in &paths.git_shared {
        mounts.at(MountKind::ReadOnly, dir.clone());
    }
    mounts.at(MountKind::ReadWrite, paths.checkout.clone());
    if let Some(git_dir) = &paths.git_dir {
        mounts.at(MountKind::ReadWrite, git_dir.clone());
    }
    for dir in &paths.agent_homes {
        mounts.at(MountKind::ReadWrite, dir.clone());
    }
    // After the homes are writable, so the sign-in a home holds is hidden
    // rather than handed over, and its hooks and other run-defining config
    // can't be rewritten to run on the host later (ADR 0093).
    for file in &paths.hide {
        mounts.at(MountKind::Hide, file.clone());
    }
    for file in &paths.pin_config {
        mounts.at(MountKind::ReadOnly, file.clone());
    }
    mounts.at(MountKind::ReadWrite, paths.scratch.clone());
    for (path, writable) in &reach.paths {
        mounts.at(if *writable { MountKind::ReadWrite } else { MountKind::ReadOnly }, PathBuf::from(path));
    }
    // Run later by git or Silt outside the sandbox, so never writable in it
    // (B2.3, CVE-2026-25725).
    let git_dir = paths.git_dir.clone().unwrap_or_else(|| paths.checkout.join(".git"));
    for protected in [git_dir.join("config"), git_dir.join("hooks"), paths.checkout.join(".silt")] {
        mounts.at(MountKind::ReadOnly, protected);
    }
    // The git dir is writable for objects, refs and the index, so the paths
    // that tell git what to run are pinned read-only on top: `config`, `hooks`
    // and `commondir` can't be rewritten, and `commondir` can't be made to
    // point git's config and hooks lookup at a directory the agent controls.
    for protected in &paths.git_protect {
        mounts.at(MountKind::ReadOnly, protected.clone());
    }
    match reach.devices {
        // Silt opens them and mounts what it relays them through.
        Devices::None | Devices::Relayed => {}
        Devices::Named => {
            for node in &reach.device_nodes {
                mounts.at(MountKind::Device, node.into());
            }
        }
        Devices::AllUsb => {
            mounts.at(MountKind::Device, "/dev/bus/usb".into());
            mounts.at(MountKind::ReadOnly, "/sys".into());
            mounts.at(MountKind::ReadOnly, "/run/udev".into());
        }
    }
    if reach.gpu {
        mounts.at(MountKind::Device, "/dev/dri".into());
        mounts.at(MountKind::ReadOnly, "/sys".into());
    }
    if reach.docker {
        mounts.at(MountKind::ReadWrite, "/var/run/docker.sock".into());
        mounts.at(MountKind::ReadWrite, "/run/docker.sock".into());
    }
    if reach.ssh_agent
        && let Some(socket) = &paths.ssh_agent_socket
    {
        mounts.at(MountKind::ReadWrite, socket.clone());
    }
    if let Some(socket) = &paths.run_socket {
        mounts.at(MountKind::ReadWrite, socket.clone());
    }
    let allowed: Vec<String> = reach.ranges.clone();
    SandboxPlan {
        mounts: mounts.0,
        network: reach.network,
        blocked: ALWAYS_BLOCKED.iter().map(|range| (*range).to_owned()).collect(),
        allowed,
        ports: reach.ports.clone(),
        nested: reach.nested,
        cwd: paths.checkout.clone(),
        relays: Vec::new(),
        sockets: paths.run_socket.iter().cloned().collect(),
        gateway_port: None,
    }
}

#[derive(Default)]
struct Mounts(Vec<Mount>);

impl Mounts {
    fn at(&mut self, kind: MountKind, path: PathBuf) {
        self.add(Mount { kind, path, source: None });
    }

    fn add(&mut self, mount: Mount) {
        // Read-only inside something read-only already adds nothing, and
        // mounting it again can fail where it's a link.
        let covered = mount.kind == MountKind::ReadOnly
            && self
                .0
                .iter()
                .rev()
                .find(|earlier| mount.path.starts_with(&earlier.path))
                .is_some_and(|earlier| earlier.kind == MountKind::ReadOnly);
        if !covered && !self.0.contains(&mount) {
            self.0.push(mount);
        }
    }
}

impl SandboxPlan {
    /// Whether `path` is visible in the sandbox, and writable there.
    pub fn sees(&self, path: &Path) -> Option<bool> {
        let mut seen = None;
        for mount in &self.mounts {
            if path.starts_with(&mount.path) {
                seen = match mount.kind {
                    MountKind::ReadOnly => Some(false),
                    MountKind::ReadWrite | MountKind::Device => Some(true),
                    // Empty: nothing of the machine's is there.
                    MountKind::Tmpfs => (path == mount.path).then_some(true),
                    MountKind::Hide => None,
                };
            }
        }
        seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Policy;

    fn paths() -> RunPaths {
        RunPaths {
            home: Some("/home/ana".into()),
            real_home: None,
            checkout: "/home/ana/work/api/.silt/checkouts/t1".into(),
            git_dir: Some("/home/ana/work/api/.git/silt/clones/t1".into()),
            git_shared: vec!["/home/ana/work/api/.git/objects".into()],
            agent_homes: vec!["/home/ana/.claude".into(), "/home/ana/.claude.json".into()],
            hide: vec!["/home/ana/.claude/.credentials.json".into()],
            pin_config: vec!["/home/ana/.claude/settings.json".into()],
            git_protect: vec![
                "/home/ana/work/api/.git/silt/clones/t1/commondir".into(),
                "/home/ana/work/api/.git/silt/clones/t1/config".into(),
            ],
            path_dirs: vec!["/home/ana/.local/bin".into(), "/home/ana".into(), "/home".into(), "/usr/bin".into()],
            silt: "/home/ana/.cargo/bin/silt".into(),
            read_only: Vec::new(),
            data_dir: Some("/home/ana/.local/share/silt".into()),
            real_data_dir: None,
            scratch: "/home/ana/.local/share/silt/runs/t1-1".into(),
            run_socket: Some("/home/ana/.local/share/silt/agents.sock".into()),
            ssh_agent_socket: Some("/run/user/1000/ssh-agent.sock".into()),
        }
    }

    #[test]
    fn credentials_are_never_in_and_the_work_is_writable() {
        let plan = plan(&paths(), &Reach::of(&Policy::default()));
        for secret in [
            "/home/ana/.ssh/id_ed25519",
            "/home/ana/.config/gcloud/credentials.db",
            "/home/ana/.aws/credentials",
            "/home/ana/.kube/config",
            "/home/ana/.config/gh/hosts.yml",
            "/home/ana/.local/share/silt/silt.sqlite",
            "/home/ana/.local/share/silt/daemon.sock",
            "/home/ana/.cargo/credentials.toml",
            "/run/user/1000/bus",
            "/run/user/1000/ssh-agent.sock",
            "/var/run/docker.sock",
        ] {
            assert_eq!(plan.sees(Path::new(secret)), None, "{secret}");
        }
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.silt/checkouts/t1/src/main.rs")), Some(true));
        assert_eq!(plan.sees(Path::new("/home/ana/.claude/projects/x.jsonl")), Some(true));
        assert_eq!(plan.sees(Path::new("/home/ana/.cargo/registry/index")), Some(true));
        assert_eq!(plan.sees(Path::new("/home/ana/.cargo/bin/cargo")), Some(false));
        assert_eq!(plan.sees(Path::new("/usr/bin/git")), Some(false));
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.git/objects/ab/cdef")), Some(false));
        assert_eq!(plan.network, Network::Full);
        assert!(plan.blocked.iter().any(|range| range == "169.254.0.0/16"));
    }

    #[test]
    fn what_runs_outside_later_stays_read_only() {
        let plan = plan(&paths(), &Reach::of(&Policy::default()));
        let git = Path::new("/home/ana/work/api/.git/silt/clones/t1");
        assert_eq!(plan.sees(&git.join("objects/pack/x.pack")), Some(true), "commits write objects");
        assert_eq!(plan.sees(&git.join("config")), Some(false));
        assert_eq!(plan.sees(&git.join("hooks/pre-commit")), Some(false));
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.silt/checkouts/t1/.silt/prepare")), Some(false));

        // Working in the clone itself.
        let here = RunPaths { checkout: "/home/ana/work/api".into(), git_dir: None, git_shared: Vec::new(), ..paths() };
        let plan = super::plan(&here, &Reach::of(&Policy::default()));
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.git/HEAD")), Some(true));
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.git/config")), Some(false));
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.git/hooks/post-checkout")), Some(false));
        assert_eq!(plan.sees(Path::new("/home/ana/work/api/.silt/checkouts/other/x")), Some(false));
    }

    #[test]
    fn the_home_folder_on_the_path_is_not_mounted() {
        let plan = plan(&paths(), &Reach::of(&Policy::default()));
        assert!(!plan.mounts.iter().any(|mount| mount.path == Path::new("/home/ana") || mount.path == Path::new("/home")));
        assert_eq!(plan.sees(Path::new("/home/ana/Documents/taxes.pdf")), None);
    }

    #[test]
    fn credentials_stay_hidden_under_whatever_mounts_them() {
        let paths = RunPaths { read_only: vec!["/home/ana".into()], ..paths() };
        let plan = plan(&paths, &Reach::of(&Policy::default()));
        assert_eq!(plan.sees(Path::new("/home/ana/notes.md")), Some(false), "named, so mounted");
        for secret in ["/home/ana/.ssh/id_ed25519", "/home/ana/.aws/credentials", "/home/ana/.local/share/silt/silt.sqlite"] {
            assert_eq!(plan.sees(Path::new(secret)), None, "{secret}");
        }
        assert_eq!(plan.sees(Path::new("/home/ana/.local/share/silt/runs/t1-1/sandbox.json")), Some(true), "its own corner");
        assert_eq!(plan.sees(Path::new("/home/ana/.local/share/silt/agents.sock")), Some(true));
    }

    /// fnm's shell folder: `/run/user/1000/fnm_multishells/<pid>_<ts>` links
    /// to a node installation whose `bin/npx` links to `../lib/…`.
    fn linked(link: &str, target: &str) -> PathDir {
        PathDir {
            path: format!("{link}/bin").into(),
            real: Some(format!("{target}/bin").into()),
            link: Some((link.into(), target.into())),
        }
    }

    #[test]
    fn a_path_folder_under_a_link_brings_its_links_target() {
        let node = "/home/ana/.local/share/fnm/node-versions/v22.11.0/installation";
        let shell = "/run/user/1000/fnm_multishells/4242_1731000000000";
        let paths = RunPaths { path_dirs: vec![linked(shell, node)], ..paths() };
        let plan = plan(&paths, &Reach::of(&Policy::default()));
        let mount = plan.mounts.iter().find(|mount| mount.path == Path::new(shell)).expect("the link's folder");
        assert_eq!((mount.kind, mount.source.as_deref()), (MountKind::ReadOnly, Some(Path::new(node))));
        assert_eq!(plan.sees(&Path::new(shell).join("bin/npx")), Some(false));
        assert_eq!(plan.sees(&Path::new(shell).join("lib/node_modules/npm/bin/npx-cli.js")), Some(false), "where ../lib leads");
        assert_eq!(plan.sees(&Path::new(node).join("bin/node")), Some(false), "fnm's own folder is a toolchain");
        assert_eq!(plan.sees(Path::new("/run/user/1000/bus")), None);
    }

    #[test]
    fn a_links_target_never_brings_the_home_folder_silts_data_or_credentials() {
        let real_home = RunPaths { real_home: Some("/var/home/ana".into()), ..paths() };
        for (paths, target) in [
            (paths(), "/home/ana"),
            (paths(), "/home"),
            (paths(), "/home/ana/.local/share/silt"),
            (paths(), "/home/ana/.local/share/silt/agents"),
            (paths(), "/home/ana/.config"),
            (paths(), "/home/ana/.aws"),
            (paths(), "/home/ana/.aws/cli"),
            (real_home.clone(), "/var/home"),
            (real_home, "/var/home/ana/.config"),
        ] {
            let paths = RunPaths { path_dirs: vec![linked("/run/x", target)], ..paths };
            let plan = plan(&paths, &Reach::of(&Policy::default()));
            assert!(!plan.mounts.iter().any(|mount| mount.source.is_some()), "{target}: {:?}", plan.mounts);
            assert!(!plan.mounts.iter().any(|mount| mount.path == Path::new("/run/x")), "{target}");
            assert_eq!(plan.sees(Path::new("/run/x/bin/tool")), Some(false), "{target}: the folder alone, as before");
            assert_eq!(plan.sees(Path::new("/run/x/.ssh/id_ed25519")), None, "{target}");
        }
    }

    #[test]
    fn a_path_folder_that_is_the_home_folder_by_another_name_is_not_mounted() {
        let paths = RunPaths {
            real_home: Some("/var/home/ana".into()),
            path_dirs: vec![
                PathDir {
                    path: "/run/x".into(),
                    real: Some("/home/ana".into()),
                    link: Some(("/run/x".into(), "/home/ana".into())),
                },
                PathDir { path: "/var/home".into(), real: Some("/var/home".into()), link: None },
            ],
            ..paths()
        };
        let plan = plan(&paths, &Reach::of(&Policy::default()));
        assert!(!plan.mounts.iter().any(|mount| mount.path == Path::new("/run/x") || mount.path == Path::new("/var/home")));
        assert_eq!(plan.sees(Path::new("/run/x/Documents/taxes.pdf")), None);
    }

    #[test]
    fn reach_hands_over_what_it_names() {
        let policy = Policy {
            devices: Some(Devices::Named),
            device_nodes: ["/dev/ttyUSB0".to_owned()].into(),
            ssh_agent: true,
            paths: [("/home/ana/notes".to_owned(), false)].into(),
            ..Policy::default()
        };
        let plan = plan(&paths(), &Reach::of(&policy));
        assert_eq!(plan.sees(Path::new("/dev/ttyUSB0")), Some(true));
        assert_eq!(plan.sees(Path::new("/run/user/1000/ssh-agent.sock")), Some(true));
        assert_eq!(plan.sees(Path::new("/home/ana/notes/todo.md")), Some(false));
        assert_eq!(plan.sees(Path::new("/dev/ttyUSB1")), None);
    }
}

/// An address range, IPv4 or IPv6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cidr {
    pub v6: bool,
    /// The address, IPv4 in the low 32 bits.
    pub address: u128,
    pub prefix: u8,
}

impl Cidr {
    fn bits(self) -> u8 {
        if self.v6 { 128 } else { 32 }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (address, prefix) = text.split_once('/').map_or((text, None), |(address, prefix)| (address, Some(prefix)));
        let (v6, address) = match address.parse::<std::net::IpAddr>().ok()? {
            std::net::IpAddr::V4(v4) => (false, u128::from(u32::from(v4))),
            std::net::IpAddr::V6(v6) => (true, u128::from(v6)),
        };
        let bits = if v6 { 128 } else { 32 };
        let prefix = match prefix {
            Some(prefix) => prefix.parse::<u8>().ok().filter(|prefix| *prefix <= bits)?,
            None => bits,
        };
        Some(Self { v6, address: address & mask(bits, prefix), prefix })
    }

    /// Whether `other` lies wholly inside it.
    pub fn contains(self, other: Self) -> bool {
        self.v6 == other.v6 && self.prefix <= other.prefix && other.address & mask(self.bits(), self.prefix) == self.address
    }

    /// Its two halves.
    fn split(self) -> [Self; 2] {
        let prefix = self.prefix + 1;
        let high = 1u128 << (self.bits() - prefix);
        [Self { prefix, ..self }, Self { address: self.address | high, prefix, ..self }]
    }

    pub fn octets(self) -> Vec<u8> {
        if self.v6 { self.address.to_be_bytes().to_vec() } else { (self.address as u32).to_be_bytes().to_vec() }
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.v6 {
            write!(f, "{}/{}", std::net::Ipv6Addr::from(self.address), self.prefix)
        } else {
            write!(f, "{}/{}", std::net::Ipv4Addr::from(self.address as u32), self.prefix)
        }
    }
}

fn mask(bits: u8, prefix: u8) -> u128 {
    if prefix == 0 {
        return 0;
    }
    let all = if bits == 128 { u128::MAX } else { u128::from(u32::MAX) };
    if prefix >= bits {
        return all;
    }
    all & !(all >> prefix)
}

/// The ranges in `blocked` less those in `allowed`, as the fewest ranges.
pub fn subtract(blocked: &[String], allowed: &[String]) -> Vec<Cidr> {
    let allowed: Vec<Cidr> = allowed.iter().filter_map(|range| Cidr::parse(range)).collect();
    let mut left = Vec::new();
    for range in blocked.iter().filter_map(|range| Cidr::parse(range)) {
        cut(range, &allowed, &mut left);
    }
    left
}

fn cut(range: Cidr, allowed: &[Cidr], left: &mut Vec<Cidr>) {
    if allowed.iter().any(|open| open.contains(range)) {
        return;
    }
    if !allowed.iter().any(|open| range.contains(*open)) {
        left.push(range);
        return;
    }
    for half in range.split() {
        cut(half, allowed, left);
    }
}

#[cfg(test)]
mod cidr_tests {
    use super::*;

    #[test]
    fn an_allowed_range_is_cut_out_of_a_blocked_one() {
        let left: Vec<String> = subtract(&["10.0.0.0/8".into(), "fc00::/7".into()], &["10.20.0.0/16".into()])
            .into_iter()
            .map(|range| range.to_string())
            .collect();
        assert!(!left.iter().any(|range| range.starts_with("10.20.")), "{left:?}");
        assert!(left.contains(&"10.0.0.0/12".to_owned()) && left.contains(&"10.21.0.0/16".to_owned()), "{left:?}");
        assert!(left.contains(&"fc00::/7".to_owned()));
        let covered = |address: &str| {
            left.iter().filter_map(|range| Cidr::parse(range)).any(|range| range.contains(Cidr::parse(address).unwrap()))
        };
        assert!(covered("10.19.255.255") && covered("10.21.0.1") && covered("10.255.0.1"));
        assert!(!covered("10.20.4.12") && !covered("8.8.8.8"));
        assert!(subtract(&["10.0.0.0/8".into()], &["10.0.0.0/8".into()]).is_empty());
        assert_eq!(Cidr::parse("10.1.2.3/8").unwrap().to_string(), "10.0.0.0/8");
        assert_eq!(Cidr::parse("nonsense"), None);
        assert_eq!(Cidr::parse("::1").unwrap().to_string(), "::1/128");
        assert!(Cidr::parse("::1/128").unwrap().contains(Cidr::parse("::1").unwrap()));
        assert_eq!(Cidr::parse("10.1.2.3").unwrap().to_string(), "10.1.2.3/32");
    }
}
