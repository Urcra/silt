//! `sync.push` and `sync.pull`: handing a long session to another machine
//! through a server. One machine pushes the family holding the 100 MB
//! transcript with `silt sync --once`; another pulls it with `silt resume`.
//! Then `app.threads` and `app.history`: the server's hosted app listing
//! threads and opening that session, as a browser signed in to it does.

use std::fs;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use exn::{Exn, ResultExt};
use silt_daemon::protocol::{Request, ThreadSummary};

use crate::budgets::{APP_HISTORY, APP_THREADS, Measured, SYNC_PULL, SYNC_PUSH};
use crate::daemon::Home;
use crate::fixtures::{Fixtures, SESSION_ID};
use crate::measure::daemon::{place, recorded, session_transcript};
use crate::{BenchError, fail};

const LISTEN_TIMEOUT: Duration = Duration::from_secs(30);
const LISTING_TIMEOUT: Duration = Duration::from_secs(60);
const BROWSER_PASSWORD: &str = "correct horse battery staple";

pub async fn measure(silt: &Path, root: &Path, fixtures: &Fixtures) -> Result<Vec<(&'static str, Measured)>, Exn<BenchError>> {
    let transcript = fixtures.transcript().or_raise(|| BenchError("cannot write the transcript".into()))?;
    let bytes = fs::metadata(&transcript).or_raise(|| BenchError("cannot read the transcript".into()))?.len();
    let server = Server::start(silt, &root.join("server"))?;
    let origin = origin(&root.join("origin"))?;

    let laptop = Home::new(silt, &root.join("laptop"));
    let repo = clone(&origin, &root.join("laptop-repo"))?;
    server.sign_in(&laptop, &repo, "laptop")?;
    place(&transcript, &session_transcript(&laptop, &repo, SESSION_ID))?;
    let daemon = laptop.start()?;
    let tracked = laptop.silt(&repo, &["track"])?;
    recorded(&laptop, SESSION_ID, bytes).await?;
    daemon.stop()?;
    let thread = tracked.split_whitespace().last().ok_or_else(|| fail(format!("no thread in {tracked:?}")))?.to_owned();
    let start = Instant::now();
    laptop.silt(&repo, &["sync", "--once"])?;
    let pushed = start.elapsed();

    let home = Home::new(silt, &root.join("home"));
    let home_repo = clone(&origin, &root.join("home-repo"))?;
    server.sign_in(&home, &home_repo, "home")?;
    let daemon = home.start()?;
    listed(&home, &thread)?;
    let agent = quiet_agent(root)?;
    let start = Instant::now();
    let resumed = home
        .command(&home_repo)
        .args(["resume", &thread])
        .env("SILT_CLAUDE_BIN", &agent)
        .output()
        .or_raise(|| BenchError("cannot run silt resume".into()))?;
    let pulled = start.elapsed();
    if !resumed.status.success() {
        return Err(fail(format!("silt resume failed: {}", String::from_utf8_lossy(&resumed.stderr).trim())));
    }
    daemon.stop()?;

    let app = server.browser("owner").await?;
    let start = Instant::now();
    let threads: Vec<ThreadSummary> = app.get("/app/api/threads").await?;
    let app_threads = start.elapsed();
    let id = threads
        .iter()
        .find(|summary| summary.id.ends_with(&thread))
        .ok_or_else(|| fail(format!("the hosted app doesn't list thread {thread}")))?
        .id
        .clone();
    let start = Instant::now();
    app.get::<serde_json::Value>(&format!("/app/api/threads/{id}/history?offset=0&limit=50")).await?;
    let app_history = start.elapsed();
    Ok(vec![
        (SYNC_PUSH, Measured::Time(pushed)),
        (SYNC_PULL, Measured::Time(pulled)),
        (APP_THREADS, Measured::Time(app_threads)),
        (APP_HISTORY, Measured::Time(app_history)),
    ])
}

/// The hosted app as a browser signed in to it uses it.
struct Browser {
    client: reqwest::Client,
    url: String,
    cookie: String,
}

impl Browser {
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Exn<BenchError>> {
        let failed = || BenchError(format!("the hosted app didn't answer {path}"));
        let response =
            self.client.get(format!("{}{path}", self.url)).header("cookie", &self.cookie).send().await.or_raise(failed)?;
        if !response.status().is_success() {
            return Err(fail(format!("the hosted app refused {path}: {}", response.status())));
        }
        response.json().await.or_raise(failed)
    }
}

/// Waits until `home` lists `thread`, which its daemon learns from the
/// server's listing on its own time; that wait isn't a transfer, so it isn't
/// timed.
fn listed(home: &Home, thread: &str) -> Result<(), Exn<BenchError>> {
    let deadline = Instant::now() + LISTING_TIMEOUT;
    while Instant::now() < deadline {
        let threads: Vec<ThreadSummary> = serde_json::from_value(home.request(&Request::Threads)?)
            .or_raise(|| BenchError("cannot read the thread listing".into()))?;
        if threads.iter().any(|summary| summary.id.ends_with(thread)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(fail(format!("the other machine never listed thread {thread}")))
}

/// A server on a free port, stopped when dropped.
struct Server {
    silt: PathBuf,
    data: PathBuf,
    url: String,
    child: Child,
}

impl Server {
    fn start(silt: &Path, data: &Path) -> Result<Self, Exn<BenchError>> {
        let failed = || BenchError("cannot start the server".into());
        let port = TcpListener::bind("127.0.0.1:0").or_raise(failed)?.local_addr().or_raise(failed)?.port();
        let listen = format!("127.0.0.1:{port}");
        fs::create_dir_all(data).or_raise(failed)?;
        let log_path = data.with_extension("log");
        let log = fs::File::create(&log_path).or_raise(failed)?;
        let child = Command::new(silt)
            .args(["server", "run", "--listen", &listen, "--data"])
            .arg(data)
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .or_raise(failed)?;
        let server = Self { silt: silt.to_owned(), data: data.to_owned(), url: format!("http://{listen}"), child };
        let deadline = Instant::now() + LISTEN_TIMEOUT;
        while Instant::now() < deadline {
            if fs::read_to_string(&log_path).is_ok_and(|log| log.contains(&format!("listening on {listen}"))) {
                return Ok(server);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Err(fail(format!("the server never listened: {}", fs::read_to_string(&log_path).unwrap_or_default())))
    }

    /// Signs a browser in to the hosted app as `user`, with a password set
    /// for it here.
    async fn browser(&self, user: &str) -> Result<Browser, Exn<BenchError>> {
        let failed = || BenchError("cannot sign in to the hosted app".into());
        let mut passwd = Command::new(&self.silt)
            .args(["server", "user", "passwd", user, "--password-stdin", "--data"])
            .arg(&self.data)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .or_raise(failed)?;
        std::io::Write::write_all(&mut passwd.stdin.take().expect("piped"), format!("{BROWSER_PASSWORD}\n").as_bytes())
            .or_raise(failed)?;
        if !passwd.wait().or_raise(failed)?.success() {
            return Err(fail(format!("cannot set {user}'s password")));
        }
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().or_raise(failed)?;
        let signed_in = client
            .post(format!("{}/login", self.url))
            .form(&[("user", user), ("password", BROWSER_PASSWORD), ("next", "/app/")])
            .send()
            .await
            .or_raise(failed)?;
        let cookie = signed_in
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok()?.split(';').next())
            .find(|cookie| cookie.starts_with("silt_session="))
            .ok_or_else(|| fail(format!("signing in as {user} started no session")))?
            .to_owned();
        Ok(Browser { client, url: self.url.clone(), cookie })
    }

    /// Signs `home` in to this server as its owner, under device `name`.
    fn sign_in(&self, home: &Home, cwd: &Path, name: &str) -> Result<(), Exn<BenchError>> {
        let paired = Command::new(&self.silt)
            .args(["server", "pair", "--user", "owner", "--data"])
            .arg(&self.data)
            .output()
            .or_raise(|| BenchError("cannot pair a device".into()))?;
        if !paired.status.success() {
            return Err(fail(format!("pairing failed: {}", String::from_utf8_lossy(&paired.stderr).trim())));
        }
        let code = String::from_utf8_lossy(&paired.stdout).trim().to_owned();
        home.silt(cwd, &["auth", "login", "--server", &self.url, "--code", &code, "--name", name])?;
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A bare repository with one commit, for both machines to clone.
fn origin(at: &Path) -> Result<PathBuf, Exn<BenchError>> {
    let failed = || BenchError("cannot make the origin repository".into());
    let seed = at.with_extension("seed");
    fs::create_dir_all(seed.join("src")).or_raise(failed)?;
    fs::write(seed.join("src/main.rs"), "fn main() {}\n").or_raise(failed)?;
    git(&seed, &["init", "-q", "-b", "main"])?;
    git(&seed, &["add", "."])?;
    git(&seed, &["-c", "user.name=bench", "-c", "user.email=bench@example.com", "commit", "-q", "-m", "init"])?;
    git(Path::new("/"), &["clone", "-q", "--bare", &seed.to_string_lossy(), &at.to_string_lossy()])?;
    Ok(at.to_owned())
}

fn clone(origin: &Path, to: &Path) -> Result<PathBuf, Exn<BenchError>> {
    git(Path::new("/"), &["clone", "-q", &origin.to_string_lossy(), &to.to_string_lossy()])?;
    to.canonicalize().or_raise(|| BenchError("cannot resolve the clone".into()))
}

fn git(dir: &Path, args: &[&str]) -> Result<(), Exn<BenchError>> {
    let status = Command::new("git").args(args).current_dir(dir).status().or_raise(|| BenchError("cannot run git".into()))?;
    if status.success() { Ok(()) } else { Err(fail(format!("git {} failed", args.join(" ")))) }
}

/// An agent `silt resume` starts once it has pulled, which exits at once.
fn quiet_agent(root: &Path) -> Result<PathBuf, Exn<BenchError>> {
    let failed = || BenchError("cannot write the agent".into());
    let path = root.join("claude-bin");
    fs::write(&path, "#!/bin/sh\nexit 0\n").or_raise(failed)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).or_raise(failed)?;
    Ok(path)
}
