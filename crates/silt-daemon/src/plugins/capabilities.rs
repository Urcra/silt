//! The daemon's side of a plugin's capabilities (ADR 0104), once the plugin
//! host has decided the plugin may: requests sent with the credential its
//! manifest names, git on mirrors Silt keeps for it, the thread read.
//!
//! Runs on a blocking thread: the host's calls are synchronous.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use silt_plugin::{Manifest, Send};
use silt_plugin_host::{Capabilities, Commit, FileChange, Folder, HttpRequest, HttpResponse, Scope};

use super::installed::Installed;

pub(crate) struct DaemonCapabilities {
    pub(crate) data_dir: PathBuf,
    pub(crate) manifest: Arc<Manifest>,
    pub(crate) installed: Installed,
    /// What the plugin keeps for this thread; its `agent/` holds mirrors.
    pub(crate) thread_dir: PathBuf,
    /// The thread's checkout, if it has one on this machine.
    pub(crate) checkout: Option<PathBuf>,
    /// The thread's work area, where agents clone what they work on.
    pub(crate) work: Option<PathBuf>,
    pub(crate) handle: tokio::runtime::Handle,
    /// Who the folders a running call grants go to; `None` outside a call
    /// that runs.
    pub(crate) folders: Option<crate::folders::FolderScope>,
    /// The plugin's own data folder, which its MCP server writes in.
    pub(crate) own: PathBuf,
}

/// Where tests send a plugin's requests instead, as URL prefixes: read
/// from `plugins/redirect.json` in Silt's data by a debug build only, so a
/// GitHub stand-in can answer for github.com.
pub(crate) fn redirects(data_dir: &Path) -> Vec<(String, String)> {
    if !cfg!(debug_assertions) {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(data_dir.join("plugins/redirect.json")) else { return Vec::new() };
    serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&bytes)
        .map(|map| map.into_iter().collect())
        .unwrap_or_default()
}

impl DaemonCapabilities {
    /// The secret to send to `host`, and how, from the credential the
    /// manifest names for it.
    fn credential_for(&self, scope: &Scope, host: &str) -> Option<(Send, String)> {
        let set_hosts = |key: &str| {
            self.setting(scope, key)
                .and_then(|json| serde_json::from_str(&json).ok())
                .map(|value| silt_plugin::hosts_in(&value))
                .unwrap_or_default()
        };
        let spec = self.manifest.credentials.iter().find(|spec| {
            spec.hosts.iter().any(|pattern| host_matches(pattern, host))
                || spec.hosts_from.as_deref().is_some_and(|key| set_hosts(key).iter().any(|pattern| host_matches(pattern, host)))
        })?;
        let id = spec.same_as.as_deref().unwrap_or(&spec.id);
        let name = self.installed.credential_for(id, scope.environment.as_deref())?;
        let secret = crate::credentials::secret_named(&self.data_dir, &scope.workspace, name)?;
        Some((spec.send.clone(), secret))
    }

    fn mcp_server(&self) -> super::local::Server<'_> {
        super::local::Server {
            settings: self,
            data_dir: &self.data_dir,
            manifest: &self.manifest,
            installed: &self.installed,
            own: &self.own,
            checkout: self.checkout.as_deref(),
            handle: &self.handle,
        }
    }

    /// `url`, where a test redirects it.
    fn to(&self, url: &str) -> String {
        for (from, to) in redirects(&self.data_dir) {
            if let Some(rest) = url.strip_prefix(&from) {
                return format!("{to}{rest}");
            }
        }
        url.to_owned()
    }

    fn mirror(&self, name: &str) -> PathBuf {
        self.thread_dir.join("agent").join(format!("{name}.git"))
    }

    /// git, as Silt runs it: none of the person's configuration, no hooks,
    /// no prompts, and `extra` headers only for this command.
    fn git(&self, directory: Option<&Path>, extra: &[String], args: &[&str]) -> Result<String, String> {
        let mut command = Command::new("git");
        if let Some(directory) = directory {
            command.arg("-C").arg(directory);
        }
        command
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "credential.helper=",
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.https.allow=always",
            ])
            .args(["-c", "protocol.http.allow=always", "-c", "protocol.file.allow=always", "-c", "core.fsmonitor=false"]);
        for header in extra {
            command.arg("-c").arg(format!("http.extraHeader={header}"));
        }
        command.args(args);
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_COMMON_DIR");
        let output = command.output().map_err(|error| format!("git didn't start: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!("git {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&output.stderr).trim()))
        }
    }

    /// The header git sends `remote`'s host, if the plugin has a credential
    /// for it.
    fn git_headers(&self, scope: &Scope, remote: &str) -> Vec<String> {
        let Some(host) = reqwest::Url::parse(remote).ok().and_then(|url| url.host_str().map(str::to_owned)) else {
            return Vec::new();
        };
        match self.credential_for(scope, &host) {
            Some((Send::Header { name, scheme }, secret)) => {
                vec![format!("{name}: {}", scheme.map_or(secret.clone(), |scheme| format!("{scheme} {secret}")))]
            }
            Some((Send::Basic { user }, secret)) => {
                vec![format!(
                    "Authorization: Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}"))
                )]
            }
            _ => Vec::new(),
        }
    }

    /// A path the plugin named, within the thread's checkout or, under
    /// `work/`, its work area; never through a link out of either.
    fn within_thread(&self, path: &str) -> Result<PathBuf, String> {
        let (root, rest) = match path.strip_prefix("work") {
            Some(rest) if rest.is_empty() || rest.starts_with('/') => (self.work.as_ref(), rest.trim_start_matches('/')),
            _ => (self.checkout.as_ref(), path),
        };
        let root = root.ok_or_else(|| "this thread has nothing there on this machine".to_owned())?;
        let root = std::fs::canonicalize(root).map_err(|error| error.to_string())?;
        let full = std::fs::canonicalize(root.join(rest)).map_err(|error| format!("{path}: {error}"))?;
        if full.starts_with(&root) { Ok(full) } else { Err(format!("{path} leads out of the thread")) }
    }
}

pub(crate) fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => pattern.eq_ignore_ascii_case(host),
    }
}

impl Capabilities for DaemonCapabilities {
    fn setting(&self, scope: &Scope, key: &str) -> Option<String> {
        let spec = self.manifest.settings.iter().find(|spec| spec.key == key)?;
        let workspace = (!scope.workspace.is_empty()).then_some(scope.workspace.as_str());
        silt_plugin::resolve_scoped(spec, &self.installed.settings, scope.environment.as_deref(), workspace, None)
            .map(|value| value.value.to_string())
    }

    fn http(&self, scope: &Scope, request: HttpRequest) -> Result<HttpResponse, String> {
        let mut url = reqwest::Url::parse(&request.url).map_err(|error| error.to_string())?;
        let host = url.host_str().unwrap_or_default().to_owned();
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in &request.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| error.to_string())?;
            let value = reqwest::header::HeaderValue::from_str(value).map_err(|error| error.to_string())?;
            headers.append(name, value);
        }
        match self.credential_for(scope, &host) {
            Some((Send::Header { name, scheme }, secret)) => {
                let value = scheme.map_or(secret.clone(), |scheme| format!("{scheme} {secret}"));
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|error| error.to_string())?;
                let mut value = reqwest::header::HeaderValue::from_str(&value)
                    .map_err(|_| "the credential isn't a valid header value".to_owned())?;
                value.set_sensitive(true);
                headers.insert(name, value);
            }
            Some((Send::Basic { user }, secret)) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{secret}"));
                let mut value =
                    reqwest::header::HeaderValue::from_str(&format!("Basic {encoded}")).map_err(|error| error.to_string())?;
                value.set_sensitive(true);
                headers.insert(reqwest::header::AUTHORIZATION, value);
            }
            Some((Send::Query { name }, secret)) => {
                url.query_pairs_mut().append_pair(&name, &secret);
            }
            None => {}
        }
        let method = reqwest::Method::from_bytes(request.method.as_bytes()).map_err(|error| error.to_string())?;
        let client = reqwest::Client::builder()
            // A credential never follows a redirect to another host.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|error| error.to_string())?;
        let url = reqwest::Url::parse(&self.to(url.as_str())).map_err(|error| error.to_string())?;
        let mut builder = client.request(method, url).headers(headers);
        if let Some(body) = request.body {
            builder = builder.body(body);
        }
        self.handle.block_on(async move {
            let response = builder.send().await.map_err(|error| format!("{host} didn't answer: {error}"))?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.as_str().to_owned(), value.to_owned())))
                .collect();
            let body = response.bytes().await.map_err(|error| error.to_string())?.to_vec();
            Ok(HttpResponse { status, headers, body })
        })
    }

    fn git_fetch(&self, scope: &Scope, mirror: &str, remote: &str, refspecs: &[String]) -> Result<(), String> {
        let path = self.mirror(mirror);
        if !path.join("HEAD").exists() {
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            self.git(Some(&path), &[], &["init", "--quiet", "--bare"])?;
        }
        let headers = self.git_headers(scope, remote);
        let remote = self.to(remote);
        let mut args = vec!["fetch", "--quiet", "--no-tags", "--prune", remote.as_str()];
        args.extend(refspecs.iter().map(String::as_str));
        if refspecs.is_empty() {
            args.push("+refs/heads/*:refs/heads/*");
        }
        self.git(Some(&path), &headers, &args).map(|_| ())
    }

    fn git_remote_head(&self, scope: &Scope, remote: &str, reference: &str) -> Result<Option<String>, String> {
        let headers = self.git_headers(scope, remote);
        let listed = self.git(None, &headers, &["ls-remote", "--", &self.to(remote), reference])?;
        Ok(listed
            .lines()
            .find(|line| line.ends_with(&format!("\t{reference}")))
            .and_then(|line| line.split('\t').next())
            .map(str::to_owned))
    }

    fn git_import(&self, _scope: &Scope, mirror: &str, path: &str, reference: &str) -> Result<String, String> {
        let source = self.within_thread(path)?;
        let target = self.mirror(mirror);
        if !target.join("HEAD").exists() {
            return Err(format!("there's no mirror {mirror} yet; fetch it first"));
        }
        // upload-pack is all that runs in the agent's repository, and it's
        // safe there: it reads objects, never the repository's hooks.
        let url = format!("file://{}", source.display());
        self.git(Some(&target), &[], &["fetch", "--quiet", "--no-tags", &url, reference])?;
        Ok(self.git(Some(&target), &[], &["rev-parse", "FETCH_HEAD"])?.trim().to_owned())
    }

    fn git_log(&self, _scope: &Scope, mirror: &str, base: &str, head: &str) -> Result<(Vec<Commit>, Vec<FileChange>), String> {
        let path = self.mirror(mirror);
        let range = if base.is_empty() { head.to_owned() } else { format!("{base}..{head}") };
        let log = self.git(Some(&path), &[], &["log", "--format=%H%x09%s", "-n", "200", &range, "--"])?;
        let commits = log
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(id, subject)| Commit { id: id.to_owned(), subject: subject.to_owned() })
            .collect();
        let diff = if base.is_empty() {
            self.git(Some(&path), &[], &["show", "--numstat", "--format=", head])?
        } else {
            self.git(Some(&path), &[], &["diff", "--numstat", base, head])?
        };
        let files = diff
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let added = parts.next()?.parse().unwrap_or(0);
                let removed = parts.next()?.parse().unwrap_or(0);
                Some(FileChange { path: parts.next()?.to_owned(), added, removed })
            })
            .collect();
        Ok((commits, files))
    }

    fn git_push(
        &self,
        scope: &Scope,
        mirror: &str,
        remote: &str,
        commit: &str,
        reference: &str,
        old: &str,
    ) -> Result<(), String> {
        let path = self.mirror(mirror);
        let headers = self.git_headers(scope, remote);
        let lease = format!("--force-with-lease={reference}:{old}");
        let refspec = format!("{commit}:{reference}");
        self.git(Some(&path), &headers, &["push", "--quiet", "--no-verify", &lease, &self.to(remote), &refspec]).map(|_| ())
    }

    fn thread_read(&self, _scope: &Scope, path: &str) -> Result<Vec<u8>, String> {
        let full = self.within_thread(path)?;
        let metadata = std::fs::metadata(&full).map_err(|error| error.to_string())?;
        if metadata.len() > 8 << 20 {
            return Err(format!("{path} is over 8 MiB"));
        }
        std::fs::read(full).map_err(|error| error.to_string())
    }

    fn thread_list(&self, _scope: &Scope, path: &str) -> Result<Vec<String>, String> {
        let full = self.within_thread(path)?;
        let mut names: Vec<String> = std::fs::read_dir(full)
            .map_err(|error| error.to_string())?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name != ".git")
            .collect();
        names.sort();
        Ok(names)
    }

    fn socket_connect(&self, _scope: &Scope, host: &str, port: u16) -> Result<Box<dyn silt_plugin_host::Connection>, String> {
        super::local::connect(host, port)
    }

    fn ssh_exec(&self, _scope: &Scope, host: &str, command: &str, stdin: &[u8]) -> Result<silt_plugin_host::SshOutput, String> {
        super::local::ssh(&super::local::ssh_program(&self.data_dir), host, command, stdin)
    }

    fn mcp_tools(&self, scope: &Scope) -> Result<String, String> {
        let listed = self.mcp_server().ask(scope, "tools/list", serde_json::json!({}))?;
        Ok(listed.to_string())
    }

    fn mcp_call(&self, scope: &Scope, tool: &str, arguments: &str) -> Result<String, String> {
        let arguments: serde_json::Value =
            serde_json::from_str(arguments).map_err(|error| format!("its arguments aren't JSON: {error}"))?;
        let called = self.mcp_server().ask(scope, "tools/call", serde_json::json!({"name": tool, "arguments": arguments}))?;
        Ok(called.to_string())
    }

    fn reach_resolve(&self, _scope: &Scope, path: &str, writable: bool) -> Result<Folder, String> {
        let dir = crate::sandbox::extra_dir(path, writable, silt_os::home_dir().as_deref());
        if let Some(why) = crate::sandbox::folder_refusal(&self.data_dir, &dir) {
            return Err(format!("{why}: it's never handed over"));
        }
        let real = dir.real.as_ref().ok_or_else(|| format!("{path} isn't there"))?;
        if !real.is_dir() {
            return Err(format!("{path} isn't a folder"));
        }
        let repos = dir
            .repos
            .unwrap_or_default()
            .into_iter()
            .map(|repo| repo.work_tree.unwrap_or(repo.git_dir).to_string_lossy().into_owned())
            .collect();
        Ok(Folder { asked: path.to_owned(), real: real.to_string_lossy().into_owned(), repos })
    }

    fn reach_grant(&self, scope: &Scope, real: &str, writable: bool) -> Result<(), String> {
        let to = self.folders.clone().ok_or("a folder is granted only by a call that runs")?;
        let grant = crate::folders::FolderGrant { path: real.to_owned(), writable, scope: to, granted_ms: crate::core::now_ms() };
        crate::folders::record(&self.data_dir, &scope.thread, grant).map_err(|error| error.to_string())
    }
}
