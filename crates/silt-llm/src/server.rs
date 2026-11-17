//! Running llama.cpp's `llama-server` (ADR 0092): found rather than linked,
//! on loopback with a key drawn at each start, and stopped with everything
//! it started.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

use crate::{Device, LlmError, parse_devices};

/// Lines of the server's output kept, to say why it failed.
const KEPT_LINES: usize = 20;
const HEALTH_EVERY: Duration = Duration::from_millis(250);

/// `$SILT_LLAMA_SERVER` if set, else the `PATH`'s.
pub fn find_server(chosen: Option<OsString>, path: Option<&OsStr>, is_file: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(chosen) = chosen.filter(|chosen| !chosen.is_empty()) {
        let chosen = PathBuf::from(chosen);
        return is_file(&chosen).then_some(chosen);
    }
    let name = format!("llama-server{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path?).map(|dir| dir.join(&name)).find(|candidate| is_file(candidate))
}

/// What a start passes `llama-server`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub weights: PathBuf,
    pub context: u32,
    pub flags: Vec<String>,
    pub port: u16,
    pub key: String,
    /// Where the key is written for the server to read, which other users
    /// can't, rather than on its command line, which they can.
    pub key_file: PathBuf,
}

impl Launch {
    pub fn args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["-m".into(), self.weights.clone().into()];
        for arg in ["--jinja", "-ngl", "999", "--host", "127.0.0.1"] {
            args.push(arg.into());
        }
        args.extend(["-c".into(), self.context.to_string().into()]);
        args.extend(["--port".into(), self.port.to_string().into()]);
        args.extend(["--api-key-file".into(), self.key_file.clone().into()]);
        args.extend(self.flags.iter().map(OsString::from));
        args
    }
}

/// A free port on loopback, for the server to take.
pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// The devices `program` can use.
pub async fn list_devices(program: &Path) -> Result<Vec<Device>, LlmError> {
    let mut command = Command::new(program);
    command.arg("--list-devices").stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| LlmError::Start("llama-server --list-devices didn't answer".to_owned()))?
        .map_err(|error| LlmError::Start(format!("could not run {}: {error}", program.display())))?;
    let text = format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    Ok(parse_devices(&text))
}

/// A running `llama-server`.
#[derive(Debug)]
pub struct Server {
    child: Child,
    pub port: u16,
    pub key: String,
    log: Arc<Mutex<VecDeque<String>>>,
}

impl Server {
    /// Starts `program` and waits until it answers `/health`, which it does
    /// only once the model is loaded.
    pub async fn start(program: &Path, launch: &Launch, within: Duration) -> Result<Self, LlmError> {
        silt_os::write_private(&launch.key_file, launch.key.as_bytes())
            .map_err(|error| LlmError::Start(format!("could not keep the server's key: {error}")))?;
        silt_os::write_private(&launch.key_file, launch.key.as_bytes())
            .map_err(|error| LlmError::Start(format!("could not keep the server's key: {error}")))?;
        let mut command = Command::new(program);
        command.args(launch.args()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        silt_os::own_group(&mut command);
        let mut child =
            command.spawn().map_err(|error| LlmError::Start(format!("could not run {}: {error}", program.display())))?;
        let log = Arc::new(Mutex::new(VecDeque::new()));
        keep_lines(child.stdout.take(), log.clone());
        keep_lines(child.stderr.take(), log.clone());
        let mut server = Self { child, port: launch.port, key: launch.key.clone(), log };
        let client = reqwest::Client::builder().no_proxy().build().map_err(|error| LlmError::Start(error.to_string()))?;
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = server.child.try_wait() {
                return Err(LlmError::Start(format!("llama-server exited ({status}): {}", server.last_lines())));
            }
            let health = client.get(format!("{}/health", server.url())).timeout(Duration::from_secs(2)).send().await;
            if health.is_ok_and(|response| response.status().is_success()) {
                return Ok(server);
            }
            if started.elapsed() > within {
                server.stop().await;
                return Err(LlmError::Start("llama-server didn't load the model in time".to_owned()));
            }
            tokio::time::sleep(HEALTH_EVERY).await;
        }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Whether it has stopped by itself.
    pub fn has_exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
    }

    pub fn last_lines(&self) -> String {
        let lines = self.log.lock().expect("lock");
        lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    pub async fn stop(mut self) {
        if let Some(pid) = self.child.id() {
            silt_os::kill_tree(pid);
        }
        let _ = self.child.kill().await;
    }
}

fn keep_lines(stream: Option<impl AsyncRead + Unpin + Send + 'static>, log: Arc<Mutex<VecDeque<String>>>) {
    let Some(stream) = stream else {
        return;
    };
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut log = log.lock().expect("lock");
            if log.len() == KEPT_LINES {
                log.pop_front();
            }
            log.push_back(line);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chosen_server_wins_then_the_paths() {
        let path = std::env::join_paths(["/a", "/b"]).expect("joined");
        let name = format!("llama-server{}", std::env::consts::EXE_SUFFIX);
        let on_b = |candidate: &Path| candidate == Path::new("/b").join(&name);
        assert_eq!(find_server(None, Some(&path), on_b), Some(Path::new("/b").join(&name)));
        assert_eq!(find_server(Some("/c/llama".into()), Some(&path), on_b), None, "a chosen one that's missing isn't replaced");
        assert_eq!(find_server(Some("".into()), Some(&path), on_b), Some(Path::new("/b").join(&name)));
        assert_eq!(find_server(None, None, on_b), None);
    }

    #[test]
    fn a_launch_binds_loopback_with_its_key() {
        let launch = Launch {
            weights: "/m/model.gguf".into(),
            context: 32_768,
            flags: vec!["--cache-type-k".into(), "q8_0".into()],
            port: 18431,
            key: "k".into(),
            key_file: "/run/key".into(),
        };
        let args: Vec<String> = launch.args().into_iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(
            args.join(" "),
            "-m /m/model.gguf --jinja -ngl 999 --host 127.0.0.1 -c 32768 --port 18431 --api-key-file /run/key --cache-type-k q8_0"
        );
    }
}
