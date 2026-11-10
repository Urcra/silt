//! Silt's speech helper (ADR 0059). The daemon runs it for each dictation:
//! audio stays in pipes and memory, and a crash here can't reach the daemon.

mod listen;
mod model;
mod take;

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("could not download the speech model: {0}")]
    Request(String),
    #[error("could not download the speech model: {0}")]
    Fetch(io::Error),
    #[error("the downloaded file {0} isn't the one expected")]
    Corrupt(&'static str),
    #[error("could not write the speech model: {0}")]
    Disk(io::Error),
    #[error("could not load {0}")]
    Engine(&'static str),
    #[error("the daemon went away: {0}")]
    Pipe(io::Error),
}

#[derive(Parser)]
#[command(about = "Live dictation for Silt, on this machine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Transcribe 16 kHz mono f32le audio from stdin, writing JSON events
    Listen {
        #[arg(long)]
        model: PathBuf,
    },
    /// Download the speech models, writing JSON progress
    Download {
        #[arg(long)]
        to: PathBuf,
    },
}

fn main() -> ExitCode {
    let done = match Cli::parse().command {
        Command::Listen { model } => listen::run(&model, io::stdin().lock(), io::stdout().lock()),
        Command::Download { to } => model::download(&to, |done, total| {
            println!(r#"{{"done":{done},"total":{total}}}"#);
        }),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("silt-voice: {error}");
            ExitCode::FAILURE
        }
    }
}
