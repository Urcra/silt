//! Dictation through a real daemon, with `silt-fake-voice` in place of the
//! speech helper (ADR 0061).

mod common;

use std::process::Command;
use std::time::Duration;

use common::{fake_agent, silt_command, temp_root, web_ui};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Daemon {
    root: tempfile::TempDir,
    base: String,
    token: String,
}

impl Daemon {
    fn start(envs: &[(&str, &str)]) -> Self {
        Self::with_helper(&fake_agent("voice").to_string_lossy(), envs)
    }

    fn with_helper(helper: &str, envs: &[(&str, &str)]) -> Self {
        let root = temp_root("silt-voice");
        let output = silt_command()
            .args(["open", "--no-browser"])
            .current_dir(root.path())
            .env("SILT_HOME", root.path().join("silt"))
            .env("SILT_VOICE_BIN", helper)
            .envs(envs.iter().copied())
            .output()
            .expect("silt runs");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let url = String::from_utf8_lossy(&output.stdout).into_owned();
        let (base, token) = web_ui(&url);
        let (base, token) = (base.to_owned(), token.to_owned());
        Self { root, base, token }
    }

    fn call(&self, method: &str, path: &str, token: &str) -> (String, String) {
        let output = Command::new("curl")
            .args(["-s", "-w", "\n%{http_code}", "-X", method, "-H"])
            .arg(format!("Authorization: Bearer {token}"))
            .arg(format!("{}{path}", self.base))
            .output()
            .expect("curl runs");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (body, status) = text.rsplit_once('\n').expect("a status line");
        (status.to_owned(), body.to_owned())
    }

    fn status(&self) -> Value {
        let (status, body) = self.call("GET", "/api/voice", &self.token);
        assert_eq!(status, "200", "{body}");
        serde_json::from_str(&body).expect("JSON")
    }

    fn fetch_model(&self) -> Value {
        let (status, _) = self.call("POST", "/api/voice/model", &self.token);
        assert_eq!(status, "200");
        wait_for(|| {
            let now = self.status();
            (now["model"] == true || !now["failed"].is_null()).then_some(now)
        })
    }

    async fn dictate(&self) -> Socket {
        let url = format!("{}/api/voice/dictate?token={}", self.base.replacen("http", "ws", 1), self.token);
        tokio_tungstenite::connect_async(url).await.expect("connects").0
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = silt_command().args(["daemon", "stop"]).env("SILT_HOME", self.root.path().join("silt")).output();
    }
}

fn wait_for<T>(mut check: impl FnMut() -> Option<T>) -> T {
    for _ in 0..200 {
        if let Some(found) = check() {
            return found;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("it never happened");
}

fn on<T>(work: impl Future<Output = T>) -> T {
    tokio::runtime::Runtime::new().expect("runtime").block_on(work)
}

fn audio() -> Message {
    Message::binary([0.25f32; 64].iter().flat_map(|sample| sample.to_le_bytes()).collect::<Vec<u8>>())
}

async fn next(socket: &mut Socket) -> Value {
    let waited = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(Ok(message)) = socket.next().await {
            if let Ok(text) = message.to_text()
                && !text.is_empty()
            {
                return serde_json::from_str(text).expect("events are JSON");
            }
        }
        panic!("the socket closed");
    });
    waited.await.expect("an event came")
}

#[test]
fn the_voice_routes_need_the_token() {
    let daemon = Daemon::start(&[]);
    assert_eq!(daemon.call("GET", "/api/voice", "wrong").0, "401");
    assert_eq!(daemon.call("POST", "/api/voice/model", "wrong").0, "401");
    on(async {
        let url = format!("{}/api/voice/dictate?token=wrong", daemon.base.replacen("http", "ws", 1));
        assert!(tokio_tungstenite::connect_async(url).await.is_err());
    });
}

#[test]
fn a_helper_that_isnt_there_is_said_to_be_missing() {
    let daemon = Daemon::with_helper("/nowhere/silt-voice", &[]);
    assert_eq!(daemon.status()["helper"], false);
    assert_eq!(daemon.fetch_model()["failed"], "silt-voice isn't installed");
    on(async {
        let mut socket = daemon.dictate().await;
        let event = next(&mut socket).await;
        assert_eq!(event["kind"], "failed", "{event}");
    });
}

#[test]
fn the_model_is_fetched_only_when_asked_for() {
    let daemon = Daemon::start(&[]);
    let before = daemon.status();
    assert_eq!((&before["helper"], &before["model"]), (&Value::Bool(true), &Value::Bool(false)));
    assert_eq!(daemon.fetch_model()["model"], true);
}

#[test]
fn a_failed_download_says_why() {
    let daemon = Daemon::start(&[("FAKE_VOICE_DOWNLOAD_FAILS", "1")]);
    let answer = daemon.fetch_model();
    assert_eq!(answer["model"], false);
    assert_eq!(answer["failed"], "the network is down");
}

#[test]
fn a_take_says_its_words_as_they_come_and_ends_when_told() {
    let daemon = Daemon::start(&[]);
    on(async {
        let mut socket = daemon.dictate().await;
        socket.send(audio()).await.expect("sends");
        assert_eq!(next(&mut socket).await["text"], "hello there");
        socket.send(Message::text("finish")).await.expect("sends");
        assert_eq!(next(&mut socket).await["text"], "Hello there.");
        assert_eq!(next(&mut socket).await["kind"], "done");
    });
}

#[test]
fn silence_past_a_phrase_is_passed_on() {
    let daemon = Daemon::start(&[("FAKE_VOICE_PAUSE", "1")]);
    on(async {
        let mut socket = daemon.dictate().await;
        socket.send(audio()).await.expect("sends");
        next(&mut socket).await;
        assert_eq!(next(&mut socket).await["kind"], "segment");
        assert_eq!(next(&mut socket).await["kind"], "paused");
    });
}

#[test]
fn a_helper_that_dies_is_reported() {
    let daemon = Daemon::start(&[("FAKE_VOICE_CRASH", "1")]);
    on(async {
        let mut socket = daemon.dictate().await;
        socket.send(audio()).await.expect("sends");
        let event = next(&mut socket).await;
        assert_eq!(event["kind"], "failed", "{event}");
    });
}

#[test]
fn audio_that_isnt_samples_ends_the_take() {
    let daemon = Daemon::start(&[]);
    on(async {
        let mut socket = daemon.dictate().await;
        socket.send(Message::binary(vec![1u8, 2, 3])).await.expect("sends");
        let event = next(&mut socket).await;
        assert_eq!(event["kind"], "failed", "{event}");
    });
}

#[test]
fn one_take_at_a_time_and_closing_frees_the_next() {
    let daemon = Daemon::start(&[]);
    on(async {
        let mut first = daemon.dictate().await;
        first.send(audio()).await.expect("sends");
        next(&mut first).await;
        let mut second = daemon.dictate().await;
        let event = next(&mut second).await;
        assert_eq!(event["message"], "another dictation is under way", "{event}");
        drop(first);
        for _ in 0..100 {
            let mut third = daemon.dictate().await;
            third.send(audio()).await.expect("sends");
            if next(&mut third).await["kind"] == "partial" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the closed take never freed the helper");
    });
}

#[cfg(unix)]
#[test]
fn closing_the_socket_ends_the_helper() {
    let pids = temp_root("silt-voice-pid");
    let pid_file = pids.path().join("pid");
    let daemon = Daemon::start(&[("FAKE_VOICE_PID", &pid_file.to_string_lossy())]);
    on(async {
        let mut socket = daemon.dictate().await;
        socket.send(audio()).await.expect("sends");
        next(&mut socket).await;
        drop(socket);
    });
    let pid = std::fs::read_to_string(&pid_file).expect("the helper ran");
    wait_for(|| {
        let alive = Command::new("kill").args(["-0", &pid]).output().expect("kill runs").status.success();
        (!alive).then_some(())
    });
}
