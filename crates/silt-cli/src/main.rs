mod commands;
mod dev_status;
mod render;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use exn::Exn;

#[derive(Debug, Parser)]
#[command(name = "silt", version, about = "Resume, fork and search your agent sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run Claude Code with Silt recording the session and every file edit
    Claude {
        /// Work in a managed checkout instead of the current one
        #[arg(long)]
        isolated: bool,
        /// Arguments passed on to `claude`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Start recording the most recent Claude Code session in this directory
    Track,
    /// List recorded threads
    Log,
    /// Show a thread's conversation and the files each tool call changed
    Show { thread: String },
    /// Show what a thread, or one of its turns, changed
    Diff {
        thread: String,
        /// Only the changes made in this turn (1-based)
        #[arg(long)]
        turn: Option<usize>,
    },
    /// Search conversations
    Search {
        #[arg(required = true)]
        query: Vec<String>,
    },
    /// Run or administer a self-hosted sync server
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
    /// Manage the background daemon
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Called by agent hooks; reads the hook input on stdin
    #[command(hide = true)]
    Hook { event: String },
    /// Commands for working on Silt itself
    Dev {
        #[command(subcommand)]
        command: DevCommand,
    },
}

#[derive(Debug, Subcommand)]
enum DaemonCommand {
    /// Run the daemon in the foreground
    Run,
    /// Start the daemon in the background if it isn't running
    Start,
    /// Stop the daemon
    Stop,
    /// Show whether the daemon is running
    Status,
}

#[derive(Debug, Subcommand)]
enum ServerCommand {
    /// Serve the sync API
    Run {
        /// Address to listen on
        #[arg(long, default_value = "127.0.0.1:7450")]
        listen: String,
        /// Server data directory; defaults to `server` in the Silt data directory
        #[arg(long)]
        data: Option<PathBuf>,
    },
    /// Print a one-time code a device uses with `silt auth login`
    Pair {
        #[arg(long)]
        data: Option<PathBuf>,
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
pub struct CliError(pub String);

fn main() -> Result<(), Exn<CliError>> {
    match Cli::parse().command {
        Command::Claude { isolated, args } => commands::claude(isolated, &args),
        Command::Track => commands::track(),
        Command::Log => commands::log(),
        Command::Show { thread } => commands::show(&thread),
        Command::Diff { thread, turn } => commands::diff(&thread, turn),
        Command::Search { query } => commands::search(&query.join(" ")),
        Command::Server { command } => match command {
            ServerCommand::Run { listen, data } => commands::server_run(&listen, data),
            ServerCommand::Pair { data } => commands::server_pair(data),
        },
        Command::Daemon { command } => match command {
            DaemonCommand::Run => commands::daemon_run(),
            DaemonCommand::Start => commands::ensure_daemon().map(|_| ()),
            DaemonCommand::Stop => commands::daemon_stop(),
            DaemonCommand::Status => commands::daemon_status(),
        },
        Command::Hook { event } => {
            commands::hook(&event);
            Ok(())
        }
        Command::Dev {
            command: DevCommand::Status { root },
        } => dev_status::print(root),
    }
}
