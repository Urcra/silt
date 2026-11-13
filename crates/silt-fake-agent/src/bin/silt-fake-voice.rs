//! A stand-in for `silt-voice`. `download` makes the model directory.
//! `listen` says a fixed phrase once audio arrives, and:
//! - `FAKE_VOICE_SILENT` hears no words at all;
//! - `FAKE_VOICE_PAUSE` finishes the phrase and pauses without waiting for
//!   the audio to end;
//! - `FAKE_VOICE_CRASH` dies on the first audio;
//! - `FAKE_VOICE_PID` names a file to write its process id to.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

const SEGMENT: &str = r#"{"kind":"segment","text":"Hello there."}"#;

fn say(line: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn listen() -> ExitCode {
    if let Some(file) = env::var_os("FAKE_VOICE_PID") {
        let _ = fs::write(file, std::process::id().to_string());
    }
    let silent = env::var_os("FAKE_VOICE_SILENT").is_some();
    let pause = env::var_os("FAKE_VOICE_PAUSE").is_some();
    let mut stdin = io::stdin().lock();
    let mut buffer = [0; 4096];
    let mut heard = false;
    let mut finished_phrase = false;
    while stdin.read(&mut buffer).is_ok_and(|read| read > 0) {
        if env::var_os("FAKE_VOICE_CRASH").is_some() {
            return ExitCode::from(3);
        }
        if !heard && !silent {
            say(r#"{"kind":"partial","text":"hello there"}"#);
        }
        if !heard && !silent && pause {
            say(SEGMENT);
            say(r#"{"kind":"paused"}"#);
            finished_phrase = true;
        }
        heard = true;
    }
    if heard && !silent && !finished_phrase {
        say(SEGMENT);
    }
    say(r#"{"kind":"done"}"#);
    ExitCode::SUCCESS
}

fn download(to: &str) -> ExitCode {
    if env::var_os("FAKE_VOICE_DOWNLOAD_FAILS").is_some() {
        eprintln!("silt-voice: the network is down");
        return ExitCode::FAILURE;
    }
    say(r#"{"done":1,"total":2}"#);
    if fs::create_dir_all(to).is_err() {
        return ExitCode::FAILURE;
    }
    say(r#"{"done":2,"total":2}"#);
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[1..] {
        ["listen", "--model", _] => listen(),
        ["download", "--to", to] => download(to),
        _ => ExitCode::from(2),
    }
}
