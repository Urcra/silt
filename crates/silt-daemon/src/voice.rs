//! Dictation for the web UI (ADR 0061). The engine lives in a separate
//! program, `silt-voice`, run once per take: the daemon only passes audio in
//! and results out, and never keeps either.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Semaphore, mpsc};

use crate::paths::data_dir;
use crate::protocol::{VoiceDownload, VoiceEvent, VoiceStatus};

const MODEL_DIR: &str = "zipformer-en-2023-06-26";
const SAMPLE_BYTES: usize = 4;
const SAMPLES_PER_SECOND: usize = 16_000;
const LONGEST_TAKE: Duration = Duration::from_secs(5 * 60);
const MOST_TAKE_BYTES: usize = SAMPLE_BYTES * SAMPLES_PER_SECOND * LONGEST_TAKE.as_secs() as usize;

/// One frame is a fraction of a second; this leaves plenty of room.
pub(crate) const MOST_FRAME_BYTES: usize = 1 << 16;

/// The UI's word for "that was all the audio".
const FINISH: &str = "finish";

pub(crate) struct Voice {
    taking: Semaphore,
    fetching: Mutex<Fetching>,
}

#[derive(Default)]
struct Fetching {
    running: bool,
    progress: Option<VoiceDownload>,
    failed: Option<String>,
}

impl Voice {
    pub(crate) fn new() -> Self {
        Self { taking: Semaphore::new(1), fetching: Mutex::default() }
    }

    pub(crate) fn status(&self) -> VoiceStatus {
        let fetching = self.fetching.lock().expect("lock");
        VoiceStatus {
            helper: helper().is_some(),
            model: model_dir().is_dir(),
            download: fetching.progress.clone(),
            failed: fetching.failed.clone(),
        }
    }

    /// Starts fetching the model, unless that's under way or done.
    pub(crate) fn start_download(self: &Arc<Self>) -> VoiceStatus {
        let should_run = {
            let mut fetching = self.fetching.lock().expect("lock");
            let idle = !fetching.running && !model_dir().is_dir();
            if idle {
                fetching.running = true;
                fetching.failed = None;
                fetching.progress = None;
            }
            idle
        };
        if should_run {
            let voice = self.clone();
            tokio::spawn(async move {
                let result = voice.download().await;
                let mut fetching = voice.fetching.lock().expect("lock");
                fetching.running = false;
                fetching.progress = None;
                fetching.failed = result.err();
            });
        }
        self.status()
    }

    async fn download(&self) -> Result<(), String> {
        let helper = helper().ok_or_else(no_helper)?;
        let mut child = Command::new(helper)
            .arg("download")
            .arg("--to")
            .arg(model_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("could not run silt-voice: {error}"))?;
        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Ok(progress) = serde_json::from_str::<VoiceDownload>(&line) {
                self.fetching.lock().expect("lock").progress = Some(progress);
            }
        }
        let output = child.wait_with_output().await.map_err(|error| error.to_string())?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed");
        Err(reason.strip_prefix("silt-voice: ").unwrap_or(reason).to_owned())
    }
}

fn model_dir() -> PathBuf {
    data_dir().join("models").join(MODEL_DIR)
}

fn no_helper() -> String {
    "silt-voice isn't installed".to_owned()
}

fn helper() -> Option<PathBuf> {
    find_helper(env::var_os("SILT_VOICE_BIN"), env::current_exe().ok().as_deref(), env::var_os("PATH"), |path| path.is_file())
}

/// `$SILT_VOICE_BIN`, else the one beside this program, else the `PATH`'s.
fn find_helper(
    chosen: Option<OsString>,
    exe: Option<&Path>,
    path: Option<OsString>,
    is_file: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(chosen) = chosen.filter(|chosen| !chosen.is_empty()) {
        return Some(PathBuf::from(chosen)).filter(|chosen| is_file(chosen));
    }
    let name = format!("silt-voice{}", env::consts::EXE_SUFFIX);
    let beside = exe.and_then(Path::parent).map(Path::to_path_buf);
    let on_path = path.iter().flat_map(env::split_paths).collect::<Vec<_>>();
    beside.into_iter().chain(on_path).map(|dir| dir.join(&name)).find(|candidate| is_file(candidate))
}

/// Whether `frame` is whole samples of finite audio.
fn valid_frame(frame: &[u8]) -> bool {
    !frame.is_empty()
        && frame.len().is_multiple_of(SAMPLE_BYTES)
        && frame.as_chunks::<SAMPLE_BYTES>().0.iter().all(|sample| f32::from_le_bytes(*sample).is_finite())
}

async fn send(socket: &mut WebSocket, event: &VoiceEvent) -> bool {
    let text = serde_json::to_string(event).expect("an event is plain data");
    socket.send(Message::Text(text.into())).await.is_ok()
}

/// Serves one take on `socket`: audio frames in, [`VoiceEvent`]s out.
pub(crate) async fn dictate(mut socket: WebSocket, voice: Arc<Voice>) {
    if let Err(message) = take(&mut socket, &voice).await {
        send(&mut socket, &VoiceEvent::Failed { message }).await;
    }
}

async fn take(socket: &mut WebSocket, voice: &Voice) -> Result<(), String> {
    let _only = voice.taking.try_acquire().map_err(|_| "another dictation is under way".to_owned())?;
    let helper = helper().ok_or_else(no_helper)?;
    let mut child = Command::new(helper)
        .arg("listen")
        .arg("--model")
        .arg(model_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("could not run silt-voice: {error}"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
    // Writing apart from reading, so a full pipe either way can't stop both.
    let (sender, mut frames) = mpsc::unbounded_channel::<Bytes>();
    tokio::spawn(async move {
        while let Some(frame) = frames.recv().await {
            if stdin.write_all(&frame).await.is_err() {
                break;
            }
        }
    });
    let mut audio = Some(sender);
    let mut heard = 0;
    let deadline = tokio::time::sleep(LONGEST_TAKE);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(frame))) => {
                    if !valid_frame(&frame) {
                        return Err("the audio wasn't understood".to_owned());
                    }
                    heard += frame.len();
                    if heard > MOST_TAKE_BYTES {
                        audio = None;
                    } else if let Some(audio) = &audio {
                        let _ = audio.send(frame);
                    }
                }
                Some(Ok(Message::Text(text))) if text.as_str() == FINISH => audio = None,
                Some(Ok(_)) => {}
                // Gone: dropping the child ends it.
                Some(Err(_)) | None => return Ok(()),
            },
            line = lines.next_line() => match line {
                Ok(Some(line)) => {
                    let Ok(event) = serde_json::from_str::<VoiceEvent>(&line) else {
                        continue;
                    };
                    let done = event == VoiceEvent::Done;
                    if !send(socket, &event).await || done {
                        return Ok(());
                    }
                }
                Ok(None) | Err(_) => return Err("the speech helper stopped unexpectedly".to_owned()),
            },
            () = &mut deadline, if audio.is_some() => audio = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|sample| sample.to_le_bytes()).collect()
    }

    #[test]
    fn only_whole_finite_samples_are_audio() {
        assert!(valid_frame(&frame(&[0.0, 0.5, -1.0])));
        assert!(!valid_frame(&[]));
        assert!(!valid_frame(&[0, 0, 0]));
        assert!(!valid_frame(&frame(&[0.0, f32::NAN])));
        assert!(!valid_frame(&frame(&[f32::INFINITY])));
    }

    #[test]
    fn the_helper_is_the_chosen_one_then_the_one_beside_then_the_paths() {
        let exe = Path::new("/opt/silt/silt");
        let name = format!("silt-voice{}", env::consts::EXE_SUFFIX);
        let beside = Path::new("/opt/silt").join(&name);
        let path = env::join_paths(["/usr/bin"]).expect("a path");
        let on_path = Path::new("/usr/bin").join(&name);
        let everywhere = |_: &Path| true;
        let nowhere = |_: &Path| false;
        assert_eq!(find_helper(Some("/x/v".into()), Some(exe), None, everywhere), Some(PathBuf::from("/x/v")));
        assert_eq!(
            find_helper(Some("/x/v".into()), Some(exe), None, nowhere),
            None,
            "a chosen helper that's missing stays missing"
        );
        assert_eq!(find_helper(Some("".into()), Some(exe), None, everywhere), Some(beside));
        assert_eq!(find_helper(None, None, Some(path.clone()), everywhere), Some(on_path));
        assert_eq!(find_helper(None, Some(exe), Some(path), nowhere), None);
    }

    #[test]
    fn events_are_read_as_the_helper_writes_them() {
        assert_eq!(
            serde_json::from_str::<VoiceEvent>(r#"{"kind":"segment","text":"Hi."}"#).expect("an event"),
            VoiceEvent::Segment { text: "Hi.".into() }
        );
        assert_eq!(serde_json::from_str::<VoiceEvent>(r#"{"kind":"paused"}"#).expect("an event"), VoiceEvent::Paused);
    }
}
