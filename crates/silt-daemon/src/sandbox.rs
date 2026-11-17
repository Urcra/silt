//! Starting an agent in Silt's sandbox (ADR 0093), the same for the
//! terminal's agents and the web UI's: the run's paths, the variables it
//! gets, and the plan `silt sandbox-run` reads.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use silt_workspace::{Mount, MountKind, Network};
use silt_workspace::{Reach, RunPaths};

use crate::CoreError;

/// Variables never passed into a sandbox: what they hold or point to signs
/// in as the person somewhere.
const CREDENTIAL_VARIABLES: [&str; 12] = [
    "GOOGLE_APPLICATION_CREDENTIALS",
    "CLOUDSDK_AUTH_ACCESS_TOKEN_FILE",
    "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE",
    "KUBECONFIG",
    "DOCKER_HOST",
    "DOCKER_CONFIG",
    "VAULT_TOKEN",
    "NPM_TOKEN",
    "SSH_AUTH_SOCK",
    "GPG_AGENT_INFO",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
];

/// Prefixes of variables never passed into a sandbox.
const CREDENTIAL_PREFIXES: [&str; 2] = ["AWS_", "AZURE_"];

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
}

/// What answers a sandbox's relay outside it. There are none where the
/// sandbox doesn't run, as relays are Unix sockets.
pub enum Outside {
    #[cfg(unix)]
    Gateway { listener: std::os::unix::net::UnixListener, gateway: std::sync::Arc<silt_gateway::Gateway> },
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
    pub agent: Option<&'a dyn silt_agents::Agent>,
    pub checkout: &'a Path,
    /// The workspace whose credentials its gateway adds.
    pub workspace: &'a str,
    /// Where the plan and the run's other files go; writable in the sandbox.
    pub scratch: &'a Path,
    pub reach: &'a Reach,
    /// The run's token, which the sandbox reaches the daemon with.
    pub token: &'a str,
    /// The variables the agent would get unsandboxed.
    pub env: Vec<(String, String)>,
    /// More that the sandbox shows read-only, such as the agent's program.
    pub read_only: Vec<PathBuf>,
}

/// Writes the run's plan and works out how to start it sandboxed.
pub fn prepare(run: &Run<'_>) -> Result<Sandboxed, CoreError> {
    let lookup = |name: &str| run.env.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone());
    let home = silt_os::home_dir();
    let (git_dir, git_shared) = git_paths(run.checkout);
    let paths = RunPaths {
        home: home.clone(),
        checkout: run.checkout.to_owned(),
        git_dir,
        git_shared,
        agent_homes: match (run.agent, &home) {
            (Some(agent), Some(home)) => agent.homes(home, &lookup),
            _ => Vec::new(),
        },
        path_dirs: lookup("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default(),
        silt: std::env::current_exe()?,
        read_only: run.read_only.clone(),
        data_dir: Some(run.data_dir.to_owned()),
        scratch: run.scratch.to_owned(),
        #[cfg(unix)]
        run_socket: Some(crate::paths::agents_socket_in(run.data_dir)),
        #[cfg(not(unix))]
        run_socket: None,
        ssh_agent_socket: lookup("SSH_AUTH_SOCK").map(PathBuf::from),
    };
    // What's mounted must exist: an agent's home it hasn't made yet would
    // otherwise live in the sandbox alone, out of capture's sight, and a
    // missing hooks folder could be made writable there.
    for home in &paths.agent_homes {
        if !home.exists() && home.extension().is_none() {
            std::fs::create_dir_all(home)?;
        }
    }
    let git_dir = paths.git_dir.clone().unwrap_or_else(|| run.checkout.join(".git"));
    if git_dir.is_dir() {
        std::fs::create_dir_all(git_dir.join("hooks"))?;
    }
    std::fs::create_dir_all(run.checkout.join(".silt"))?;
    let mut plan = silt_workspace::plan(&paths, run.reach);
    std::fs::create_dir_all(run.scratch)?;
    let Relays { outside, env: extra_env, missing } = relays(run, &mut plan)?;
    let plan_path = run.scratch.join("sandbox.json");
    std::fs::write(&plan_path, serde_json::to_vec(&plan).map_err(|error| CoreError::Invalid(error.to_string()))?)?;
    let launcher =
        vec![paths.silt.into_os_string(), "sandbox-run".into(), "--plan".into(), plan_path.into_os_string(), "--".into()];
    let mut env: Vec<(String, String)> = run.env.iter().filter(|(name, _)| keeps(name, run.reach)).cloned().collect();
    env.retain(|(name, _)| name != "SILT_RUN_SOCKET" && name != "SILT_RUN_TOKEN");
    if let Some(socket) = &paths.run_socket {
        env.push(("SILT_RUN_SOCKET".into(), socket.to_string_lossy().into_owned()));
    }
    env.push(("SILT_RUN_TOKEN".into(), run.token.to_owned()));
    let added: Vec<&str> = extra_env.iter().map(|(name, _)| name.as_str()).collect();
    env.retain(|(name, _)| !added.contains(&name.as_str()));
    env.extend(extra_env);
    Ok(Sandboxed { launcher, env, outside, missing })
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
    let (credentials, placeholders, unresolved) = crate::credentials::resolve(run.data_dir, run.workspace);
    relays.missing = unresolved;
    let gateway = run.reach.network == Network::Gateway || (run.reach.network == Network::Full && !credentials.is_empty());
    if gateway {
        let socket = short_socket(run.scratch, "gateway.sock")?;
        let ca = std::sync::Arc::new(silt_gateway::Ca::load_or_create(&run.data_dir.join("gateway"))?);
        trust(plan, &mut relays.env, run.scratch, ca.pem())?;
        let rules = silt_gateway::Rules {
            network: run.reach.network,
            hosts: run.reach.hosts.clone(),
            blocked: silt_workspace::subtract(&plan.blocked, &plan.allowed),
            credentials,
        };
        let log = Some(run.scratch.join("gateway.jsonl"));
        let gateway = std::sync::Arc::new(silt_gateway::Gateway::new(rules, ca, log));
        relays.outside.push(Outside::Gateway { listener: listen(&socket)?, gateway });
        plan.relays.push(silt_workspace::Relay { port: GATEWAY_PORT, socket: socket.clone() });
        plan.mounts.push(Mount { kind: MountKind::ReadWrite, path: socket, source: None });
        let proxy = format!("http://127.0.0.1:{GATEWAY_PORT}");
        for name in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            relays.env.push((name.to_owned(), proxy.clone()));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            relays.env.push((name.to_owned(), "localhost,127.0.0.1,::1".to_owned()));
        }
        relays.env.extend(placeholders);
    }
    for port in &run.reach.ports {
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
fn short_socket(scratch: &Path, name: &str) -> std::io::Result<PathBuf> {
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

/// Whether variable `name` goes into the sandbox.
fn keeps(name: &str, reach: &Reach) -> bool {
    if name == "SSH_AUTH_SOCK" {
        return reach.ssh_agent;
    }
    !CREDENTIAL_VARIABLES.contains(&name) && !CREDENTIAL_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
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

/// Whether Silt's sandbox can run agents on this machine: on Linux, with
/// bubblewrap able to make one. Elsewhere, or without it, a workspace that
/// requires a sandbox gets the agent's own, as before (ADR 0065).
pub fn supported() -> bool {
    #[cfg(target_os = "linux")]
    {
        static WORKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *WORKS.get_or_init(|| {
            silt_os::sandbox::tools(silt_workspace::Network::None).is_ok_and(|tools| silt_os::sandbox::check(&tools).is_ok())
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_stay_out_and_the_run_reaches_its_channel() {
        let data = tempfile::tempdir().unwrap();
        let checkout = tempfile::tempdir().unwrap();
        let reach = Reach::of(&silt_workspace::Policy::default());
        let run = Run {
            data_dir: data.path(),
            agent: Some(&silt_agents::claude::ClaudeCode),
            checkout: checkout.path(),
            workspace: "default",
            scratch: &data.path().join("runs/x"),
            reach: &reach,
            token: "t0k",
            env: vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("ANTHROPIC_API_KEY".into(), "sk".into()),
                ("AWS_SECRET_ACCESS_KEY".into(), "s".into()),
                ("GOOGLE_APPLICATION_CREDENTIALS".into(), "/k.json".into()),
                ("SSH_AUTH_SOCK".into(), "/run/agent".into()),
            ],
            read_only: Vec::new(),
        };
        let sandboxed = prepare(&run).unwrap();
        let names: Vec<&str> = sandboxed.env.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"PATH") && names.contains(&"ANTHROPIC_API_KEY"), "{names:?}");
        assert!(!names.contains(&"AWS_SECRET_ACCESS_KEY") && !names.contains(&"GOOGLE_APPLICATION_CREDENTIALS"));
        assert!(!names.contains(&"SSH_AUTH_SOCK"), "only when the rules hand ssh-agent over");
        assert!(sandboxed.env.contains(&("SILT_RUN_TOKEN".into(), "t0k".into())));
        assert_eq!(sandboxed.launcher[1], "sandbox-run");
        let plan: silt_workspace::SandboxPlan =
            serde_json::from_slice(&std::fs::read(data.path().join("runs/x/sandbox.json")).unwrap()).unwrap();
        #[cfg(unix)]
        assert_eq!(plan.sees(&crate::paths::agents_socket_in(data.path())), Some(true));
        assert_eq!(plan.sees(&crate::paths::socket_path_in(data.path())), None, "never the person's socket");
        assert_eq!(plan.sees(&data.path().join("silt.sqlite")), None);
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
            agent: None,
            checkout: checkout.path(),
            workspace: "work",
            scratch: &scratch,
            reach: &reach,
            token: "t",
            env: vec![("PATH".into(), "/usr/bin".into())],
            read_only: Vec::new(),
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
        assert_eq!(plan.relays.iter().map(|relay| relay.port).collect::<Vec<_>>(), vec![GATEWAY_PORT, 5432]);
        assert!(!std::fs::read_to_string(scratch.join("sandbox.json")).unwrap().contains("s3cret"));
        assert_eq!(plan.sees(&data.path().join("gateway/ca-key.pem")), None, "the authority's key stays out");
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
