//! A real `silt` daemon in a scratch data dir, driven as people run it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use serde_json::Value;
use silt_daemon::client;
use silt_daemon::protocol::{Request, Response};

use crate::{BenchError, fail};

const READY_TIMEOUT: Duration = Duration::from_secs(120);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// Where one run keeps its data and its agent's sessions.
pub struct Home {
    pub silt: PathBuf,
    pub data: PathBuf,
    pub claude: PathBuf,
}

impl Home {
    pub fn new(silt: &Path, root: &Path) -> Self {
        Self { silt: silt.to_owned(), data: root.join("silt"), claude: root.join("claude") }
    }

    pub fn socket(&self) -> PathBuf {
        silt_daemon::paths::socket_path_in(&self.data)
    }

    pub fn command(&self, cwd: &Path) -> Command {
        let mut command = Command::new(&self.silt);
        command.current_dir(cwd).env("SILT_HOME", &self.data).env("CLAUDE_CONFIG_DIR", &self.claude);
        command
    }

    /// Runs `silt <args>` in `cwd`, returning what it printed.
    pub fn silt(&self, cwd: &Path, args: &[&str]) -> Result<String, Exn<BenchError>> {
        let output = self.command(cwd).args(args).output().or_raise(|| BenchError(format!("cannot run silt {}", args[0])))?;
        if !output.status.success() {
            return Err(fail(format!("silt {} failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn request(&self, request: &Request) -> Result<Value, Exn<BenchError>> {
        let response = client::send(&self.socket(), request, None).or_raise(|| BenchError("the daemon didn't answer".into()))?;
        match response {
            Response::Ok { data } => Ok(data),
            Response::Error { message } => Err(fail(format!("the daemon refused: {message}"))),
        }
    }

    /// Starts `silt daemon run`, returning once it has started, not once it
    /// answers.
    pub fn spawn(&self) -> Result<Daemon, Exn<BenchError>> {
        fs::create_dir_all(&self.data).or_raise(|| BenchError("cannot make the data dir".into()))?;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.data.join("bench-daemon.log"))
            .or_raise(|| BenchError("cannot open the daemon log".into()))?;
        let log_err = log.try_clone().or_raise(|| BenchError("cannot open the daemon log".into()))?;
        let child = self
            .command(Path::new("/"))
            .args(["daemon", "run"])
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            .spawn()
            .or_raise(|| BenchError("cannot start the daemon".into()))?;
        Ok(Daemon { child: Some(child), socket: self.socket() })
    }

    /// Starts the daemon and waits until it answers.
    pub fn start(&self) -> Result<Daemon, Exn<BenchError>> {
        let mut daemon = self.spawn()?;
        daemon.ready()?;
        Ok(daemon)
    }
}

pub struct Daemon {
    child: Option<Child>,
    socket: PathBuf,
}

impl Daemon {
    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("running").id()
    }

    /// Waits for the first answer to a ping; how long since `spawn` that
    /// took is the caller's to measure.
    pub fn ready(&mut self) -> Result<(), Exn<BenchError>> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if client::send(&self.socket, &Request::Ping, Some(Duration::from_secs(5))).is_ok() {
                return Ok(());
            }
            if let Some(status) = self.child.as_mut().and_then(|child| child.try_wait().ok().flatten()) {
                return Err(fail(format!("the daemon exited with {status}")));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(fail("the daemon never answered"))
    }

    /// Asks the daemon to shut down and waits for it to exit.
    pub fn stop(mut self) -> Result<(), Exn<BenchError>> {
        self.shut_down()
    }

    fn shut_down(&mut self) -> Result<(), Exn<BenchError>> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        let _ = client::send(&self.socket, &Request::Shutdown, Some(STOP_TIMEOUT));
        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        Err(fail("the daemon didn't stop when asked"))
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.shut_down();
    }
}
