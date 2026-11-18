//! One call's store: the plugin's WASI context, its phase, and the host side
//! of each `silt:plugin` interface, which checks before it hands anything to
//! the daemon.

use std::sync::Arc;

use silt_plugin::{Channel, Ledger, Manifest, Observed, Refused};
use wasmtime::StoreLimits;
use wasmtime::component::{Resource, ResourceTable};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::Env;
use crate::bindings::silt::plugin::{git, host, http, mcp, reach, sockets, ssh, thread};
use crate::capabilities::{ConnectionResource, HttpRequest, declared, host_of};

const RESPONSE_BYTES: usize = 32 << 20;

pub(crate) enum Phase {
    /// Saying what tools it offers: nothing beyond itself.
    Describe,
    /// Preparing a call: only what looks.
    Prepare,
    /// Running an allowed call: only its approved effects.
    Run(Box<Ledger>),
}

pub(crate) struct State {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: StoreLimits,
    pub(crate) env: Env,
    pub(crate) manifest: Arc<Manifest>,
    pub(crate) phase: Phase,
    pub(crate) observed: Vec<Observed>,
    pub(crate) refused: Vec<Refused>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl State {
    fn granted(&self, capability: &str) -> Result<(), String> {
        if self.manifest.capability(capability).is_some() && self.env.granted.contains(capability) {
            Ok(())
        } else {
            Err(format!("{} wasn't granted the {capability} capability", self.manifest.name))
        }
    }

    fn reaches(&self, capability: &str, host: &str) -> Result<(), String> {
        self.granted(capability)?;
        if declared(&self.manifest, capability, host) {
            Ok(())
        } else {
            Err(format!("{host} isn't among the hosts {} declares for {capability}", self.manifest.name))
        }
    }

    /// Whether `observed` may happen now: never while describing; only looking
    /// while preparing; only as approved while running.
    fn effect(&mut self, observed: Observed, looks: bool) -> Result<(), String> {
        let outcome = match &mut self.phase {
            Phase::Describe => Err("a plugin can't reach anything while it says what tools it offers".to_owned()),
            Phase::Prepare if looks => Ok(()),
            Phase::Prepare => {
                Err("a plugin can't change anything while it prepares a call; that waits for the call to be allowed".to_owned())
            }
            Phase::Run(ledger) => ledger.check(&observed).map_err(|refused| {
                self.refused.push(refused.clone());
                refused.to_string()
            }),
        };
        if outcome.is_ok() {
            self.observed.push(observed);
        }
        outcome
    }

    fn running(&self) -> bool {
        matches!(self.phase, Phase::Run(_))
    }
}

impl host::Host for State {
    fn log(&mut self, level: host::Level, message: String) {
        let plugin = &self.manifest.id;
        let message: String = message.chars().take(4096).collect();
        match level {
            host::Level::Debug => tracing::debug!(%plugin, "{message}"),
            host::Level::Info => tracing::info!(%plugin, "{message}"),
            host::Level::Warn => tracing::warn!(%plugin, "{message}"),
            host::Level::Error => tracing::error!(%plugin, "{message}"),
        }
    }

    fn setting(&mut self, key: String) -> Option<String> {
        let secret =
            self.manifest.settings.iter().any(|setting| setting.key == key && setting.kind == silt_plugin::SettingKind::Secret);
        if secret {
            return None;
        }
        self.env.capabilities.setting(&self.env.scope, &key)
    }

    fn kv_get(&mut self, key: String) -> Option<Vec<u8>> {
        std::fs::read(kv_path(&self.env.data_dir, &key)).ok()
    }

    fn kv_set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        if value.len() > 1 << 20 {
            return Err("a stored value is at most 1 MiB; keep more in the plugin's /data".into());
        }
        let path = kv_path(&self.env.data_dir, &key);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        }
        std::fs::write(path, value).map_err(|error| error.to_string())
    }
}

fn kv_path(data: &std::path::Path, key: &str) -> std::path::PathBuf {
    data.join(".silt-kv").join(blake3::hash(key.as_bytes()).to_hex().as_str())
}

const LOOKING: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

impl http::Host for State {
    fn send(&mut self, request: http::Request) -> Result<http::Response, String> {
        let host = host_of(&request.url).ok_or_else(|| format!("{} isn't a URL", request.url))?;
        self.reaches("http", &host)?;
        if request
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("proxy-authorization"))
        {
            return Err("a plugin never sends credentials itself: Silt adds them for the hosts its manifest names".into());
        }
        let method = request.method.to_ascii_uppercase();
        let body_sha256 = request.body.as_deref().filter(|body| !body.is_empty()).map(silt_plugin::sha256_hex);
        let looks = LOOKING.contains(&method.as_str());
        self.effect(Observed::Http { method: method.clone(), url: request.url.clone(), body_sha256 }, looks)?;
        let response = self
            .env
            .capabilities
            .http(&self.env.scope, HttpRequest { method, url: request.url, headers: request.headers, body: request.body })?;
        if response.body.len() > RESPONSE_BYTES {
            return Err(format!("the answer is over {} MiB", RESPONSE_BYTES >> 20));
        }
        Ok(http::Response { status: response.status, headers: response.headers, body: response.body })
    }
}

impl git::Host for State {
    fn fetch(&mut self, mirror: String, remote: String, refspecs: Vec<String>) -> Result<(), String> {
        let host = host_of(&remote).ok_or_else(|| format!("{remote} isn't a URL"))?;
        self.reaches("git", &host)?;
        if matches!(self.phase, Phase::Describe) {
            return Err("a plugin can't fetch while it says what tools it offers".into());
        }
        if refspecs.iter().any(|spec| spec.starts_with('-') || spec.contains("..")) {
            return Err("a refspec can't start with `-` or hold `..`".into());
        }
        self.env.capabilities.git_fetch(&self.env.scope, &mirror_name(&mirror)?, &remote, &refspecs)
    }

    fn remote_head(&mut self, remote: String, reference: String) -> Result<Option<String>, String> {
        let host = host_of(&remote).ok_or_else(|| format!("{remote} isn't a URL"))?;
        self.reaches("git", &host)?;
        self.env.capabilities.git_remote_head(&self.env.scope, &remote, &reference)
    }

    fn import_commit(&mut self, mirror: String, path: String, reference: String) -> Result<String, String> {
        self.granted("git")?;
        self.granted("thread-read")?;
        if reference.starts_with('-') {
            return Err("a reference can't start with `-`".into());
        }
        self.env.capabilities.git_import(&self.env.scope, &mirror_name(&mirror)?, &relative(&path)?, &reference)
    }

    fn log(&mut self, mirror: String, base: String, head: String) -> Result<(Vec<git::Commit>, Vec<git::FileChange>), String> {
        self.granted("git")?;
        let (commits, files) = self.env.capabilities.git_log(&self.env.scope, &mirror_name(&mirror)?, &base, &head)?;
        Ok((
            commits.into_iter().map(|commit| git::Commit { id: commit.id, subject: commit.subject }).collect(),
            files.into_iter().map(|file| git::FileChange { path: file.path, added: file.added, removed: file.removed }).collect(),
        ))
    }

    fn push(&mut self, mirror: String, remote: String, commit: String, reference: String, old: String) -> Result<(), String> {
        let host = host_of(&remote).ok_or_else(|| format!("{remote} isn't a URL"))?;
        self.reaches("git", &host)?;
        self.effect(
            Observed::GitPush { remote: remote.clone(), reference: reference.clone(), old: old.clone(), new: commit.clone() },
            false,
        )?;
        self.env.capabilities.git_push(&self.env.scope, &mirror_name(&mirror)?, &remote, &commit, &reference, &old)
    }
}

/// A mirror's name: `owner/repo`-like parts, never climbing out.
fn mirror_name(name: &str) -> Result<String, String> {
    let ok = !name.is_empty()
        && name.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        });
    if ok { Ok(name.to_owned()) } else { Err(format!("{name:?} isn't a mirror's name, such as `acme/api`")) }
}

/// A path within the thread, never climbing out of it.
fn relative(path: &str) -> Result<String, String> {
    let trimmed = path.trim_start_matches("./");
    if trimmed.starts_with('/') || trimmed.split('/').any(|part| part == "..") {
        return Err(format!("{path} isn't a path within the thread"));
    }
    Ok(trimmed.to_owned())
}

impl thread::Host for State {
    fn read(&mut self, path: String) -> Result<Vec<u8>, String> {
        self.granted("thread-read")?;
        self.env.capabilities.thread_read(&self.env.scope, &relative(&path)?)
    }

    fn list_dir(&mut self, path: String) -> Result<Vec<String>, String> {
        self.granted("thread-read")?;
        self.env.capabilities.thread_list(&self.env.scope, &relative(&path)?)
    }
}

impl reach::Host for State {
    fn resolve(&mut self, path: String, writable: bool) -> Result<reach::Folder, String> {
        self.granted("reach")?;
        let folder = self.env.capabilities.reach_resolve(&self.env.scope, &path, writable)?;
        Ok(reach::Folder { asked: folder.asked, real: folder.real, repos: folder.repos })
    }

    fn grant(&mut self, real: String, writable: bool) -> Result<(), String> {
        self.granted("reach")?;
        self.effect(Observed::Reach { path: real.clone(), writable }, false)?;
        self.env.capabilities.reach_grant(&self.env.scope, &real, writable)
    }
}

impl sockets::Host for State {
    fn connect(&mut self, host: String, port: u16) -> Result<Resource<ConnectionResource>, String> {
        self.reaches("sockets", &host.to_ascii_lowercase())?;
        self.effect(Observed::Opaque { channel: Channel::Sockets }, false)?;
        let connection = self.env.capabilities.socket_connect(&self.env.scope, &host, port)?;
        self.table.push(ConnectionResource(connection)).map_err(|error| error.to_string())
    }
}

impl sockets::HostConnection for State {
    fn send(&mut self, connection: Resource<ConnectionResource>, bytes: Vec<u8>) -> Result<(), String> {
        if !self.running() {
            return Err("only an allowed call sends".into());
        }
        self.table.get_mut(&connection).map_err(|error| error.to_string())?.0.send(&bytes)
    }

    fn receive(&mut self, connection: Resource<ConnectionResource>, max: u32) -> Result<Vec<u8>, String> {
        self.table.get_mut(&connection).map_err(|error| error.to_string())?.0.receive(max.min(1 << 20))
    }

    fn drop(&mut self, connection: Resource<ConnectionResource>) -> wasmtime::Result<()> {
        self.table.delete(connection)?;
        Ok(())
    }
}

impl ssh::Host for State {
    fn exec(&mut self, host: String, command: String, stdin: Vec<u8>) -> Result<ssh::Output, String> {
        self.reaches("ssh", &host.to_ascii_lowercase())?;
        self.effect(Observed::Opaque { channel: Channel::Ssh }, false)?;
        let output = self.env.capabilities.ssh_exec(&self.env.scope, &host, &command, &stdin)?;
        Ok(ssh::Output { status: output.status, stdout: output.stdout, stderr: output.stderr })
    }
}

impl mcp::Host for State {
    fn tools(&mut self) -> Result<String, String> {
        self.granted("mcp-process")?;
        if matches!(self.phase, Phase::Describe) {
            // Describing may list the server's tools: listing changes nothing.
        }
        self.env.capabilities.mcp_tools(&self.env.scope)
    }

    fn call(&mut self, tool: String, arguments: String) -> Result<String, String> {
        self.granted("mcp-process")?;
        self.effect(Observed::Opaque { channel: Channel::McpProcess }, false)?;
        self.env.capabilities.mcp_call(&self.env.scope, &tool, &arguments)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths_never_climb_out() {
        assert!(mirror_name("acme/api").is_ok());
        assert!(mirror_name("../etc").is_err());
        assert!(mirror_name("acme//api").is_err());
        assert!(mirror_name("").is_err());
        assert_eq!(relative("./work/api").as_deref(), Ok("work/api"));
        assert!(relative("/etc/passwd").is_err());
        assert!(relative("work/../../x").is_err());
    }
}
