mod dev_status;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use exn::{Exn, ResultExt};

#[derive(Debug, Parser)]
#[command(name = "silt", version, about = "Resume, fork and search your agent sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Commands for working on Silt itself
    Dev {
        #[command(subcommand)]
        command: DevCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DevCommand {
    /// Show milestone and task progress from ROADMAP.md and TASKS.md
    Status {
        /// Repository root; defaults to the nearest ancestor containing ROADMAP.md
        #[arg(long)]
        root: Option<PathBuf>,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct CliError(String);

fn main() -> Result<(), Exn<CliError>> {
    match Cli::parse().command {
        Command::Dev {
            command: DevCommand::Status { root },
        } => print_dev_status(root),
    }
}

fn print_dev_status(root: Option<PathBuf>) -> Result<(), Exn<CliError>> {
    let root = match root {
        Some(root) => root,
        None => find_repo_root()?,
    };
    let roadmap = read(&root.join("ROADMAP.md"))?;
    let tasks = read(&root.join("TASKS.md"))?;
    print!(
        "{}",
        dev_status::render(&dev_status::parse_roadmap(&roadmap), &dev_status::parse_tasks(&tasks))
    );
    Ok(())
}

fn find_repo_root() -> Result<PathBuf, Exn<CliError>> {
    let cwd = env::current_dir().or_raise(|| CliError("cannot read current directory".into()))?;
    cwd.ancestors()
        .find(|dir| dir.join("ROADMAP.md").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| Exn::new(CliError("no ROADMAP.md found in any parent directory".into())))
}

fn read(path: &Path) -> Result<String, Exn<CliError>> {
    fs::read_to_string(path).or_raise(|| CliError(format!("cannot read {}", path.display())))
}
