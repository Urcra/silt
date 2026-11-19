//! The capabilities whose traffic Silt can't check (ADR 0104): TCP to the
//! hosts a manifest names, one command over SSH, and a local MCP server run
//! in a sandbox of its own. Each call that uses one is opaque, and says so
//! on its card.

use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;
use silt_plugin::{Capability, Manifest, Send};
use silt_plugin_host::{Connection, Scope, SshOutput};

use super::installed::Installed;

/// The most a plugin reads from one connection at a time.
const MOST: u32 = 1 << 20;

struct Tcp(TcpStream);

impl Connection for Tcp {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.0.write_all(bytes).map_err(|error| error.to_string())
    }

    fn receive(&mut self, max: u32) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0; max.min(MOST) as usize];
        let read = self.0.read(&mut buffer).map_err(|error| error.to_string())?;
        buffer.truncate(read);
        Ok(buffer)
    }
}

pub(crate) fn connect(host: &str, port: u16) -> Result<Box<dyn Connection>, String> {
    let address = (host, port)
        .to_socket_addrs()
        .map_err(|error| format!("{host}: {error}"))?
        .next()
        .ok_or_else(|| format!("{host} has no address"))?;
    let stream =
        TcpStream::connect_timeout(&address, Duration::from_secs(10)).map_err(|error| format!("{host}:{port}: {error}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).map_err(|error| error.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(30))).map_err(|error| error.to_string())?;
    Ok(Box::new(Tcp(stream)))
}

/// One command on `host`, through this machine's `ssh` with the person's
/// keys and known hosts, and nothing else of it: no terminal, forwarding,
/// local command or prompt.
pub(crate) fn ssh(program: &Path, host: &str, command: &str, stdin: &[u8]) -> Result<SshOutput, String> {
    if host.starts_with('-') || host.contains(char::is_whitespace) {
        return Err(format!("{host} isn't a host"));
    }
    let mut child = Command::new(program)
        .args(["-T", "-o", "BatchMode=yes", "-o", "ClearAllForwardings=yes", "-o", "ForwardAgent=no", "-o", "ForwardX11=no"])
        .args(["-o", "PermitLocalCommand=no", "-o", "RequestTTY=no", "-o", "ConnectTimeout=15", "--", host, command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("ssh didn't start: {error}"))?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(stdin).map_err(|error| error.to_string())?;
    }
    let output = child.wait_with_output().map_err(|error| error.to_string())?;
    Ok(SshOutput { status: output.status.code().unwrap_or(-1), stdout: output.stdout, stderr: output.stderr })
}

/// The program `ssh` runs as: the machine's, or a test's stand-in in a
/// debug build.
pub(crate) fn ssh_program(data_dir: &Path) -> PathBuf {
    super::capabilities::redirects(data_dir)
        .into_iter()
        .find(|(from, _)| from == "ssh")
        .map_or_else(|| PathBuf::from("ssh"), |(_, to)| PathBuf::from(to))
}

/// What a plugin's MCP server is started with, from its manifest.
pub(crate) struct Server<'a> {
    pub(crate) data_dir: &'a Path,
    pub(crate) manifest: &'a Manifest,
    pub(crate) installed: &'a Installed,
    /// Its own data folder, its only writable one.
    pub(crate) own: &'a Path,
    /// The thread's checkout, shown read-only with `thread-read`.
    pub(crate) checkout: Option<&'a Path>,
    pub(crate) handle: &'a tokio::runtime::Handle,
    /// Where the plugin's settings come from, resolved for a scope.
    pub(crate) settings: &'a dyn silt_plugin_host::Capabilities,
}

impl Server<'_> {
    /// The server's command and the hosts it reaches: as the manifest
    /// names them, or the person did in the settings it names.
    fn command(&self, scope: &Scope) -> Result<(Vec<String>, Vec<String>), String> {
        let (command, command_from, mut hosts, hosts_from) = self
            .manifest
            .capabilities
            .iter()
            .find_map(|capability| match capability {
                Capability::McpProcess { command, command_from, hosts, hosts_from, .. } => {
                    Some((command.clone(), command_from.clone(), hosts.clone(), hosts_from.clone()))
                }
                _ => None,
            })
            .ok_or_else(|| "its manifest names no MCP server".to_owned())?;
        let setting = |key: &str| self.settings.setting(scope, key).and_then(|json| serde_json::from_str::<Value>(&json).ok());
        let command = match command_from {
            Some(key) => match setting(&key) {
                Some(Value::String(line)) => line.split_whitespace().map(str::to_owned).collect(),
                Some(Value::Array(words)) => words.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
                _ => Vec::new(),
            },
            None => command,
        };
        if command.first().is_none_or(|program| program.trim().is_empty()) {
            return Err(format!("set the command that starts {}'s MCP server in its settings", self.manifest.name));
        }
        if let Some(key) = hosts_from
            && let Some(value) = setting(&key)
        {
            hosts.extend(silt_plugin::hosts_in(&value));
        }
        Ok((command, hosts))
    }

    /// The plugin's own credentials for its server's hosts, as the gateway
    /// adds them: never the workspace's.
    fn credentials(&self, scope: &Scope) -> Vec<silt_gateway::Credential> {
        let mut credentials = Vec::new();
        for spec in &self.manifest.credentials {
            let id = spec.same_as.as_deref().unwrap_or(&spec.id);
            let Some(name) = self.installed.credential_for(id, scope.environment.as_deref()) else { continue };
            let Some(secret) = crate::credentials::secret_named(self.data_dir, &scope.workspace, name) else { continue };
            let (header, value) = match &spec.send {
                Send::Header { name, scheme } => {
                    (name.to_ascii_lowercase(), scheme.as_ref().map_or(secret.clone(), |scheme| format!("{scheme} {secret}")))
                }
                Send::Basic { user } => {
                    use base64::Engine as _;
                    let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}"));
                    ("authorization".to_owned(), format!("Basic {encoded}"))
                }
                Send::Query { .. } => continue,
            };
            credentials.extend(spec.hosts.iter().map(|host| silt_gateway::Credential {
                host: host.clone(),
                path: "/".to_owned(),
                header: header.clone(),
                value: value.clone(),
            }));
        }
        credentials
    }

    /// Starts the server in its sandbox, says `method` with `params` to it
    /// after MCP's handshake, and answers its result.
    #[cfg(unix)]
    pub(crate) fn ask(&self, scope: &Scope, method: &str, params: Value) -> Result<Value, String> {
        let (command, hosts) = self.command(scope)?;
        let scratch = Scratch(self.data_dir.join("runs").join(format!("mcp-{}", uuid::Uuid::new_v4().simple())));
        std::fs::create_dir_all(&scratch.0).map_err(|error| error.to_string())?;
        let reach = silt_workspace::Reach {
            network: if hosts.is_empty() { silt_workspace::Network::None } else { silt_workspace::Network::Gateway },
            hosts,
            nested: false,
            ..silt_workspace::Reach::default()
        };
        let env: Vec<(String, String)> = ["PATH", "LANG", "TZ"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.to_owned(), value)))
            .chain([("HOME".to_owned(), self.own.to_string_lossy().into_owned())])
            .collect();
        let program = std::ffi::OsStr::new(&command[0]);
        let mut read_only = crate::sandbox::program_paths(program, &env);
        if let Some(path) = which(&command[0], &env) {
            read_only.push(path);
        }
        if let Some(checkout) = self.checkout.filter(|_| self.installed.granted.contains("thread-read")) {
            read_only.push(checkout.to_owned());
        }
        std::fs::create_dir_all(self.own).map_err(|error| error.to_string())?;
        let sandboxed = crate::sandbox::prepare(&crate::sandbox::Run {
            data_dir: self.data_dir,
            thread: "",
            agent: None,
            checkout: self.own,
            workspace: &scope.workspace,
            scratch: &scratch.0,
            reach: &reach,
            token: "",
            env,
            read_only,
            relayed: Vec::new(),
            gateway_credentials: Some(self.credentials(scope)),
        })
        .map_err(|error| format!("its MCP server's sandbox couldn't be made: {error}"))?;
        let all = sandboxed.command(program, &command[1..]);
        let mut served = Vec::new();
        for outside in sandboxed.outside {
            served.push(outside.spawn(self.handle).map_err(|error| error.to_string())?);
        }
        let answered = talk(&all, &sandboxed.env, self.own, method, params);
        for task in served {
            task.abort();
        }
        answered
    }

    #[cfg(not(unix))]
    pub(crate) fn ask(&self, scope: &Scope, _method: &str, _params: Value) -> Result<Value, String> {
        let _ = (self.command(scope)?, self.credentials(scope), (self.own, self.checkout, self.handle));
        Err("a local MCP server runs in Silt's sandbox, which this system doesn't have yet".into())
    }
}

/// A server's scratch folder, gone with its call.
#[cfg(unix)]
struct Scratch(PathBuf);

#[cfg(unix)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where `program` is on the run's `PATH`, so the sandbox shows it.
#[cfg(unix)]
fn which(program: &str, env: &[(String, String)]) -> Option<PathBuf> {
    if program.contains('/') {
        return Some(PathBuf::from(program));
    }
    let path = env.iter().find(|(name, _)| name == "PATH")?.1.clone();
    std::env::split_paths(&path).map(|dir| dir.join(program)).find(|candidate| candidate.is_file())
}

/// MCP over the server's stdio: the handshake, then one request.
#[cfg(unix)]
fn talk(all: &[std::ffi::OsString], env: &[(String, String)], cwd: &Path, method: &str, params: Value) -> Result<Value, String> {
    use std::io::{BufRead as _, BufReader};

    use serde_json::json;
    let mut child = Command::new(&all[0])
        .args(&all[1..])
        .env_clear()
        .envs(env.iter().map(|(name, value)| (name, value)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("its MCP server didn't start: {error}"))?;
    let mut input = child.stdin.take().ok_or("no stdin")?;
    let messages = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "silt", "version": env!("CARGO_PKG_VERSION")}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": method, "params": params}),
    ];
    for message in messages {
        writeln!(input, "{message}").map_err(|error| error.to_string())?;
    }
    input.flush().map_err(|error| error.to_string())?;
    let output = child.stdout.take().ok_or("no stdout")?;
    let mut answer = Err("its MCP server didn't answer".to_owned());
    for line in BufReader::new(output).lines() {
        let Ok(reply) = serde_json::from_str::<Value>(&line.map_err(|error| error.to_string())?) else { continue };
        if reply["id"] == 2 {
            answer = match reply.get("error") {
                Some(error) => Err(error.get("message").and_then(Value::as_str).unwrap_or("it failed").to_owned()),
                None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
            };
            break;
        }
    }
    drop(input);
    let _ = child.kill();
    let _ = child.wait();
    answer
}
