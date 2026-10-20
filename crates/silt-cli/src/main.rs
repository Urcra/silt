mod commands;
mod dev_status;
mod mcp;
mod render;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use exn::Exn;
use silt_daemon::protocol::SearchFilters;

#[derive(Debug, Parser)]
#[command(name = "silt", version, about = "Resume, fork and search your agent sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run an agent CLI (claude, codex, gemini, opencode, copilot) with Silt
    /// recording the session and every file edit
    Run {
        agent: String,
        #[command(flatten)]
        run: AgentRun,
    },
    /// Run Claude Code with Silt recording the session and every file edit
    Claude(AgentRun),
    /// Run Codex with Silt recording the session and every file edit
    Codex(AgentRun),
    /// Run Gemini CLI with Silt recording the session and every file edit
    Gemini(AgentRun),
    /// Run OpenCode with Silt recording the session and every file edit
    Opencode(AgentRun),
    /// Run GitHub Copilot CLI with Silt recording the session and every file edit
    Copilot(AgentRun),
    /// Start recording the most recent session of an agent in this directory
    Track {
        #[arg(long, default_value = "claude")]
        agent: String,
    },
    /// Continue a thread here: its code, its conversation and its agent session
    Resume {
        thread: String,
        #[command(flatten)]
        continued: Continue,
    },
    /// Fork a thread, yours or a teammate's, with its code and conversation
    Fork {
        thread: String,
        #[command(flatten)]
        continued: Continue,
    },
    /// Bring a fork's source's newer code and conversation into the fork
    Pull {
        /// The fork; defaults to the thread checked out here
        thread: Option<String>,
    },
    /// Merge requests from forks back to their source threads
    Mr {
        #[command(subcommand)]
        command: MrCommand,
    },
    /// Set who on the server can see this project's threads, or one thread family's
    Visibility {
        #[arg(value_enum)]
        visibility: VisibilityArg,
        /// Set it for one thread (and its forks that have no setting of their own)
        #[arg(long)]
        thread: Option<String>,
    },
    /// Connect this machine to a Silt server
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// List recorded threads
    Log,
    /// Show a thread's conversation and the files each tool call changed
    Show { thread: String },
    /// Show what a thread, or one of its turns, changed
    Diff {
        thread: String,
        /// Only the changes made in this turn (1-based)
        #[arg(long, conflicts_with = "base")]
        turn: Option<usize>,
        /// Diff from merge-base (with the fork's source), last-commit,
        /// last-turn or commit:<sha>; by default from the thread's start
        #[arg(long)]
        base: Option<String>,
    },
    /// Search conversations: your team's through the server when connected
    Search {
        #[arg(required = true)]
        query: Vec<String>,
        /// Repository, as its normalized origin URL (github.com/org/repo)
        #[arg(long)]
        repo: Option<String>,
        /// The owning user
        #[arg(long)]
        author: Option<String>,
        /// active, idle or merged
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        branch: Option<String>,
        /// Threads that changed this file, or anything under this directory
        #[arg(long)]
        file: Option<String>,
        /// YYYY-MM-DD, or a number of days ago such as 7d
        #[arg(long)]
        since: Option<String>,
        /// YYYY-MM-DD, or a number of days ago such as 7d
        #[arg(long)]
        until: Option<String>,
    },
    /// Open the web UI: threads, search, history, merge requests and comments
    Open {
        /// Print the address instead of opening a browser
        #[arg(long)]
        no_browser: bool,
    },
    /// A thread's full history as numbered entries, across compactions and forks
    History {
        thread: String,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 200)]
        limit: usize,
    },
    /// Comment on lines of a file or on a history entry of a thread
    Comment {
        thread: String,
        /// A file of the thread, with --lines
        #[arg(long, requires = "lines", conflicts_with = "entry")]
        file: Option<String>,
        /// A line or a range such as 10-14
        #[arg(long)]
        lines: Option<String>,
        /// A history entry, numbered as `silt history` shows them
        #[arg(long)]
        entry: Option<usize>,
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// List a thread's comments, or resolve one
    Comments {
        thread: String,
        #[arg(long)]
        resolve: Option<String>,
    },
    /// Other threads changing a file or directory of this repository
    InFlight {
        #[arg(default_value = ".")]
        path: String,
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
    Hook {
        /// The agent whose hook format the input and output use
        #[arg(long)]
        agent: Option<String>,
        event: String,
    },
    /// Serve Silt's history tools to an agent over MCP on stdio
    #[command(hide = true)]
    Mcp {
        #[arg(long)]
        thread: String,
        /// The checkout whose skills to offer; defaults to the current directory
        #[arg(long)]
        checkout: Option<PathBuf>,
    },
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
enum MrCommand {
    /// Offer a fork's work to the thread it was forked from
    Create {
        /// The fork; defaults to the thread checked out here
        #[arg(long)]
        thread: Option<String>,
        #[arg(long)]
        title: Option<String>,
        /// Defaults to the changed files and the fork agent's last message
        #[arg(long)]
        summary: Option<String>,
    },
    /// List merge requests, all or those of one thread
    List { thread: Option<String> },
    /// Show a merge request: summary, conflict hints, diff and the fork's conversation
    Show { mr: String },
    /// Merge it into its target (on the device working on the target)
    Approve { mr: String },
    /// Decline it; both threads carry on
    Decline {
        mr: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum VisibilityArg {
    Private,
    Team,
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Log in with a code from `silt server pair`
    Login {
        #[arg(long)]
        server: String,
        #[arg(long)]
        code: String,
        /// This device's name; defaults to the host name
        #[arg(long)]
        name: Option<String>,
    },
    /// Show which server this machine syncs with
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
        /// The user the device will belong to; created if missing
        #[arg(long, default_value = "owner")]
        user: String,
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

#[derive(Debug, clap::Args)]
struct AgentRun {
    /// Work in a managed checkout instead of the current one
    #[arg(long)]
    isolated: bool,
    /// Arguments passed on to the agent
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Debug, clap::Args)]
struct Continue {
    /// Use this clone instead of a managed checkout (needs a clean tree)
    #[arg(long)]
    here: bool,
    /// Continue with another agent: its own session translated from the
    /// conversation, or a new one primed with it
    #[arg(long)]
    agent: Option<String>,
    /// Arguments passed on to the agent
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CliError(pub String);

fn main() -> Result<(), Exn<CliError>> {
    match Cli::parse().command {
        Command::Run { agent, run } => commands::run_agent(&agent, run.isolated, &run.args),
        Command::Claude(run) => commands::run_agent("claude", run.isolated, &run.args),
        Command::Codex(run) => commands::run_agent("codex", run.isolated, &run.args),
        Command::Gemini(run) => commands::run_agent("gemini", run.isolated, &run.args),
        Command::Opencode(run) => commands::run_agent("opencode", run.isolated, &run.args),
        Command::Copilot(run) => commands::run_agent("copilot", run.isolated, &run.args),
        Command::Track { agent } => commands::track(&agent),
        Command::Resume { thread, continued } => {
            commands::resume(&thread, continued.here, continued.agent.as_deref(), &continued.args)
        }
        Command::Fork { thread, continued } => {
            commands::fork(&thread, continued.here, continued.agent.as_deref(), &continued.args)
        }
        Command::Pull { thread } => commands::pull(thread),
        Command::Mr { command } => match command {
            MrCommand::Create { thread, title, summary } => commands::mr_create(thread, title, summary),
            MrCommand::List { thread } => commands::mr_list(thread),
            MrCommand::Show { mr } => commands::mr_show(&mr),
            MrCommand::Approve { mr } => commands::mr_approve(&mr),
            MrCommand::Decline { mr, reason } => commands::mr_decline(&mr, reason),
        },
        Command::Visibility { visibility, thread } => commands::visibility(
            match visibility {
                VisibilityArg::Private => silt_proto::Visibility::Private,
                VisibilityArg::Team => silt_proto::Visibility::Team,
            },
            thread,
        ),
        Command::Auth { command } => match command {
            AuthCommand::Login { server, code, name } => commands::auth_login(&server, &code, name),
            AuthCommand::Status => commands::auth_status(),
        },
        Command::Log => commands::log(),
        Command::Show { thread } => commands::show(&thread),
        Command::Diff { thread, turn, base } => commands::diff(&thread, turn, base),
        Command::Search {
            query,
            repo,
            author,
            status,
            branch,
            file,
            since,
            until,
        } => commands::search(
            &query.join(" "),
            SearchFilters {
                repo,
                author,
                status,
                branch,
                file,
                since: since.as_deref().map(commands::parse_date).transpose()?,
                until: until.as_deref().map(commands::parse_date).transpose()?,
            },
        ),
        Command::InFlight { path } => commands::in_flight(&path),
        Command::History { thread, offset, limit } => commands::history(&thread, offset, limit),
        Command::Open { no_browser } => commands::open(no_browser),
        Command::Comment {
            thread,
            file,
            lines,
            entry,
            text,
        } => commands::comment(&thread, file, lines.as_deref(), entry, &text.join(" ")),
        Command::Comments { thread, resolve } => commands::comments(&thread, resolve.as_deref()),
        Command::Server { command } => match command {
            ServerCommand::Run { listen, data } => commands::server_run(&listen, data),
            ServerCommand::Pair { data, user } => commands::server_pair(data, &user),
        },
        Command::Daemon { command } => match command {
            DaemonCommand::Run => commands::daemon_run(),
            DaemonCommand::Start => commands::ensure_daemon().map(|_| ()),
            DaemonCommand::Stop => commands::daemon_stop(),
            DaemonCommand::Status => commands::daemon_status(),
        },
        Command::Mcp { thread, checkout } => mcp::serve(&thread, checkout),
        Command::Hook { agent, event } => {
            commands::hook(agent.as_deref(), &event);
            Ok(())
        }
        Command::Dev {
            command: DevCommand::Status { root },
        } => dev_status::print(root),
    }
}
