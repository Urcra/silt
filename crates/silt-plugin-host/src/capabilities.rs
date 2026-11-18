//! What the host asks the daemon to do for a plugin, once it has decided the
//! plugin may: send a request through the gateway, run git on a mirror,
//! read the thread, widen a sandbox, run an SSH command, talk to the MCP
//! server it wraps.

use silt_plugin::{Channel, Effect, Manifest};

/// Whose call this is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub plugin: String,
    pub thread: String,
    pub workspace: String,
    pub environment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub asked: String,
    pub real: String,
    pub repos: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// A TCP connection the daemon opened for a plugin.
pub trait Connection: Send {
    fn send(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn receive(&mut self, max: u32) -> Result<Vec<u8>, String>;
}

/// What a plugin's `sockets.connection` is, in the host's table.
pub struct ConnectionResource(pub Box<dyn Connection>);

fn missing(what: &str) -> String {
    format!("this Silt can't {what} for plugins")
}

/// The daemon's side of each capability. The host has already checked the
/// plugin was granted it, that the host it names is declared, and that the
/// call's phase and approval allow it.
pub trait Capabilities: Send + Sync {
    /// A setting's value as JSON. Secrets never.
    fn setting(&self, _scope: &Scope, _key: &str) -> Option<String> {
        None
    }

    /// Through the gateway, which adds the credential for the host.
    fn http(&self, _scope: &Scope, _request: HttpRequest) -> Result<HttpResponse, String> {
        Err(missing("send HTTP requests"))
    }

    fn git_fetch(&self, _scope: &Scope, _mirror: &str, _remote: &str, _refspecs: &[String]) -> Result<(), String> {
        Err(missing("fetch with git"))
    }

    fn git_remote_head(&self, _scope: &Scope, _remote: &str, _reference: &str) -> Result<Option<String>, String> {
        Err(missing("read a remote with git"))
    }

    fn git_import(&self, _scope: &Scope, _mirror: &str, _path: &str, _reference: &str) -> Result<String, String> {
        Err(missing("import a commit"))
    }

    fn git_log(&self, _scope: &Scope, _mirror: &str, _base: &str, _head: &str) -> Result<(Vec<Commit>, Vec<FileChange>), String> {
        Err(missing("read a mirror's history"))
    }

    fn git_push(
        &self,
        _scope: &Scope,
        _mirror: &str,
        _remote: &str,
        _commit: &str,
        _reference: &str,
        _old: &str,
    ) -> Result<(), String> {
        Err(missing("push with git"))
    }

    fn thread_read(&self, _scope: &Scope, _path: &str) -> Result<Vec<u8>, String> {
        Err(missing("read the thread"))
    }

    fn thread_list(&self, _scope: &Scope, _path: &str) -> Result<Vec<String>, String> {
        Err(missing("read the thread"))
    }

    fn reach_resolve(&self, _scope: &Scope, _path: &str, _writable: bool) -> Result<Folder, String> {
        Err(missing("resolve folders"))
    }

    fn reach_grant(&self, _scope: &Scope, _real: &str, _writable: bool) -> Result<(), String> {
        Err(missing("widen a sandbox"))
    }

    fn socket_connect(&self, _scope: &Scope, _host: &str, _port: u16) -> Result<Box<dyn Connection>, String> {
        Err(missing("open connections"))
    }

    fn ssh_exec(&self, _scope: &Scope, _host: &str, _command: &str, _stdin: &[u8]) -> Result<SshOutput, String> {
        Err(missing("run SSH commands"))
    }

    fn mcp_tools(&self, _scope: &Scope) -> Result<String, String> {
        Err(missing("run MCP servers"))
    }

    fn mcp_call(&self, _scope: &Scope, _tool: &str, _arguments: &str) -> Result<String, String> {
        Err(missing("run MCP servers"))
    }
}

/// Whether `host` is among a capability's declared hosts, `*.` patterns
/// covering subdomains.
pub(crate) fn declared(manifest: &Manifest, capability: &str, host: &str) -> bool {
    manifest.capability(capability).is_some_and(|declared| declared.hosts().iter().any(|pattern| host_matches(pattern, host)))
}

/// The hosts the person named in the setting `capability` takes more
/// hosts from.
pub(crate) fn set_hosts(manifest: &Manifest, env: &crate::Env, capability: &str) -> Vec<String> {
    let Some(key) = manifest.capability(capability).and_then(silt_plugin::Capability::hosts_from) else { return Vec::new() };
    env.capabilities
        .setting(&env.scope, key)
        .and_then(|json| serde_json::from_str(&json).ok())
        .map(|value| silt_plugin::hosts_in(&value))
        .unwrap_or_default()
}

/// Whether `host` is `pattern`, or under it for a `*.` pattern.
pub(crate) fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => pattern.eq_ignore_ascii_case(host),
    }
}

/// The host of a URL, or of a git remote's URL.
pub(crate) fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_ascii_lowercase))
}

/// Whether every declared effect is one the plugin's granted capabilities
/// could cause; the effects named if not.
pub(crate) fn check_effects(manifest: &Manifest, env: &crate::Env, effects: &[Effect]) -> Result<(), String> {
    let granted = &env.granted;
    let has = |capability: &str| manifest.capability(capability).is_some() && granted.contains(capability);
    let declared = |manifest: &Manifest, capability: &str, host: &str| {
        declared(manifest, capability, host)
            || set_hosts(manifest, env, capability).iter().any(|pattern| host_matches(pattern, host))
    };
    let bad: Vec<String> = effects
        .iter()
        .filter(|effect| match effect {
            Effect::Http { url, .. } => !(has("http") && host_of(url).is_some_and(|host| declared(manifest, "http", &host))),
            Effect::GitPush { remote, .. } => {
                !(has("git") && host_of(remote).is_some_and(|host| declared(manifest, "git", &host)))
            }
            Effect::Reach { .. } => !has("reach"),
            Effect::Opaque { channel, .. } => !has(match channel {
                Channel::Sockets => "sockets",
                Channel::Ssh => "ssh",
                Channel::McpProcess => "mcp-process",
            }),
        })
        .map(|effect| serde_json::to_string(effect).unwrap_or_default())
        .collect();
    if bad.is_empty() { Ok(()) } else { Err(bad.join(", ")) }
}
