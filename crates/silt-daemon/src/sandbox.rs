//! Starting an agent in Silt's sandbox (ADR 0093), the same for the
//! terminal's agents and the web UI's: the run's paths, the variables it
//! gets, and the plan `silt sandbox-run` reads.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use silt_workspace::{ExtraDir, ExtraRepo, PathDir, Reach, RunPaths};
#[cfg(unix)]
use silt_workspace::{Mount, MountKind, Network};

use crate::CoreError;

/// The only variables from the launching environment a sandbox gets: locale,
/// terminal, and the roots of the language toolchains it builds with. An
/// allowlist, not a denylist, because an unknown variable (`GH_TOKEN`,
/// `DATABASE_URL`, a company's own) is far likelier to carry a secret than to
/// be needed, and a denylist can't know every name. A model key or a
/// workspace's credential comes back as a placeholder the gateway fills, added
/// separately, so dropping it here is right. `PATH`, `HOME` and the agent's own
/// home pointers are passed so its tools and sessions are found.
const ENV_ALLOW: [&str; 44] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "PWD",
    "OLDPWD",
    "TERM",
    "COLORTERM",
    "TERMINFO",
    "TZ",
    "LANG",
    "LANGUAGE",
    "TMPDIR",
    "EDITOR",
    "VISUAL",
    "PAGER",
    "LESS",
    "HOSTNAME",
    "CI",
    "COLUMNS",
    "LINES",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "GEMINI_CLI_HOME",
    "COPILOT_HOME",
    "QUDIKS_HOME",
    "JAVA_HOME",
    "GOPATH",
    "GOROOT",
    "GOMODCACHE",
    "GOCACHE",
    "GOFLAGS",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "PKG_CONFIG_PATH",
    "CC",
    "CXX",
    "MAKEFLAGS",
];

/// Prefixes of variables a sandbox also gets (Silt's own, `SILT_`, always): locale categories,
/// and the per-toolchain settings builds read, but for names that say they
/// hold a secret ([`SECRET_WORDS`]). Prefixes a credential hides under
/// (`AWS_`, `GITHUB_`) aren't here at all.
const ENV_ALLOW_PREFIXES: [&str; 17] = [
    "LC_",
    "CARGO_",
    "RUSTUP_",
    "RUST_",
    "RUSTC_",
    "NODE_",
    "npm_config_",
    "PNPM_",
    "COREPACK_",
    "YARN_",
    "PYENV_",
    "CONDA_",
    "DENO_",
    "BUN_",
    "GRADLE_",
    "MAVEN_",
    "CMAKE_",
];

/// The port the gateway answers on inside a sandbox.
pub const GATEWAY_PORT: u16 = 3128;

/// What starting a program in the sandbox takes.
pub struct Sandboxed {
    /// The program and arguments that start the run's command in the
    /// sandbox; the command follows them.
    pub launcher: Vec<OsString>,
    /// The run's variables, as the sandbox gets them.
    pub env: Vec<(String, String)>,
    /// What answers its relays outside, to start before it and stop after.
    pub outside: Vec<Outside>,
    /// Credentials the workspace names whose secret couldn't be had.
    pub missing: Vec<String>,
    /// Why each folder the workspace names and the sandbox doesn't get
    /// was left out.
    pub refused: Vec<String>,
}

/// What answers a sandbox's relay outside it. There are none where the
/// sandbox doesn't run, as relays are Unix sockets.
pub enum Outside {
    #[cfg(unix)]
    Gateway { listener: std::os::unix::net::UnixListener, gateway: std::sync::Arc<silt_gateway::Gateway> },
    /// The gateway on this machine's loopback, for a sandbox with no
    /// network of its own to relay it into, as Seatbelt's.
    #[cfg(unix)]
    GatewayTcp { listener: std::net::TcpListener, gateway: std::sync::Arc<silt_gateway::Gateway> },
    /// A port on this machine the workspace hands its sandboxes.
    #[cfg(unix)]
    Port { listener: std::os::unix::net::UnixListener, port: u16 },
}

impl Outside {
    /// Answers on the runtime of `handle` until the task is aborted.
    pub fn spawn(self, handle: &tokio::runtime::Handle) -> std::io::Result<tokio::task::JoinHandle<()>> {
        let _entered = handle.enter();
        match self {
            #[cfg(unix)]
            Self::Gateway { listener, gateway } => {
                let listener = accepting(listener)?;
                Ok(handle.spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        tokio::spawn(gateway.clone().answer(stream));
                    }
                }))
            }
            #[cfg(unix)]
            Self::GatewayTcp { listener, gateway } => {
                listener.set_nonblocking(true)?;
                let listener = tokio::net::TcpListener::from_std(listener)?;
                Ok(handle.spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        tokio::spawn(gateway.clone().answer(stream));
                    }
                }))
            }
            #[cfg(unix)]
            Self::Port { listener, port } => {
                let listener = accepting(listener)?;
                Ok(handle.spawn(async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        tokio::spawn(silt_gateway::forward_port(stream, port));
                    }
                }))
            }
        }
    }
}

#[cfg(unix)]
fn accepting(listener: std::os::unix::net::UnixListener) -> std::io::Result<tokio::net::UnixListener> {
    listener.set_nonblocking(true)?;
    tokio::net::UnixListener::from_std(listener)
}

impl Sandboxed {
    /// The launcher followed by `program` and its `args`.
    pub fn command(&self, program: impl Into<OsString>, args: impl IntoIterator<Item = impl Into<OsString>>) -> Vec<OsString> {
        let mut all = self.launcher.clone();
        all.push(program.into());
        all.extend(args.into_iter().map(Into::into));
        all
    }
}

/// A run to start in the sandbox.
#[derive(Clone)]
pub struct Run<'a> {
    pub data_dir: &'a Path,
    /// The thread it runs in, by id.
    pub thread: &'a str,
    pub agent: Option<&'a dyn silt_agents::Agent>,
    pub checkout: &'a Path,
    /// The workspace whose credentials its gateway adds.
    pub workspace: &'a str,
    /// Where the plan and the run's other files go; writable in the sandbox.
    pub scratch: &'a Path,
    pub reach: &'a Reach,
    /// The run's token, which the sandbox reaches the daemon with.
    pub token: &'a str,
    /// The variables the agent would get unsandboxed from the environment it
    /// was started from, which only the allowlist passes.
    pub env: Vec<(String, String)>,
    /// Those Silt, the agent's adapter or the workspace set for it on
    /// purpose, which go in as they are, but for model keys.
    pub given: Vec<(String, String)>,
    /// More that the sandbox shows read-only, such as the agent's program.
    pub read_only: Vec<PathBuf>,
    /// The devices Silt relays to the thread's sandbox.
    pub relayed: Vec<Relayed>,
    /// What the gateway adds instead of the workspace's credentials, such
    /// as a plugin's own for the MCP server it wraps (ADR 0104).
    pub gateway_credentials: Option<Vec<silt_gateway::Credential>>,
}

impl Run<'_> {
    /// It runs under macOS's Seatbelt rather than bubblewrap: no network
    /// namespace to relay into, so the gateway listens on this machine's
    /// loopback and full internet goes through it.
    fn seatbelt(&self) -> bool {
        self.reach.backend == silt_workspace::Backend::Seatbelt
    }
}

/// Writes the run's plan and works out how to start it sandboxed.
pub fn prepare(run: &Run<'_>) -> Result<Sandboxed, CoreError> {
    let lookup = |name: &str| {
        run.given.iter().rev().chain(run.env.iter().rev()).find(|(key, _)| key == name).map(|(_, value)| value.clone())
    };
    let home = silt_os::home_dir();
    let (git_dir, git_shared) = git_paths(run.checkout);
    let mut paths = RunPaths {
        home: home.clone(),
        real_home: home.as_deref().and_then(real_if_linked),
        checkout: run.checkout.to_owned(),
        git_dir,
        git_shared,
        agent_homes: match (run.agent, &home) {
            (Some(agent), Some(home)) => agent.homes(home, &lookup),
            _ => Vec::new(),
        },
        hide: match (run.agent, &home) {
            (Some(agent), Some(home)) => agent.sign_in(home, &lookup),
            _ => Vec::new(),
        },
        pin_config: match (run.agent, &home) {
            (Some(agent), Some(home)) => agent.read_only_home(home, &lookup),
            _ => Vec::new(),
        },
        git_protect: Vec::new(),
        other_projects: Vec::new(),
        own_project: Vec::new(),
        fresh: Vec::new(),
        path_dirs: lookup("PATH").map(|path| std::env::split_paths(&path).map(path_dir).collect()).unwrap_or_default(),
        silt: silt_program(run.data_dir)?,
        read_only: run.read_only.clone(),
        data_dir: Some(run.data_dir.to_owned()),
        real_data_dir: real_if_linked(run.data_dir),
        scratch: run.scratch.to_owned(),
        #[cfg(unix)]
        run_socket: Some(crate::paths::agents_socket_in(run.data_dir)),
        #[cfg(not(unix))]
        run_socket: None,
        ssh_agent_socket: lookup("SSH_AUTH_SOCK").map(PathBuf::from),
        extra: run.reach.paths.iter().map(|(path, writable)| extra_dir(path, *writable, home.as_deref())).collect(),
    };
    let refused: Vec<String> = paths.extra.iter().filter_map(|dir| paths.extra_refusal(dir)).collect();
    // As for the checkout below: a hooks folder made in the sandbox would
    // be writable there.
    for dir in paths.extra.iter().filter(|dir| dir.writable && paths.extra_refusal(dir).is_none()) {
        for repo in dir.repos.iter().flatten().filter(|repo| repo.git_dir.join("objects").is_dir()) {
            std::fs::create_dir_all(repo.git_dir.join("hooks"))?;
        }
    }
    // What's mounted must exist: an agent's home it hasn't made yet would
    // otherwise live in the sandbox alone, out of capture's sight, and a
    // missing hooks folder could be made writable there.
    for home in &paths.agent_homes {
        if !home.exists() && home.extension().is_none() {
            std::fs::create_dir_all(home)?;
        }
    }
    // A config file that doesn't exist yet would otherwise be the agent's to
    // create in its writable home and have run on the host later; create an
    // empty one so pinning it read-only closes that (ADR 0093).
    for config in &paths.pin_config {
        if !config.exists() && config.extension().is_none() {
            std::fs::create_dir_all(config)?;
        } else if !config.exists() {
            if let Some(parent) = config.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let empty = if config.extension().is_some_and(|ext| ext == "json") { "{}" } else { "" };
            std::fs::write(config, empty)?;
        }
    }
    let git_dir = paths.git_dir.clone().unwrap_or_else(|| run.checkout.join(".git"));
    if git_dir.is_dir() {
        std::fs::create_dir_all(git_dir.join("hooks"))?;
        paths.git_protect = git_protect(&git_dir)?;
    }
    std::fs::create_dir_all(run.checkout.join(".silt"))?;
    std::fs::create_dir_all(run.scratch)?;
    if let (Some(agent), Some(home)) = (run.agent, &home) {
        let state = agent.project_state(home, &lookup, run.checkout);
        // Made now, as what's hidden is hidden only where it exists, and the
        // run's own folder must be there to show it.
        for dir in state.hide.iter().chain(&state.own) {
            std::fs::create_dir_all(dir)?;
        }
        for (index, file) in state.fresh.iter().enumerate() {
            let own = run.scratch.join(format!("fresh-{index}"));
            std::fs::write(&own, "")?;
            paths.fresh.push((file.clone(), own));
        }
        for (index, file) in state.copied.iter().enumerate().filter(|(_, file)| file.is_file()) {
            let own = run.scratch.join(format!("copy-{index}"));
            std::fs::copy(file, &own)?;
            paths.fresh.push((file.clone(), own));
        }
        paths.other_projects = state.hide;
        paths.own_project = state.own;
    }
    let mut plan = silt_workspace::plan(&paths, run.reach);
    // What plugins keep for the thread and show its agents, read-only, at
    // `/silt/<plugin>`, such as GitHub's mirrors (ADR 0104). Seatbelt can't
    // show a folder under another name, so it waits for the VM backends.
    if !run.seatbelt() && !run.thread.is_empty() {
        for (source, path) in crate::plugins::wasm::agent_folders(run.data_dir, run.workspace, run.thread) {
            std::fs::create_dir_all(&source)?;
            plan.mounts.push(silt_workspace::Mount { kind: silt_workspace::MountKind::ReadOnly, path, source: Some(source) });
        }
        // Where agents clone what a plugin's mirrors hold (ADR 0103).
        if let Some(work) = crate::plugins::wasm::work_area(run.data_dir, run.workspace, run.thread) {
            std::fs::create_dir_all(&work)?;
            plan.mounts.push(silt_workspace::Mount {
                kind: silt_workspace::MountKind::ReadWrite,
                path: PathBuf::from(crate::plugins::wasm::WORK_AREA),
                source: Some(work),
            });
        }
    }
    let Relays { outside, env: mut extra_env, missing } = relays(run, &mut plan)?;
    extra_env.extend(devices(run, &mut plan));
    if run.seatbelt() {
        // Its own temporary folder, as the machine's `/tmp` can't be swapped.
        let tmp = run.scratch.join("tmp");
        std::fs::create_dir_all(&tmp)?;
        extra_env.push(("TMPDIR".to_owned(), tmp.to_string_lossy().into_owned()));
    }
    let plan_path = run.scratch.join("sandbox.json");
    // What `silt sandbox-run` reads to make the sandbox, read-only inside it;
    // the last two it writes there before starting bubblewrap.
    for file in ["sandbox.json", "seccomp.bpf", "resolv.conf"] {
        let path = run.scratch.join(file);
        plan.mounts.push(silt_workspace::Mount { kind: silt_workspace::MountKind::ReadOnly, path, source: None });
    }
    std::fs::write(&plan_path, serde_json::to_vec(&plan).map_err(|error| CoreError::Invalid(error.to_string()))?)?;
    let launcher =
        vec![paths.silt.into_os_string(), "sandbox-run".into(), "--plan".into(), plan_path.into_os_string(), "--".into()];
    // Names the person lists in `<data>/agent-env` go to every agent, here too.
    let listed = silt_acp::registry::listed_variables(run.data_dir);
    let mut env: Vec<(String, String)> = run
        .env
        .iter()
        .filter(|(name, _)| keeps(name, run.reach) || listed.contains(name))
        .filter(|(name, _)| !run.given.iter().any(|(given, _)| given == name))
        .cloned()
        .collect();
    env.extend(run.given.iter().cloned());
    env.retain(|(name, _)| name != "SILT_RUN_SOCKET" && name != "SILT_RUN_TOKEN");
    if let Some(socket) = &paths.run_socket {
        env.push(("SILT_RUN_SOCKET".into(), socket.to_string_lossy().into_owned()));
    }
    env.push(("SILT_RUN_TOKEN".into(), run.token.to_owned()));
    let added: Vec<&str> = extra_env.iter().map(|(name, _)| name.as_str()).collect();
    env.retain(|(name, _)| !added.contains(&name.as_str()));
    env.extend(extra_env);
    Ok(Sandboxed { launcher, env, outside, missing, refused })
}

/// What the devices relayed to the run add to its plan, and the variables
/// that say where they are.
fn devices(run: &Run<'_>, plan: &mut silt_workspace::SandboxPlan) -> Vec<(String, String)> {
    use silt_workspace::{Mount, MountKind};

    let mut env = Vec::new();
    let mut serials = Vec::new();
    for relayed in &run.relayed {
        match relayed {
            // Nothing can be shown at the port's path without a root of its
            // own: the run is told where its terminal is.
            Relayed::Serial { device, pts } if run.seatbelt() => {
                plan.mounts.push(Mount { kind: MountKind::Device, path: pts.clone(), source: None });
                serials.push(format!("{}={}", device.display(), pts.display()));
            }
            // The terminal the port's session runs on, at the port's own path.
            Relayed::Serial { device, pts } => {
                plan.mounts.push(Mount { kind: MountKind::Device, path: device.clone(), source: Some(pts.clone()) });
                serials.push(format!("{}={}", device.display(), device.display()));
            }
            Relayed::Adb { serial, socket } => {
                if run.seatbelt() {
                    env.push(("ADB_SERVER_SOCKET".to_owned(), format!("localfilesystem:{}", socket.display())));
                } else {
                    plan.relays.push(silt_workspace::Relay { port: ADB_PORT, socket: socket.clone() });
                }
                plan.mounts.push(Mount { kind: MountKind::ReadWrite, path: socket.clone(), source: None });
                plan.sockets.push(socket.clone());
                env.push(("ANDROID_SERIAL".to_owned(), serial.clone()));
            }
        }
    }
    if !serials.is_empty() {
        env.push(("SILT_SERIAL_PORTS".to_owned(), serials.join(";")));
    }
    env
}

/// A `PATH` folder with the links on the way to it resolved, for the plan.
fn path_dir(path: PathBuf) -> PathDir {
    let plain = path.is_absolute()
        && path.components().all(|part| matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_)));
    let link = plain
        .then(|| path.ancestors().find(|dir| dir.symlink_metadata().is_ok_and(|meta| meta.file_type().is_symlink())))
        .flatten()
        .and_then(|link| Some((link.to_owned(), std::fs::canonicalize(link).ok()?)));
    PathDir { real: std::fs::canonicalize(&path).ok(), link, path }
}

/// Where `path` really is, if a link makes that differ.
fn real_if_linked(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok().filter(|real| real != path)
}

/// What a run's relays out of the sandbox add to it.
#[derive(Default)]
struct Relays {
    outside: Vec<Outside>,
    env: Vec<(String, String)>,
    /// Credentials the workspace names whose secret couldn't be had.
    missing: Vec<String>,
}

/// The relays the run's rules give it: to the gateway, and to the ports its
/// workspace hands it.
#[cfg(unix)]
fn relays(run: &Run<'_>, plan: &mut silt_workspace::SandboxPlan) -> Result<Relays, CoreError> {
    let mut relays = Relays::default();
    // Where GitHub's plugin is on, GitHub is reached through it alone.
    let github = crate::plugins::wasm::github_on(run.data_dir, run.workspace);
    let (mut credentials, mut placeholders, unresolved) = match &run.gateway_credentials {
        Some(own) => (own.clone(), Vec::new(), Vec::new()),
        None => crate::credentials::resolve(run.data_dir, run.workspace, &|host| github && github_host(host)),
    };
    relays.missing = unresolved;
    // The agent's own model key goes on at the gateway too, so the sandbox
    // holds a placeholder rather than the key.
    if matches!(run.reach.network, Network::Gateway | Network::Full) {
        let (model_credentials, model_placeholders) = model_keys(run.agent, &|name| {
            run.given.iter().rev().chain(run.env.iter().rev()).find(|(key, _)| key == name).map(|(_, value)| value.clone())
        });
        credentials.extend(model_credentials);
        placeholders.extend(model_placeholders);
    }
    let graphql = crate::plugins::direct_graphql(run.data_dir, run.workspace);
    // Seatbelt can't refuse address ranges itself: full internet there
    // goes through the gateway, which does.
    let gateway = run.reach.network == Network::Gateway
        || (run.reach.network == Network::Full && (run.seatbelt() || !credentials.is_empty() || !graphql.is_empty()));
    if gateway {
        let ca = std::sync::Arc::new(silt_gateway::Ca::load_or_create(&run.data_dir.join("gateway"))?);
        trust(plan, &mut relays.env, run.scratch, ca.pem())?;
        let mut hosts = run.reach.hosts.clone();
        // A list of hosts still lets the agent reach its own model.
        if !hosts.is_empty()
            && let Some(agent) = run.agent
        {
            hosts.extend(agent.model_hosts().iter().map(|host| (*host).to_owned()));
        }
        let proxy_token = run.seatbelt().then(|| format!("{:032x}", uuid::Uuid::new_v4().as_u128()));
        let rules = silt_gateway::Rules {
            network: run.reach.network,
            hosts,
            blocked: silt_workspace::subtract(&plan.blocked, &plan.allowed),
            credentials,
            graphql,
            proxy_token: proxy_token.clone(),
        };
        let log = access_log(run.data_dir, run.thread);
        if let Some(dir) = log.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let gateway = std::sync::Arc::new(silt_gateway::Gateway::new(rules, ca, Some(log)));
        let proxy = match proxy_token {
            Some(token) => {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
                let port = listener.local_addr()?.port();
                relays.outside.push(Outside::GatewayTcp { listener, gateway });
                plan.gateway_port = Some(port);
                format!("http://silt:{token}@127.0.0.1:{port}")
            }
            None => {
                let socket = short_socket(run.scratch, "gateway.sock")?;
                relays.outside.push(Outside::Gateway { listener: listen(&socket)?, gateway });
                plan.relays.push(silt_workspace::Relay { port: GATEWAY_PORT, socket: socket.clone() });
                plan.mounts.push(Mount { kind: MountKind::ReadWrite, path: socket, source: None });
                format!("http://127.0.0.1:{GATEWAY_PORT}")
            }
        };
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            relays.env.push((name.to_owned(), proxy.clone()));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            relays.env.push((name.to_owned(), "localhost,127.0.0.1,::1".to_owned()));
        }
        relays.env.extend(placeholders);
    }
    // Under Seatbelt the sandbox reaches those ports on this machine's
    // loopback itself, as its profile allows.
    for port in run.reach.ports.iter().filter(|_| !run.seatbelt()) {
        let socket = short_socket(run.scratch, &format!("port-{port}.sock"))?;
        relays.outside.push(Outside::Port { listener: listen(&socket)?, port: *port });
        plan.relays.push(silt_workspace::Relay { port: *port, socket: socket.clone() });
        plan.mounts.push(Mount { kind: MountKind::ReadWrite, path: socket, source: None });
    }
    Ok(relays)
}

#[cfg(not(unix))]
fn relays(_run: &Run<'_>, _plan: &mut silt_workspace::SandboxPlan) -> Result<Relays, CoreError> {
    Ok(Relays::default())
}

/// The gateway's credentials for the model keys `agent` reads that the run
/// has, and the placeholders the sandbox gets in their place.
#[cfg(unix)]
fn model_keys(
    agent: Option<&dyn silt_agents::Agent>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> (Vec<silt_gateway::Credential>, Vec<(String, String)>) {
    let mut credentials = Vec::new();
    let mut placeholders = Vec::new();
    for api in agent.map(|agent| agent.model_apis()).unwrap_or_default() {
        let Some(key) = lookup(api.variable).filter(|key| !key.is_empty() && key != PLACEHOLDER) else { continue };
        let value = if api.scheme.is_empty() { key } else { format!("{} {key}", api.scheme) };
        credentials.push(silt_gateway::Credential {
            host: api.host.to_owned(),
            path: "/".to_owned(),
            header: api.header.to_owned(),
            value,
        });
        placeholders.push((api.variable.to_owned(), PLACEHOLDER.to_owned()));
    }
    (credentials, placeholders)
}

/// What a sandbox holds in place of a key the gateway adds.
#[cfg(unix)]
const PLACEHOLDER: &str = "silt-placeholder";

/// What a sandbox gets of a relayed device.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Relayed {
    /// The pseudo-terminal shown at the port's own path.
    Serial { device: PathBuf, pts: PathBuf },
    /// The socket the sandbox's adb port is passed to.
    Adb { serial: String, socket: PathBuf },
}

/// The port the sandbox's adb finds its server on.
pub const ADB_PORT: u16 = 5037;

/// Where the gateway notes what a thread's sandboxes reached and what it
/// refused: outside every run's own folder, so it outlives the run.
pub fn access_log(data_dir: &Path, thread: &str) -> PathBuf {
    data_dir.join("access").join(format!("{thread}.jsonl"))
}

/// Where the system keeps the certificates it trusts, by distribution.
#[cfg(unix)]
const TRUST_STORES: [&str; 4] =
    ["/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt", "/etc/ssl/ca-bundle.pem", "/etc/ssl/cert.pem"];

/// Makes the sandbox trust the gateway's authority: in the system's store,
/// shown with it added, and in the variables tools read a store from.
#[cfg(unix)]
fn trust(
    plan: &mut silt_workspace::SandboxPlan,
    env: &mut Vec<(String, String)>,
    scratch: &Path,
    authority: &str,
) -> std::io::Result<()> {
    let own = scratch.join("silt-gateway-ca.pem");
    std::fs::write(&own, authority)?;
    let system = TRUST_STORES.iter().map(Path::new).find(|store| store.is_file());
    let bundle = scratch.join("ca-bundle.pem");
    let mut all = system.map(std::fs::read_to_string).transpose()?.unwrap_or_default();
    all.push('\n');
    all.push_str(authority);
    std::fs::write(&bundle, all)?;
    for store in TRUST_STORES.iter().map(Path::new).filter(|store| store.is_file()) {
        plan.mounts.push(Mount { kind: MountKind::ReadOnly, path: store.to_owned(), source: Some(bundle.clone()) });
    }
    let bundle = bundle.to_string_lossy().into_owned();
    for name in
        ["SSL_CERT_FILE", "CURL_CA_BUNDLE", "REQUESTS_CA_BUNDLE", "GIT_SSL_CAINFO", "AWS_CA_BUNDLE", "DENO_CERT", "PIP_CERT"]
    {
        env.push((name.to_owned(), bundle.clone()));
    }
    env.push(("NODE_EXTRA_CA_CERTS".to_owned(), own.to_string_lossy().into_owned()));
    Ok(())
}

/// A socket path short enough for `sun_path`: in the run's folder, else in
/// a private folder of this user's under the temporary directory.
#[cfg(unix)]
pub(crate) fn short_socket(scratch: &Path, name: &str) -> std::io::Result<PathBuf> {
    let path = scratch.join(name);
    if path.as_os_str().len() <= 100 {
        return Ok(path);
    }
    let dir = std::env::temp_dir().join(format!("silt-{}", silt_os::uid()));
    silt_os::create_private_dir(&dir)?;
    let id = blake3::hash(path.as_os_str().as_encoded_bytes()).to_hex();
    Ok(dir.join(format!("{}-{name}", &id[..16])))
}

#[cfg(unix)]
fn listen(socket: &Path) -> std::io::Result<std::os::unix::net::UnixListener> {
    let _ = std::fs::remove_file(socket);
    let listener = std::os::unix::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(listener)
}

fn github_host(host: &str) -> bool {
    let host = host.trim_start_matches("*.");
    host == "github.com" || host.ends_with(".github.com")
}

/// Whether variable `name` goes into the sandbox.
fn keeps(name: &str, reach: &Reach) -> bool {
    if name == "SSH_AUTH_SOCK" {
        return reach.ssh_agent;
    }
    if ENV_ALLOW.contains(&name) || name.starts_with("SILT_") {
        return true;
    }
    // A toolchain's prefix also carries its registry tokens, such as
    // `CARGO_REGISTRY_TOKEN`, `NODE_AUTH_TOKEN` and `npm_config__authToken`.
    let upper = name.to_ascii_uppercase();
    ENV_ALLOW_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) && !SECRET_WORDS.iter().any(|word| upper.contains(word))
}

/// Words in a variable's name that say it holds a secret.
const SECRET_WORDS: [&str; 8] = ["TOKEN", "SECRET", "PASSWORD", "PASSWD", "AUTH", "CREDENTIAL", "APIKEY", "_KEY"];

/// How many entries a writable extra folder is walked through for git
/// repos; one with more is refused rather than handed over unchecked.
const EXTRA_WALK_LIMIT: usize = 100_000;

/// A folder as a workspace's rule names it, resolved: where it really is,
/// and the git repos in it where it's writable.
pub fn extra_dir(path: &str, writable: bool, home: Option<&Path>) -> ExtraDir {
    let path = silt_workspace::expand_home(path, home);
    let real = std::fs::canonicalize(&path).ok().filter(|real| real.is_dir());
    let repos = match &real {
        Some(real) if writable => repos_in(real, EXTRA_WALK_LIMIT),
        _ => Some(Vec::new()),
    };
    ExtraDir { path, real, writable, repos }
}

/// The `silt` a sandbox is launched with and its hooks run: this program,
/// or in a debug build the one a test names, when the test itself isn't
/// `silt`.
fn silt_program(data_dir: &Path) -> std::io::Result<PathBuf> {
    let named = crate::plugins::capabilities::redirects(data_dir).into_iter().find(|(from, _)| from == "silt");
    match named {
        Some((_, path)) => Ok(PathBuf::from(path)),
        None => std::env::current_exe(),
    }
}

/// Why `dir` is never handed to a sandbox, whatever the rules say: the
/// built-in refusals of ADR 0102, judged by both its names.
pub fn folder_refusal(data_dir: &Path, dir: &ExtraDir) -> Option<String> {
    let home = silt_os::home_dir();
    let paths = RunPaths {
        real_home: home.as_deref().and_then(real_if_linked),
        home,
        data_dir: Some(data_dir.to_owned()),
        real_data_dir: real_if_linked(data_dir),
        ..RunPaths::default()
    };
    paths.extra_refusal(dir)
}

/// The git dirs under `root`, never following a link; `None` once more
/// than `limit` entries have been looked at.
fn repos_in(root: &Path, limit: usize) -> Option<Vec<ExtraRepo>> {
    let mut repos = Vec::new();
    let mut seen = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if is_git_dir(&dir) {
            // A bare repo, a submodule's or a worktree's: only what holds
            // more of them is looked into, not its objects.
            stack.extend(["modules", "worktrees"].map(|name| dir.join(name)));
            repos.push(ExtraRepo { git_dir: dir, pointer: None, work_tree: None });
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > limit {
                return None;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if entry.file_name() == ".git" {
                if kind.is_dir() {
                    stack.extend(["modules", "worktrees"].map(|name| path.join(name)));
                    repos.push(ExtraRepo { git_dir: path, pointer: None, work_tree: Some(dir.clone()) });
                } else {
                    let git_dir = pointed_at(&path).unwrap_or_else(|| path.clone());
                    repos.push(ExtraRepo { git_dir, pointer: Some(path), work_tree: Some(dir.clone()) });
                }
            } else if kind.is_dir() {
                stack.push(path);
            }
        }
    }
    Some(repos)
}

fn is_git_dir(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && (dir.join("objects").is_dir() || dir.join("commondir").is_file())
}

/// Where a `.git` file points, resolved.
fn pointed_at(pointer: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(pointer).ok()?;
    let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let target = if target.is_absolute() { target } else { pointer.parent()?.join(target) };
    Some(std::fs::canonicalize(&target).unwrap_or(target))
}

/// A checkout's own git dir when it's elsewhere, as a managed checkout's
/// is, and the objects it borrows through alternates.
fn git_paths(checkout: &Path) -> (Option<PathBuf>, Vec<PathBuf>) {
    let pointer = std::fs::read_to_string(checkout.join(".git")).ok();
    let git_dir = pointer.as_deref().and_then(|text| text.trim().strip_prefix("gitdir:")).map(|dir| {
        let dir = PathBuf::from(dir.trim());
        if dir.is_absolute() { dir } else { checkout.join(dir) }
    });
    let own = git_dir.clone().unwrap_or_else(|| checkout.join(".git"));
    let alternates = std::fs::read_to_string(own.join("objects/info/alternates")).unwrap_or_default();
    let shared = alternates
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let path = PathBuf::from(line);
            if path.is_absolute() { path } else { own.join("objects").join(path) }
        })
        .collect();
    (git_dir, shared)
}

/// The paths under a git dir that decide what git runs, to keep read-only in
/// the sandbox (B2.3, CVE-2026-25725): `config`, `hooks` and `commondir`, for
/// the repo and for each submodule and linked worktree. The git dir itself is
/// writable (objects, refs, the index), so without this an agent could rewrite
/// a hook, or point `commondir` at a directory it controls to have git read
/// its `config` and `hooks` from there when the person next runs git outside
/// the sandbox. A missing `commondir` is created pointing at the dir itself, so
/// it can't later be made to point elsewhere.
fn git_protect(git_dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    pin_gitdir(git_dir, &mut out)?;
    for subtree in [git_dir.join("modules"), git_dir.join("worktrees")] {
        protect_subtree(&subtree, &mut out)?;
    }
    Ok(out)
}

/// Pins one git dir's `commondir`, `config`, `config.worktree` and `hooks`.
/// An absent `commondir` is written as `.` (the common dir is the git dir
/// itself, git's own default), so pinning it read-only closes the redirect.
fn pin_gitdir(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let commondir = dir.join("commondir");
    if !commondir.exists() {
        std::fs::write(&commondir, ".")?;
    }
    out.push(commondir);
    for name in ["config", "config.worktree", "hooks"] {
        let path = dir.join(name);
        if path.exists() {
            out.push(path);
        }
    }
    Ok(())
}

/// Pins every git dir nested under a `modules` or `worktrees` tree, without
/// walking into the objects, refs, logs or hooks that make those trees large.
fn protect_subtree(root: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        if dir.join("HEAD").exists() {
            pin_gitdir(&dir, out)?;
        }
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|name| name.to_str());
            if path.is_dir() && !matches!(name, Some("objects" | "refs" | "logs" | "hooks")) {
                stack.push(path);
            }
        }
    }
    Ok(())
}

/// `program` as it runs, and what it links to, for the sandbox to show: a
/// bare name is found on the run's `PATH`.
pub fn program_paths(program: &std::ffi::OsStr, env: &[(String, String)]) -> Vec<PathBuf> {
    let path = env.iter().find(|(name, _)| name == "PATH").map(|(_, value)| value.as_str()).unwrap_or_default();
    let found = if Path::new(program).components().count() > 1 {
        Some(PathBuf::from(program))
    } else {
        silt_os::find_program(&program.to_string_lossy(), std::ffi::OsStr::new(path))
    };
    let Some(found) = found else { return Vec::new() };
    let mut paths = vec![found.clone()];
    if let Ok(real) = std::fs::canonicalize(&found)
        && real != found
    {
        paths.push(real);
    }
    paths
}

/// Why `backend` can't make Silt's sandbox on this machine, if it can't.
pub fn unavailable(backend: silt_workspace::Backend) -> Option<&'static str> {
    match backend {
        silt_workspace::Backend::Bubblewrap => bubblewrap_unavailable(),
        silt_workspace::Backend::Seatbelt => seatbelt_unavailable(),
    }
}

fn seatbelt_unavailable() -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        static WHY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        WHY.get_or_init(|| silt_os::seatbelt::check().err()).as_deref()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Some("Seatbelt runs on macOS only")
    }
}

fn bubblewrap_unavailable() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        static WHY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        WHY.get_or_init(|| {
            silt_os::sandbox::tools(silt_workspace::Network::None)
                .and_then(|tools| silt_os::sandbox::check(&tools))
                .err()
                .map(|error| error.to_string())
        })
        .as_deref()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Some("bubblewrap runs on Linux only")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "protocol.file.allow=always"])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    }

    #[cfg(unix)]
    #[test]
    fn a_writable_folders_repos_are_found_without_following_links() {
        let root = tempfile::tempdir().unwrap();
        let code = std::fs::canonicalize(root.path()).unwrap().join("code");
        let (app, lib, tree) = (code.join("app"), code.join("nested/lib"), code.join("app-feature"));
        for dir in [&app, &lib] {
            std::fs::create_dir_all(dir).unwrap();
            git(dir, &["init", "-q"]);
            git(dir, &["commit", "-q", "--allow-empty", "-m", "first"]);
        }
        git(&app, &["worktree", "add", "-q", tree.to_str().unwrap()]);
        git(&app, &["submodule", "add", "-q", lib.to_str().unwrap(), "vendor/lib"]);
        let outside = root.path().join("outside");
        std::fs::create_dir_all(outside.join("repo")).unwrap();
        git(&outside.join("repo"), &["init", "-q"]);
        std::os::unix::fs::symlink(&outside, code.join("link")).unwrap();

        let dir = extra_dir(code.to_str().unwrap(), true, None);
        let repos = dir.repos.expect("walked");
        let git_dirs: Vec<&Path> = repos.iter().map(|repo| repo.git_dir.as_path()).collect();
        assert!(git_dirs.contains(&app.join(".git").as_path()), "{git_dirs:?}");
        assert!(git_dirs.contains(&lib.join(".git").as_path()));
        assert!(git_dirs.contains(&app.join(".git/modules/vendor/lib").as_path()), "the submodule's own");
        assert!(git_dirs.iter().any(|dir| dir.starts_with(app.join(".git/worktrees"))), "the worktree's own");
        assert!(repos.iter().any(|repo| repo.pointer.as_deref() == Some(tree.join(".git").as_path())));
        assert!(!git_dirs.iter().any(|dir| dir.starts_with(&outside)), "links aren't followed");

        assert_eq!(repos_in(&code, 3), None, "too much to look through");
        assert_eq!(extra_dir(code.to_str().unwrap(), false, None).repos, Some(Vec::new()), "read-only isn't walked");
        assert_eq!(extra_dir("~/nowhere-at-all", false, Some(root.path())).real, None);
    }

    #[cfg(unix)]
    #[test]
    fn credentials_stay_out_and_the_run_reaches_its_channel() {
        let data = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        // Claude's folder of its own, so the test leaves the person's alone.
        let claude = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(claude.path().join("projects/-elsewhere")).unwrap();
        std::fs::write(claude.path().join(".claude.json"), r#"{"mcpServers":{}}"#).unwrap();
        let claude_dir = claude.path().to_string_lossy().into_owned();
        let reach = Reach::of(&silt_workspace::Policy::default());
        let run = Run {
            data_dir: data.path(),
            thread: "t1",
            agent: Some(&silt_agents::claude::ClaudeCode),
            checkout: checkout.path(),
            workspace: "default",
            scratch: &data.path().join("runs/x"),
            reach: &reach,
            token: "t0k",
            relayed: Vec::new(),
            env: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("CLAUDE_CONFIG_DIR".into(), claude_dir.clone()),
                ("LANG".into(), "en_US.UTF-8".into()),
                ("CARGO_HOME".into(), "/home/a/.cargo".into()),
                ("ANTHROPIC_API_KEY".into(), "sk".into()),
                ("AWS_SECRET_ACCESS_KEY".into(), "s".into()),
                ("GOOGLE_APPLICATION_CREDENTIALS".into(), "/k.json".into()),
                ("GH_TOKEN".into(), "ghp_secret".into()),
                ("DATABASE_URL".into(), "postgres://u:p@prod/db".into()),
                ("MY_COMPANY_SECRET".into(), "s3cr3t".into()),
                ("CARGO_REGISTRY_TOKEN".into(), "cio".into()),
                ("CARGO_REGISTRIES_ACME_TOKEN".into(), "acme".into()),
                ("NODE_AUTH_TOKEN".into(), "npm".into()),
                ("npm_config__authToken".into(), "npm".into()),
                ("YARN_NPM_AUTH_TOKEN".into(), "yarn".into()),
                ("NODE_OPTIONS".into(), "--max-old-space-size=4096".into()),
                ("SSH_AUTH_SOCK".into(), "/run/agent".into()),
            ],
            read_only: Vec::new(),
            given: Vec::new(),
            gateway_credentials: None,
        };
        let sandboxed = prepare(&run).unwrap();
        let names: Vec<&str> = sandboxed.env.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"PATH") && names.contains(&"LANG") && names.contains(&"CARGO_HOME"), "{names:?}");
        #[cfg(unix)]
        {
            // The agent's key goes on at the gateway: the sandbox holds a placeholder.
            assert!(sandboxed.env.contains(&("ANTHROPIC_API_KEY".into(), PLACEHOLDER.into())), "{:?}", sandboxed.env);
            assert!(sandboxed.outside.iter().any(|outside| matches!(outside, Outside::Gateway { .. })));
        }
        assert!(!names.contains(&"AWS_SECRET_ACCESS_KEY") && !names.contains(&"GOOGLE_APPLICATION_CREDENTIALS"));
        // The allowlist drops a token or URL from the launching shell, named or
        // not, rather than trusting a denylist to know every secret's name.
        assert!(names.contains(&"NODE_OPTIONS"), "a toolchain's own settings still go in: {names:?}");
        for dropped in [
            "GH_TOKEN",
            "DATABASE_URL",
            "MY_COMPANY_SECRET",
            "CARGO_REGISTRY_TOKEN",
            "CARGO_REGISTRIES_ACME_TOKEN",
            "NODE_AUTH_TOKEN",
            "npm_config__authToken",
            "YARN_NPM_AUTH_TOKEN",
        ] {
            assert!(!names.contains(&dropped), "{dropped} leaked into the sandbox: {names:?}");
        }
        assert!(!names.contains(&"SSH_AUTH_SOCK"), "only when the rules hand ssh-agent over");
        assert!(sandboxed.env.contains(&("SILT_RUN_TOKEN".into(), "t0k".into())));
        assert_eq!(sandboxed.launcher[1], "sandbox-run");
        let plan: silt_workspace::SandboxPlan =
            serde_json::from_slice(&std::fs::read(data.path().join("runs/x/sandbox.json")).unwrap()).unwrap();
        let socket = crate::paths::agents_socket_in(data.path());
        assert_eq!(plan.sees(&socket), Some(true));
        assert_eq!(plan.sees(&crate::paths::socket_path_in(data.path())), None, "never the person's socket");
        assert_eq!(plan.sees(&data.path().join("silt.sqlite")), None);
        assert_eq!(plan.sees(&data.path().join("runs/x/sandbox.json")), Some(false), "its own plan can't be rewritten");
        assert_eq!(plan.sees(&data.path().join("runs/x/settings.json")), Some(true));
        let claude = claude.path();
        assert_eq!(plan.sees(&claude.join(".credentials.json")), None, "the agent's sign-in is hidden");
        assert_eq!(plan.sees(&claude.join("settings.json")), Some(false), "the agent's hooks config is read-only");
        for pinned in ["CLAUDE.md", "plugins/x/hooks.json", "hooks/on-stop.sh", "commands/x.md", "skills/x/SKILL.md"] {
            assert_eq!(plan.sees(&claude.join(pinned)), Some(false), "{pinned}");
        }
        assert_eq!(plan.sees(&claude.join("projects/-elsewhere/s.jsonl")), None, "another project's sessions");
        let own = claude.join("projects").join(silt_agents::claude::project_slug(checkout.path()));
        assert_eq!(plan.sees(&own.join("s.jsonl")), Some(true));
        assert!(own.is_dir(), "made, so it's there to show");
        let copy = plan.mounts.iter().find(|mount| mount.path == claude.join(".claude.json")).expect("its own copy");
        assert_eq!(copy.kind, MountKind::ReadWrite);
        let copied = copy.source.as_deref().expect("a copy");
        assert!(copied.starts_with(data.path().join("runs/x")));
        assert_eq!(std::fs::read_to_string(copied).unwrap(), r#"{"mcpServers":{}}"#);
    }

    #[test]
    fn git_protect_pins_config_hooks_and_commondir() {
        let dir = tempfile::tempdir().unwrap();
        let git = dir.path().join(".git");
        std::fs::create_dir_all(git.join("hooks")).unwrap();
        std::fs::write(git.join("config"), "[core]\n").unwrap();
        // A submodule's own git dir, with its own config and hooks.
        let sub = git.join("modules/libs/a");
        std::fs::create_dir_all(sub.join("hooks")).unwrap();
        std::fs::write(sub.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(sub.join("config"), "[core]\n").unwrap();

        let protect = git_protect(&git).unwrap();

        // An absent commondir is created pointing at the git dir itself, and
        // pinned, so it can't later be made to redirect config and hooks.
        assert_eq!(std::fs::read_to_string(git.join("commondir")).unwrap(), ".");
        for pinned in [git.join("commondir"), git.join("config"), git.join("hooks")] {
            assert!(protect.contains(&pinned), "{pinned:?} not pinned: {protect:?}");
        }
        // The submodule gets the same treatment, so its config and hooks can't
        // be rewritten to run on the host either.
        assert_eq!(std::fs::read_to_string(sub.join("commondir")).unwrap(), ".");
        for pinned in [sub.join("commondir"), sub.join("config"), sub.join("hooks")] {
            assert!(protect.contains(&pinned), "{pinned:?} not pinned: {protect:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn under_seatbelt_the_gateway_is_on_the_loopback_behind_a_password() {
        let data = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        let reach = Reach::of(&silt_workspace::Policy {
            ports: [5432].into(),
            backend: Some(silt_workspace::Backend::Seatbelt),
            ..silt_workspace::Policy::default()
        });
        let scratch = data.path().join("runs/m");
        let run = Run {
            data_dir: data.path(),
            thread: "t1",
            agent: None,
            checkout: checkout.path(),
            workspace: "work",
            scratch: &scratch,
            reach: &reach,
            token: "t",
            env: vec![("PATH".into(), "/usr/bin".into())],
            read_only: Vec::new(),
            given: Vec::new(),
            gateway_credentials: None,
            relayed: vec![
                Relayed::Serial { device: "/dev/cu.usbserial-1".into(), pts: "/dev/ttys012".into() },
                Relayed::Adb { serial: "R58M".into(), socket: scratch.join("adb.sock") },
            ],
        };
        let sandboxed = prepare(&run).unwrap();
        assert!(
            matches!(sandboxed.outside.as_slice(), [Outside::GatewayTcp { .. }]),
            "full internet goes through it; no port relays"
        );
        let env = |name: &str| sandboxed.env.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
        let plan: silt_workspace::SandboxPlan =
            serde_json::from_slice(&std::fs::read(scratch.join("sandbox.json")).unwrap()).unwrap();
        let port = plan.gateway_port.expect("a port on the loopback");
        let proxy = env("HTTPS_PROXY").unwrap();
        assert!(proxy.starts_with("http://silt:") && proxy.ends_with(&format!("@127.0.0.1:{port}")), "{proxy}");
        assert!(plan.relays.is_empty(), "nothing to relay into");
        assert_eq!(env("ADB_SERVER_SOCKET"), Some(format!("localfilesystem:{}", scratch.join("adb.sock").display())));
        assert_eq!(env("SILT_SERIAL_PORTS").as_deref(), Some("/dev/cu.usbserial-1=/dev/ttys012"));
        assert_eq!(env("TMPDIR"), Some(scratch.join("tmp").to_string_lossy().into_owned()));
        let profile = silt_workspace::seatbelt_profile(&plan, &silt_workspace::SeatbeltPaths::default());
        assert!(
            profile.contains(&format!("(remote ip \"localhost:{port}\")")) && profile.contains("localhost:5432"),
            "{profile}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn through_silt_only_relays_to_a_gateway_the_sandbox_trusts() {
        let data = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        crate::credentials::save(
            data.path(),
            &[crate::credentials::StoredCredential {
                name: "gh".into(),
                workspace: "work".into(),
                host: "api.github.com".into(),
                path: "/".into(),
                header: "authorization".into(),
                scheme: "Bearer".into(),
                source: crate::credentials::Source::Command { argv: vec!["echo".into(), "s3cret".into()] },
                placeholder: Some("GH_TOKEN".into()),
                plugin: None,
            }],
        )
        .unwrap();
        let reach = Reach::of(&silt_workspace::Policy {
            network: Some(Network::Gateway),
            ports: [5432].into(),
            ..silt_workspace::Policy::default()
        });
        let scratch = data.path().join("runs/y");
        let run = Run {
            data_dir: data.path(),
            thread: "t1",
            agent: None,
            checkout: checkout.path(),
            workspace: "work",
            scratch: &scratch,
            reach: &reach,
            token: "t",
            env: vec![("PATH".into(), "/usr/bin".into())],
            read_only: Vec::new(),
            given: Vec::new(),
            gateway_credentials: None,
            relayed: vec![
                Relayed::Serial { device: "/dev/ttyUSB0".into(), pts: "/dev/pts/9".into() },
                Relayed::Adb { serial: "R58M".into(), socket: scratch.join("adb.sock") },
            ],
        };
        let sandboxed = prepare(&run).unwrap();
        assert_eq!(sandboxed.outside.len(), 2, "the gateway and the forwarded port");
        let env = |name: &str| sandboxed.env.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
        assert_eq!(env("HTTPS_PROXY").as_deref(), Some("http://127.0.0.1:3128"));
        assert_eq!(env("GH_TOKEN").as_deref(), Some("silt-placeholder"), "never the secret");
        assert!(!sandboxed.env.iter().any(|(_, value)| value.contains("s3cret")));
        let bundle = std::fs::read_to_string(scratch.join("ca-bundle.pem")).unwrap();
        assert!(bundle.contains("BEGIN CERTIFICATE"));
        let plan: silt_workspace::SandboxPlan =
            serde_json::from_slice(&std::fs::read(scratch.join("sandbox.json")).unwrap()).unwrap();
        assert_eq!(plan.relays.iter().map(|relay| relay.port).collect::<Vec<_>>(), vec![GATEWAY_PORT, 5432, ADB_PORT]);
        let serial = plan.mounts.iter().find(|mount| mount.path == Path::new("/dev/ttyUSB0")).expect("the port is there");
        assert_eq!((serial.kind, serial.source.as_deref()), (MountKind::Device, Some(Path::new("/dev/pts/9"))));
        assert_eq!(env("ANDROID_SERIAL").as_deref(), Some("R58M"));
        assert!(!std::fs::read_to_string(scratch.join("sandbox.json")).unwrap().contains("s3cret"));
        assert_eq!(plan.sees(&data.path().join("gateway/ca-key.pem")), None, "the authority's key stays out");
    }

    #[cfg(unix)]
    #[test]
    fn where_githubs_plugin_is_on_no_github_token_reaches_the_sandbox() {
        let data = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        crate::credentials::save(
            data.path(),
            &[crate::credentials::StoredCredential {
                name: "gh".into(),
                workspace: "work".into(),
                host: "api.github.com".into(),
                path: "/".into(),
                header: "authorization".into(),
                scheme: "Bearer".into(),
                source: crate::credentials::Source::Command { argv: vec!["echo".into(), "s3cret".into()] },
                placeholder: Some("GH_TOKEN".into()),
                plugin: None,
            }],
        )
        .unwrap();
        crate::plugins::installed::ensure_shipped(data.path()).unwrap();
        crate::plugins::installed::update(data.path(), "silt.github", |installed| {
            installed.workspaces.push("work".into());
            Ok(())
        })
        .unwrap();
        let reach = Reach::of(&silt_workspace::Policy { network: Some(Network::Gateway), ..silt_workspace::Policy::default() });
        let scratch = data.path().join("runs/z");
        let run = Run {
            data_dir: data.path(),
            thread: "t1",
            agent: Some(&silt_agents::copilot::Copilot),
            checkout: checkout.path(),
            workspace: "work",
            scratch: &scratch,
            reach: &reach,
            token: "t",
            env: vec![
                ("PATH".into(), "/usr/bin".into()),
                ("GH_TOKEN".into(), "ghp_real".into()),
                ("GITHUB_TOKEN".into(), "ghp_real".into()),
                ("GH_ENTERPRISE_TOKEN".into(), "ghp_real".into()),
                ("COPILOT_GITHUB_TOKEN".into(), "ghp_copilot".into()),
            ],
            read_only: Vec::new(),
            given: Vec::new(),
            gateway_credentials: None,
            relayed: Vec::new(),
        };
        let sandboxed = prepare(&run).unwrap();
        let env = |name: &str| sandboxed.env.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
        assert!(!sandboxed.env.iter().any(|(_, value)| value.starts_with("ghp_")), "{:?}", sandboxed.env);
        assert_eq!(env("GH_TOKEN"), None, "the workspace's GitHub credential is left out where the plugin is on");
        assert_eq!(env("COPILOT_GITHUB_TOKEN").as_deref(), Some(PLACEHOLDER), "Copilot's key goes on at the gateway");
        let plan: silt_workspace::SandboxPlan =
            serde_json::from_slice(&std::fs::read(scratch.join("sandbox.json")).unwrap()).unwrap();
        assert_eq!(plan.sees(Path::new("/silt/work")), Some(true), "its work area is writable");
        assert_eq!(plan.sees(Path::new("/silt/github")), Some(false), "its mirrors are read-only");
        assert!(data.path().join("work/t1").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn a_path_folder_names_the_deepest_link_on_its_way() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let node = root.join("fnm/node-versions/v22/installation");
        std::fs::create_dir_all(node.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("fnm/aliases")).unwrap();
        std::os::unix::fs::symlink("../node-versions/v22/installation", root.join("fnm/aliases/default")).unwrap();
        std::fs::create_dir_all(root.join("multishells")).unwrap();
        let shell = root.join("multishells/42_1");
        std::os::unix::fs::symlink(root.join("fnm/aliases/default"), &shell).unwrap();
        let resolved = path_dir(shell.join("bin"));
        assert_eq!(resolved.real.as_deref(), Some(node.join("bin").as_path()));
        assert_eq!(resolved.link, Some((shell, node.clone())));
        assert_eq!(path_dir(node.join("bin")).link, None);
        assert_eq!(path_dir(root.join("multishells/../fnm")).link, None, "only plain paths");
    }

    #[test]
    fn a_managed_checkout_borrows_its_clones_objects() {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("co");
        let git = dir.path().join("clone/.git/silt/clones/t1");
        std::fs::create_dir_all(git.join("objects/info")).unwrap();
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(checkout.join(".git"), format!("gitdir: {}\n", git.display())).unwrap();
        std::fs::write(git.join("objects/info/alternates"), format!("{}\n", dir.path().join("clone/.git/objects").display()))
            .unwrap();
        let (own, shared) = git_paths(&checkout);
        assert_eq!(own, Some(git));
        assert_eq!(shared, vec![dir.path().join("clone/.git/objects")]);
    }
}
