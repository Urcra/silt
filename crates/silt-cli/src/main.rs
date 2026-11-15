mod admin;
mod commands;
mod desktop;
mod mcp;
mod model;
mod recording;
mod reference;
mod render;
mod workspace;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use exn::Exn;
use silt_daemon::protocol::{Request, SearchFilters};
use silt_proto::OrgChange;

#[derive(Debug, Parser)]
#[command(name = "silt", version, about = "Resume, fork and search your agent sessions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run an agent CLI (claude, codex, gemini, agy, opencode, copilot) with Silt
    /// recording the session and every file edit
    Run {
        agent: String,
        #[command(flatten)]
        run: AgentRun,
    },
    /// Start a chat: an agent CLI in a new folder of its own rather than a project, recorded as a private thread
    Chat {
        /// The agent CLI to run (claude, codex, gemini, agy, opencode, copilot)
        #[arg(default_value = "claude")]
        agent: String,
        /// What the chat is about, naming its folder
        #[arg(long)]
        about: Option<String>,
        /// The workspace it's in, by id; the default workspace otherwise
        #[arg(long)]
        workspace: Option<String>,
        /// Arguments passed on to the agent, after `--`
        #[arg(last = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Run Claude Code with Silt recording the session and every file edit
    Claude(AgentRun),
    /// Run Codex with Silt recording the session and every file edit
    Codex(AgentRun),
    /// Run Gemini CLI with Silt recording the session and every file edit
    Gemini(AgentRun),
    /// Run Antigravity CLI with Silt recording the session and every file edit
    Agy(AgentRun),
    /// Run OpenCode with Silt recording the session and every file edit
    Opencode(AgentRun),
    /// Run GitHub Copilot CLI with Silt recording the session and every file edit
    Copilot(AgentRun),
    /// Start recording the most recent session of an agent in this directory
    Track {
        #[arg(long, default_value = "claude")]
        agent: String,
        /// Record this session rather than the most recent one
        #[arg(long, conflicts_with = "from_hook")]
        session: Option<String>,
        /// Record the session whose hook input is on stdin, and capture that
        /// hook; for a `SessionStart` hook in an environment that starts the
        /// agent itself, such as a cloud session
        #[arg(long)]
        from_hook: bool,
    },
    /// Import the agent sessions already on disk, each as a thread kept on this machine until sent
    Import {
        /// Only sessions that ran in or under these directories
        paths: Vec<PathBuf>,
        /// Only this agent's sessions
        #[arg(long)]
        agent: Option<String>,
    },
    /// Send imported threads to a server, redacted: some, a project's, or all
    Send {
        /// The threads to send
        #[arg(required_unless_present_any = ["project", "all"], conflicts_with_all = ["project", "all"])]
        threads: Vec<String>,
        /// All imported threads of this project, as `silt project` names it
        #[arg(long, conflicts_with = "all")]
        project: Option<String>,
        /// Every imported thread kept on this machine
        #[arg(long)]
        all: bool,
        /// Send them to this server rather than the default one
        #[arg(long)]
        server: Option<String>,
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
        /// Only you see the fork, until a merge request offers it back
        #[arg(long)]
        private: bool,
        /// Fork from where this turn ended, as the thread's page numbers them
        #[arg(long)]
        turn: Option<usize>,
        #[command(flatten)]
        continued: Continue,
    },
    /// Bring a fork's source's newer code and conversation into the fork
    Pull {
        /// The fork; defaults to the thread checked out here
        thread: Option<String>,
    },
    /// Take a thread's files back to how a turn left them; later turns stay
    /// in its history, marked rewound
    Revert {
        /// The thread; defaults to the one checked out here
        thread: Option<String>,
        /// The turn to go back to, as the thread's page numbers them
        #[arg(long)]
        turn: usize,
    },
    /// Merge requests from forks back to their source threads
    Mr {
        #[command(subcommand)]
        command: MrCommand,
    },
    /// Set who on the server sees this project's threads, or one thread's:
    /// `private`, `org`, or `team:<name>`
    Visibility {
        #[arg(value_parser = admin::parse_visibility)]
        visibility: silt_proto::Visibility,
        /// Set it for one thread (and its forks that have no setting of their own)
        #[arg(long)]
        thread: Option<String>,
    },
    /// Connect this machine to a Silt server
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Delete, restore or purge a thread
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
    /// What you should hear about: merge requests, comments and mentions
    Inbox {
        /// Include notifications already read
        #[arg(long)]
        all: bool,
        #[command(subcommand)]
        command: Option<InboxCommand>,
    },
    /// Orgs on the server: who is in them and what they default to
    Org {
        #[command(subcommand)]
        command: OrgCommand,
    },
    /// Teams within an org, for `team:<name>` visibility
    Team {
        #[command(subcommand)]
        command: TeamCommand,
    },
    /// The project (repository) this clone belongs to
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Local models this machine runs with llama.cpp, for Silt's own agent
    /// and for Claude Code
    Model {
        #[command(subcommand)]
        command: model::ModelCommand,
    },
    /// Workspaces: which projects belong together, the server their threads
    /// sync with, and the rules for agents started in them
    Workspace {
        #[command(subcommand)]
        command: workspace::WorkspaceCommand,
    },
    /// List recorded threads, or with --commit the threads and turns that
    /// made a commit
    Log {
        #[arg(long)]
        commit: Option<String>,
    },
    /// Show a thread's conversation and the files each tool call changed
    Show { thread: String },
    /// Show what a thread, or one of its turns, changed
    Diff {
        thread: String,
        /// Only the changes made in this turn (1-based)
        #[arg(long, conflicts_with = "base")]
        turn: Option<usize>,
        /// Diff from merge-base (with the fork's source), last-commit,
        /// last-turn or commit:<sha>; by default from the thread's start.
        /// Or against a branch as git has it now: main or branch:<name>
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
        status: Option<silt_proto::ThreadStatus>,
        #[arg(long)]
        branch: Option<String>,
        /// Threads that changed this file, or anything under this directory
        #[arg(long)]
        file: Option<String>,
        /// YYYY-MM-DD, or a number of days ago such as 7d
        #[arg(long, value_parser = commands::parse_date)]
        since: Option<i64>,
        /// YYYY-MM-DD, or a number of days ago such as 7d
        #[arg(long, value_parser = commands::parse_date)]
        until: Option<i64>,
    },
    /// Open the web UI, in the desktop app when it's installed: threads, search, history, merge requests and comments
    Open {
        /// Print the address instead of opening the app or a browser
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
        #[arg(long, value_parser = commands::parse_lines)]
        lines: Option<(usize, usize)>,
        /// A history entry, numbered as `silt history` shows them
        #[arg(long)]
        entry: Option<usize>,
        /// Number --lines as the file was at this diff base (start, last-turn,
        /// last-commit, merge-base, commit:<sha>), to comment on lines the
        /// thread deleted
        #[arg(long, requires = "lines")]
        base: Option<String>,
        /// Answer this comment, by its id as `silt comments` shows it
        #[arg(long, conflicts_with_all = ["file", "entry"])]
        reply: Option<String>,
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// List a thread's comments, or resolve one
    Comments {
        thread: String,
        #[arg(long)]
        resolve: Option<String>,
        /// Also say where comments on code are at this diff base
        #[arg(long, conflicts_with = "resolve")]
        base: Option<String>,
    },
    /// Other threads changing a file or directory of this repository
    InFlight {
        #[arg(default_value = ".")]
        path: String,
    },
    /// Each line of a file with the thread, turn and prompt that wrote it:
    /// from the log when a thread here has the file, else from the commits'
    /// notes in refs/notes/silt
    Blame {
        file: String,
        /// Read only the notes, as a clone without Silt would
        #[arg(long)]
        notes: bool,
    },
    /// Run or administer a self-hosted sync server
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
    /// Push what this machine recorded to the server once, in this process,
    /// as is done when no daemon runs
    Sync {
        /// One thread's family; defaults to every thread here
        thread: Option<String>,
        #[arg(long, required = true)]
        once: bool,
    },
    /// Manage the background daemon
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Silt's own agent, which the daemon runs over ACP
    #[command(hide = true)]
    Agent {
        #[command(subcommand)]
        agent: AgentCommand,
    },
    /// Called by agent hooks; reads the hook input on stdin
    #[command(hide = true)]
    Hook {
        /// The agent whose hook format the input and output use
        #[arg(long)]
        agent: Option<String>,
        event: String,
    },
    /// Print the command-line reference in Markdown, for the docs site
    #[command(hide = true)]
    Reference,
    /// Serve Silt's history tools to an agent over MCP on stdio
    #[command(hide = true)]
    Mcp {
        #[arg(long)]
        thread: String,
        /// The checkout whose skills to offer; defaults to the current directory
        #[arg(long)]
        checkout: Option<PathBuf>,
        /// The run's token, which limits answers to its workspace
        #[arg(long)]
        token: Option<String>,
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
    /// Give back the space a store from an older Silt holds: compress its
    /// old chunks and shrink the file. The daemon does it once by itself.
    Compact,
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
    /// Keep it open and ask the fork for changes first
    RequestChanges {
        mr: String,
        #[arg(required = true)]
        reason: Vec<String>,
    },
    /// Write its change guide for reviewers, or without text ask the
    /// fork's agent running in the web UI to write it
    Guide { mr: String, text: Vec<String> },
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Sign this machine in: by approving it in the browser, with a
    /// password (`--user`), or with a code from `silt server pair`
    Login {
        #[arg(long)]
        server: String,
        /// A pairing code from `silt server pair`
        #[arg(long, conflicts_with = "user")]
        code: Option<String>,
        /// Sign in as this local account, asking for its password
        #[arg(long)]
        user: Option<String>,
        /// Read the password from standard input
        #[arg(long, requires = "user")]
        password_stdin: bool,
        /// This device's name; defaults to the host name
        #[arg(long)]
        name: Option<String>,
        /// Only print the sign-in link
        #[arg(long)]
        no_browser: bool,
    },
    /// Sign this machine out of a server
    Logout {
        /// The server to sign out of; defaults to the first signed in to
        #[arg(long)]
        server: Option<String>,
    },
    /// Show which servers this machine syncs with
    Status,
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    /// Silt's own agent, on a model this machine runs through the daemon's gateway
    Local,
}

/// Serves `silt agent local` on stdin and stdout; its sessions go to
/// `SILT_LOCAL_HOME`, which the daemon sets, else the data dir's `local/`.
fn local_agent() -> Result<(), Exn<CliError>> {
    let home = std::env::var_os("SILT_LOCAL_HOME")
        .filter(|home| !home.is_empty())
        .map_or_else(|| silt_daemon::paths::data_dir().join("local"), PathBuf::from);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|error| commands::fail(format!("could not start a runtime: {error}")))?;
    runtime.block_on(silt_local::serve(home)).map_err(|error| commands::fail(format!("the local agent stopped: {error}")))
}

#[derive(Debug, Subcommand)]
enum ThreadCommand {
    /// Move a thread of yours to the trash, on every device
    Delete { thread: String },
    /// Bring a thread back from the trash
    Restore { thread: String },
    /// Remove a thread in the trash for good, keeping only what forks of it
    /// build on
    Purge { thread: String },
    /// Erase a Local only thread and its forks from this machine for good, with what agents kept of them
    Erase {
        thread: String,
        /// Erase it; without this, says what would go
        #[arg(long)]
        yes: bool,
    },
    /// List the threads in the trash
    Trash,
    /// Let someone edit a thread of yours with you, or stop it with --remove
    Share {
        thread: String,
        user: String,
        #[arg(long)]
        remove: bool,
    },
    /// Make a read-only link to a thread of yours, for anyone holding it
    Link {
        thread: String,
        /// How long it works: 1, 7 or 30 days
        #[arg(long, default_value_t = 7)]
        days: u32,
        /// Show the thread as it is now, not as it goes on
        #[arg(long)]
        frozen: bool,
    },
}

#[derive(Debug, Subcommand)]
enum InboxCommand {
    /// Mark notifications read
    Read {
        ids: Vec<i64>,
        /// Every unread notification
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OrgCommand {
    /// List your orgs with their members, teams and projects
    List,
    /// Create an org (server admins)
    Create { org: String },
    /// Add a user to an org, or change their role
    Add {
        org: String,
        user: String,
        /// Make them an owner, who manages the org
        #[arg(long)]
        owner: bool,
    },
    /// Remove a user from an org
    Remove { org: String, user: String },
    /// What new projects of the org default to: `private`, `org` or `team:<name>`
    Default {
        org: String,
        #[arg(value_parser = admin::parse_visibility)]
        visibility: silt_proto::Visibility,
    },
    /// Set the org's policy for agents started in its projects on members'
    /// machines (owners); rules not given restrict nothing
    Policy {
        org: String,
        /// The agents allowed, comma-separated (claude, codex, copilot, gemini, agy, opencode)
        #[arg(long)]
        agents: Option<String>,
        /// Refuse the agents of a machine's agents.json
        #[arg(long)]
        no_custom_agents: bool,
        /// The most an agent may do without asking: ask_for_everything, plan, ask_before_commands or full_access
        #[arg(long)]
        max_preset: Option<String>,
        /// Agents run only with their own sandbox on
        #[arg(long)]
        sandbox: bool,
        /// The sandbox also hides Silt's data and other workspaces
        #[arg(long)]
        isolated: bool,
        /// Projects keep this server: no other, and not one machine only
        #[arg(long)]
        home_locked: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TeamCommand {
    Create { org: String, team: String },
    Delete { org: String, team: String },
    Add { org: String, team: String, user: String },
    Remove { org: String, team: String, user: String },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Move this clone's project into an org
    Move { org: String },
    /// Name this clone's project and say what its new threads start with;
    /// an empty value clears a setting
    Set {
        #[arg(long)]
        name: Option<String>,
        /// The agent's registry id, such as claude-acp
        #[arg(long)]
        agent: Option<String>,
        /// ask_before_commands, ask_for_everything or full_access
        #[arg(long)]
        preset: Option<String>,
        /// Git notes on its threads' commits: off, local, or push to origin too
        #[arg(long)]
        git_notes: Option<silt_proto::GitNotes>,
        /// Whether the server comments on its threads' pull requests: on or off
        #[arg(long, value_parser = admin::parse_on_off)]
        forge_comments: Option<bool>,
    },
    /// Choose where this clone's project's new threads sync: a server this
    /// machine is signed in to, or `local` to keep them on this machine.
    /// Threads kept here move to the server chosen.
    Home { home: String },
}

#[derive(Debug, Subcommand)]
enum ServerCommand {
    /// Write the server's config and create its first admin
    Init {
        /// Server data directory; defaults to `server` in the Silt data directory
        #[arg(long)]
        data: Option<PathBuf>,
        /// Address to listen on
        #[arg(long)]
        listen: Option<String>,
        /// The URL people and devices reach the server at, such as https://silt.example.com
        #[arg(long)]
        public_url: Option<String>,
        /// The first admin's user name
        #[arg(long, default_value = "admin")]
        admin: String,
        /// Read the admin's password from standard input instead of generating one
        #[arg(long)]
        password_stdin: bool,
        /// Also write a systemd unit for the server to this path
        #[arg(long)]
        systemd: Option<PathBuf>,
        /// The system user the unit runs the server as
        #[arg(long, default_value = "silt")]
        service_user: String,
        /// Replace an existing config
        #[arg(long)]
        force: bool,
    },
    /// Serve the sync API and the sign-in pages
    Run {
        /// Address to listen on; overrides `listen` in server.toml
        #[arg(long)]
        listen: Option<String>,
        /// Server data directory; defaults to `server` in the Silt data directory
        #[arg(long)]
        data: Option<PathBuf>,
    },
    /// Print a one-time code a device uses with `silt auth login --code`
    Pair {
        #[arg(long)]
        data: Option<PathBuf>,
        /// The user the device will belong to; created if missing
        #[arg(long, default_value = "owner")]
        user: String,
    },
    /// Manage local accounts
    User {
        #[arg(long, global = true)]
        data: Option<PathBuf>,
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Copy the server's data to a new directory; safe while it runs
    Backup {
        dest: PathBuf,
        #[arg(long)]
        data: Option<PathBuf>,
    },
    /// Put a backup in place as the server's data; stop the server first
    Restore {
        backup: PathBuf,
        #[arg(long)]
        data: Option<PathBuf>,
        /// Replace the data that is there
        #[arg(long)]
        replace: bool,
    },
    /// Print a sign-up link for one person
    Invite {
        #[arg(long)]
        data: Option<PathBuf>,
        /// Org the person joins
        #[arg(long)]
        org: Option<String>,
        #[arg(long, default_value_t = 7)]
        days: u32,
    },
}

#[derive(Debug, Subcommand)]
enum UserCommand {
    /// Add a local account; prints a generated password unless one is given
    Add {
        name: String,
        #[arg(long)]
        admin: bool,
        #[arg(long)]
        password_stdin: bool,
    },
    List,
    /// Set a user's password
    Passwd {
        name: String,
        #[arg(long)]
        password_stdin: bool,
    },
    /// Make a user a server admin, or `--revoke` it
    Admin {
        name: String,
        #[arg(long)]
        revoke: bool,
    },
    /// Stop a user signing in and sign them out everywhere, or `--enable` them again
    Disable {
        name: String,
        #[arg(long)]
        enable: bool,
    },
    /// Sign a user out of every device and browser, such as after a lost laptop
    SignOut {
        name: String,
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

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("silt: {}", render::safe(&explain(&error)));
            std::process::ExitCode::FAILURE
        }
    }
}

/// The error and what caused it, outermost first, as one line for people:
/// no source locations, which are for Silt's developers.
pub(crate) fn explain(error: &Exn<CliError>) -> String {
    let mut causes: Vec<String> = Vec::new();
    let mut frame = error.frame();
    loop {
        causes.push(frame.error().to_string());
        match frame.children().first() {
            Some(child) => frame = child,
            None => break,
        }
    }
    let mut source = frame.error().source();
    while let Some(cause) = source {
        causes.push(cause.to_string());
        source = cause.source();
    }
    causes.dedup();
    causes.join(": ")
}

fn run() -> Result<(), Exn<CliError>> {
    let cli = Cli::parse();
    if cfg!(windows)
        && !matches!(cli.command, Command::Open { .. })
        && let Ok(cwd) = std::env::current_dir()
    {
        desktop::refuse_wsl_folder(&cwd).map_err(commands::fail)?;
    }
    match cli.command {
        Command::Run { agent, run } => commands::run_agent(&agent, run.isolated, &run.args),
        Command::Chat { agent, about, workspace, args } => commands::chat(&agent, about.as_deref(), workspace.as_deref(), &args),
        Command::Workspace { command } => workspace::run(command),
        Command::Model { command } => model::run(command),
        Command::Claude(run) => commands::run_agent("claude", run.isolated, &run.args),
        Command::Codex(run) => commands::run_agent("codex", run.isolated, &run.args),
        Command::Gemini(run) => commands::run_agent("gemini", run.isolated, &run.args),
        Command::Agy(run) => commands::run_agent("agy", run.isolated, &run.args),
        Command::Opencode(run) => commands::run_agent("opencode", run.isolated, &run.args),
        Command::Copilot(run) => commands::run_agent("copilot", run.isolated, &run.args),
        Command::Track { agent, session, from_hook } => {
            if from_hook {
                // A failed hook must not stop the agent's session.
                if let Err(error) = commands::track_from_hook(&agent) {
                    eprintln!("silt: could not record this session: {}", render::safe(&explain(&error)));
                }
                Ok(())
            } else {
                commands::track(&agent, session.as_deref())
            }
        }
        Command::Import { paths, agent } => commands::import(&paths, agent.as_deref()),
        Command::Send { threads, project, all: _, server } => commands::send(threads, project, server),
        Command::Resume { thread, continued } => {
            commands::resume(&thread, continued.here, continued.agent.as_deref(), &continued.args)
        }
        Command::Fork { thread, private, turn, continued } => {
            commands::fork(&thread, private, turn, continued.here, continued.agent.as_deref(), &continued.args)
        }
        Command::Pull { thread } => commands::pull(thread),
        Command::Revert { thread, turn } => commands::revert(thread, turn),
        Command::Mr { command } => match command {
            MrCommand::Create { thread, title, summary } => commands::mr_create(thread, title, summary),
            MrCommand::List { thread } => commands::mr_list(thread),
            MrCommand::Show { mr } => commands::mr_show(&mr),
            MrCommand::Approve { mr } => commands::mr_approve(&mr),
            MrCommand::Decline { mr, reason } => commands::mr_decline(&mr, reason),
            MrCommand::RequestChanges { mr, reason } => commands::mr_request_changes(&mr, &reason.join(" ")),
            MrCommand::Guide { mr, text } => commands::mr_guide(&mr, (!text.is_empty()).then(|| text.join(" "))),
        },
        Command::Visibility { visibility, thread } => commands::visibility(visibility, thread),
        Command::Auth { command } => match command {
            AuthCommand::Login { server, code, user, password_stdin, name, no_browser } => {
                admin::auth_login(admin::LoginOptions { server, code, user, password_stdin, name, no_browser })
            }
            AuthCommand::Logout { server } => admin::auth_logout(server),
            AuthCommand::Status => commands::auth_status(),
        },
        Command::Thread { command } => match command {
            ThreadCommand::Delete { thread } => commands::thread_change(Request::ThreadDelete { thread }),
            ThreadCommand::Restore { thread } => commands::thread_change(Request::ThreadRestore { thread }),
            ThreadCommand::Purge { thread } => commands::thread_change(Request::ThreadPurge { thread }),
            ThreadCommand::Erase { thread, yes } => commands::thread_erase(thread, yes),
            ThreadCommand::Trash => commands::trash(),
            ThreadCommand::Share { thread, user, remove } => {
                commands::thread_change(Request::ThreadShare { thread, user, remove })
            }
            ThreadCommand::Link { thread, days, frozen } => commands::thread_link(thread, days, frozen),
        },
        Command::Inbox { all, command } => match command {
            None => commands::inbox(all),
            Some(InboxCommand::Read { ids, all }) => commands::inbox_read(ids, all),
        },
        Command::Org { command } => match command {
            OrgCommand::List => admin::orgs(),
            OrgCommand::Create { org } => admin::org_change(OrgChange::Create { org }),
            OrgCommand::Add { org, user, owner } => admin::org_change(OrgChange::AddMember { org, user, owner }),
            OrgCommand::Remove { org, user } => admin::org_change(OrgChange::RemoveMember { org, user }),
            OrgCommand::Default { org, visibility } => admin::org_change(OrgChange::DefaultVisibility { org, visibility }),
            OrgCommand::Policy { org, agents, no_custom_agents, max_preset, sandbox, isolated, home_locked } => {
                let max_preset = match max_preset.as_deref().map(|name| silt_workspace::Preset::parse(name).ok_or(name)) {
                    Some(Err(name)) => return Err(commands::fail(format!("no preset {name}"))),
                    Some(Ok(preset)) => Some(preset),
                    None => None,
                };
                let policy = silt_workspace::Policy {
                    agents: agents.map(|list| {
                        list.split(',').map(str::trim).filter(|key| !key.is_empty()).map(silt_workspace::AgentKey::new).collect()
                    }),
                    custom_agents: no_custom_agents.then_some(false),
                    max_preset,
                    sandbox,
                    isolated,
                    home_locked,
                    // A Local only workspace has no server, so no org's policy reaches it.
                    local_only: false,
                    wide_history: false,
                };
                admin::org_change(OrgChange::Policy { org, policy })
            }
        },
        Command::Team { command } => admin::org_change(match command {
            TeamCommand::Create { org, team } => OrgChange::CreateTeam { org, team },
            TeamCommand::Delete { org, team } => OrgChange::RemoveTeam { org, team },
            TeamCommand::Add { org, team, user } => OrgChange::AddToTeam { org, team, user },
            TeamCommand::Remove { org, team, user } => OrgChange::RemoveFromTeam { org, team, user },
        }),
        Command::Project { command: ProjectCommand::Move { org } } => {
            admin::project_here().and_then(|repo| admin::org_change(OrgChange::MoveProject { repo, org }))
        }
        Command::Project { command: ProjectCommand::Home { home } } => {
            admin::project_here().and_then(|repo| commands::project_home(repo, home))
        }
        Command::Project { command: ProjectCommand::Set { name, agent, preset, git_notes, forge_comments } } => {
            admin::project_here().and_then(|repo| {
                admin::org_change(OrgChange::ProjectSettings {
                    repo,
                    name,
                    default_agent: agent,
                    default_preset: preset,
                    git_notes,
                    forge_comments,
                })
            })
        }
        Command::Log { commit: None } => commands::log(),
        Command::Log { commit: Some(commit) } => commands::find_commit(&commit),
        Command::Show { thread } => commands::show(&thread),
        Command::Diff { thread, turn, base } => commands::diff(&thread, turn, base),
        Command::Search { query, repo, author, status, branch, file, since, until } => {
            commands::search(&query.join(" "), SearchFilters { repo, author, status, branch, file, since, until })
        }
        Command::InFlight { path } => commands::in_flight(&path),
        Command::Blame { file, notes } => commands::blame(&file, notes),
        Command::History { thread, offset, limit } => commands::history(&thread, offset, limit),
        Command::Open { no_browser } => commands::open(no_browser),
        Command::Comment { thread, file, lines, entry, base: _, reply: Some(comment), text }
            if file.is_none() && lines.is_none() && entry.is_none() =>
        {
            commands::reply(&thread, &comment, &text.join(" "))
        }
        Command::Comment { thread, file, lines, entry, base, reply: _, text } => {
            commands::comment(&thread, file, lines, entry, base, &text.join(" "))
        }
        Command::Comments { thread, resolve, base } => commands::comments(&thread, resolve.as_deref(), base),
        Command::Server { command } => match command {
            ServerCommand::Init { data, listen, public_url, admin, password_stdin, systemd, service_user, force } => {
                admin::server_init(admin::InitOptions {
                    data,
                    listen,
                    public_url,
                    admin,
                    password_stdin,
                    systemd,
                    service_user,
                    force,
                })
            }
            ServerCommand::Run { listen, data } => admin::server_run(listen, data),
            ServerCommand::Pair { data, user } => admin::server_pair(data, &user),
            ServerCommand::User { data, command } => match command {
                UserCommand::Add { name, admin: is_admin, password_stdin } => {
                    admin::user_add(data, &name, is_admin, password_stdin)
                }
                UserCommand::List => admin::user_list(data),
                UserCommand::Passwd { name, password_stdin } => admin::user_passwd(data, &name, password_stdin),
                UserCommand::Admin { name, revoke } => admin::user_admin(data, &name, revoke),
                UserCommand::Disable { name, enable } => admin::user_disable(data, &name, !enable),
                UserCommand::SignOut { name } => admin::user_sign_out(data, &name),
            },
            ServerCommand::Invite { data, org, days } => admin::invite(data, org, days),
            ServerCommand::Backup { dest, data } => admin::backup(data, &dest),
            ServerCommand::Restore { backup, data, replace } => admin::restore(&backup, data, replace),
        },
        Command::Sync { thread, once: _ } => commands::sync_once(thread.as_deref()),
        Command::Daemon { command } => match command {
            DaemonCommand::Run => commands::daemon_run(),
            DaemonCommand::Start => commands::ensure_daemon().map(|_| ()),
            DaemonCommand::Stop => commands::daemon_stop(),
            DaemonCommand::Status => commands::daemon_status(),
            DaemonCommand::Compact => commands::daemon_compact(),
        },
        Command::Mcp { thread, checkout, token } => mcp::serve(&thread, checkout, token),
        Command::Reference => {
            print!("{}", reference::markdown(&<Cli as clap::CommandFactory>::command()));
            Ok(())
        }
        Command::Agent { agent: AgentCommand::Local } => local_agent(),
        Command::Hook { agent, event } => {
            commands::hook(agent.as_deref(), &event);
            Ok(())
        }
    }
}
