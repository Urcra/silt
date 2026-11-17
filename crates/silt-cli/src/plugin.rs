//! `silt plugin` and `silt approvals`: what a workspace's sandboxed agents
//! reach production through, and the person's answers to the calls that
//! wait for them (ADR 0086).

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;

use clap::Subcommand;
use exn::{Exn, ResultExt as _};
use silt_daemon::paths;
use silt_daemon::plugins::{self, AccessView, ApprovalState, ApprovalView, GrantScope, Kind, Outcome, Plugin};
use silt_daemon::protocol::Request;

use crate::CliError;
use crate::commands::{call, fail};

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Add a plugin from its JSON definition, or replace the one of its name
    Add { file: PathBuf },
    /// List this machine's plugins and the workspaces each is on in
    List,
    /// Remove a plugin
    Remove { name: String },
    /// Turn a plugin on in a workspace, whose sandboxed agents then get its tools
    Enable {
        name: String,
        #[arg(long)]
        workspace: String,
    },
    /// Turn a plugin off in a workspace
    Disable {
        name: String,
        #[arg(long)]
        workspace: String,
    },
    /// Fetch a GraphQL environment's schema by introspection, for data rules that see past field names
    Schema {
        name: String,
        #[arg(long)]
        environment: String,
        /// The workspace whose credential it's fetched with; the first it's on in by default
        #[arg(long)]
        workspace: Option<String>,
    },
}

fn core_error(error: silt_daemon::CoreError) -> Exn<CliError> {
    fail(error.to_string())
}

pub fn run(command: PluginCommand) -> Result<(), Exn<CliError>> {
    let data = paths::data_dir();
    match command {
        PluginCommand::Add { file } => {
            let bytes = std::fs::read(&file).or_raise(|| CliError(format!("cannot read {}", file.display())))?;
            let plugin: Plugin = serde_json::from_slice(&bytes)
                .or_raise(|| CliError(format!("{} isn't a plugin's definition", file.display())))?;
            let name = plugin.name.clone();
            plugins::put(&data, plugin).map_err(core_error)?;
            println!("added {name}; turn it on with `silt plugin enable {name} --workspace <id>`");
        }
        PluginCommand::List => {
            let all = plugins::load(&data);
            if all.is_empty() {
                println!("no plugins; add one with `silt plugin add <file>`");
            }
            for plugin in all {
                let kind = match &plugin.kind {
                    Kind::Graphql => "GraphQL".to_owned(),
                    Kind::Command { argv } => format!("runs {}", argv.join(" ")),
                    Kind::Mcp { argv } => format!("MCP server {}", argv.join(" ")),
                };
                let on = if plugin.workspaces.is_empty() {
                    "on in no workspace".to_owned()
                } else {
                    format!("on in {}", plugin.workspaces.join(", "))
                };
                println!("{}  {kind}  {on}", plugin.name);
                for (name, environment) in &plugin.environments {
                    let data: Vec<&str> = environment.data.iter().map(|class| class.label()).collect();
                    let kept = if data.is_empty() { String::new() } else { format!("  keeps {} from agents", data.join(", ")) };
                    println!(
                        "  {name}: {}  reads {:?}, changes {:?}{kept}",
                        environment.target, environment.reads, environment.changes
                    );
                }
            }
        }
        PluginCommand::Remove { name } => {
            plugins::remove(&data, &name).map_err(core_error)?;
            println!("removed {name}");
        }
        PluginCommand::Schema { name, environment, workspace } => {
            let plugin = plugins::load(&data)
                .into_iter()
                .find(|plugin| plugin.name == name)
                .ok_or_else(|| fail(format!("no plugin {name}")))?;
            let workspace = workspace.or_else(|| plugin.workspaces.first().cloned()).unwrap_or_else(|| "default".to_owned());
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .or_raise(|| CliError("cannot start the async runtime".into()))?;
            runtime.block_on(plugins::fetch_schema(&data, &plugin, &environment, &workspace)).map_err(fail)?;
            println!("kept {name}'s {environment} schema; its data rules now walk it");
        }
        PluginCommand::Enable { name, workspace } => turn(&data, &name, &workspace, true)?,
        PluginCommand::Disable { name, workspace } => turn(&data, &name, &workspace, false)?,
    }
    Ok(())
}

fn turn(data: &std::path::Path, name: &str, workspace: &str, on: bool) -> Result<(), Exn<CliError>> {
    let mut plugin =
        plugins::load(data).into_iter().find(|plugin| plugin.name == name).ok_or_else(|| fail(format!("no plugin {name}")))?;
    plugin.workspaces.retain(|other| other != workspace);
    if on {
        plugin.workspaces.push(workspace.to_owned());
        plugin.workspaces.sort();
    }
    plugins::put(data, plugin).map_err(core_error)?;
    println!("{name} is {} in {workspace}", if on { "on" } else { "off" });
    Ok(())
}

fn state(approval: &ApprovalView) -> &'static str {
    match approval.state {
        ApprovalState::Pending => "waiting",
        ApprovalState::Approved => "approved",
        ApprovalState::Rejected => "rejected",
        ApprovalState::Expired => "expired",
    }
}

/// `silt approvals`: the calls waiting for the person, or all of them.
pub fn list(all: bool, thread: Option<String>) -> Result<(), Exn<CliError>> {
    let approvals: Vec<ApprovalView> = call(&Request::Approvals { thread, all })?;
    let shown: Vec<&ApprovalView> = approvals.iter().filter(|approval| all || approval.state == ApprovalState::Pending).collect();
    if shown.is_empty() {
        println!("{}", if all { "no approvals" } else { "nothing waits for you" });
    }
    for approval in shown {
        println!(
            "{}  {}  {} {} in {} ({})  thread {}",
            approval.id,
            state(approval),
            approval.plugin,
            approval.tool,
            approval.environment,
            approval.target,
            crate::render::short(&approval.thread)
        );
    }
    Ok(())
}

/// `silt approve`: shows Silt's own copy of the call and asks, on a
/// terminal, whether to run it.
pub fn answer(id: u64, reject: bool, grant: Option<&str>, minutes: u32) -> Result<(), Exn<CliError>> {
    let grant = match grant {
        Some("run") => Some(GrantScope::Run),
        Some(_) => Some(GrantScope::Thread { minutes }),
        None => None,
    };
    let approvals: Vec<ApprovalView> = call(&Request::Approvals { thread: None, all: false })?;
    let approval = approvals.into_iter().find(|approval| approval.id == id).ok_or_else(|| fail(format!("no approval {id}")))?;
    if approval.state != ApprovalState::Pending {
        return Err(fail(format!("approval {id} is {} already", state(&approval))));
    }
    println!("{} {} in {} ({})", approval.plugin, approval.tool, approval.environment, approval.target);
    println!("thread {}\n", crate::render::short(&approval.thread));
    println!("{}\n", approval.shown);
    if !reject {
        // An agent runs without a terminal, so it can't answer for the person.
        if !std::io::stdin().is_terminal() {
            return Err(fail("approving asks on a terminal; run `silt approve` in one, or answer in the web UI"));
        }
        if grant.is_some() && approval.once {
            return Err(fail(format!(
                "{}'s {} allows no grant: each call is approved on its own",
                approval.plugin, approval.environment
            )));
        }
        match grant {
            Some(GrantScope::Run) => print!("Run exactly this, and let its run make calls like it without asking? [y/N] "),
            Some(GrantScope::Thread { minutes }) => {
                print!("Run exactly this, and let the thread make calls like it without asking for {minutes} minutes? [y/N] ");
            }
            None => print!("Run exactly this, once? [y/N] "),
        }
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).or_raise(|| CliError("cannot read the answer".into()))?;
        if !matches!(line.trim(), "y" | "Y" | "yes") {
            println!("left waiting");
            return Ok(());
        }
    }
    let said: String =
        call(&Request::ApprovalAnswer { id, approve: !reject, hash: approval.hash, grant: grant.filter(|_| !reject) })?;
    println!("{said}");
    Ok(())
}

/// `silt revoke`: ends a grant.
pub fn revoke(id: u64) -> Result<(), Exn<CliError>> {
    let said: String = call(&Request::GrantRevoke { id })?;
    println!("{said}");
    Ok(())
}

/// `silt access show`: what reached a thread's sandboxes through Silt.
pub fn access(thread: &str) -> Result<(), Exn<CliError>> {
    let access: AccessView = call(&Request::Access { thread: thread.to_owned() })?;
    if !access.grants.is_empty() {
        println!("Grants");
        for grant in &access.grants {
            let what = if grant.change { "changes" } else { "reads" };
            let scope = if grant.run_only { "its run" } else { "the thread" };
            let state = if grant.active { "in force" } else { "ended" };
            println!("  {}  {} {} {what}, for {scope}  {state}", grant.id, grant.plugin, grant.environment);
        }
    }
    println!("Plugin calls");
    if access.calls.is_empty() {
        println!("  none");
    }
    for call in &access.calls {
        let how = match (&call.outcome, call.asked) {
            (Some(Outcome::Done { .. }), true) => "approved, ran",
            (Some(Outcome::Done { .. }), false) if call.grant.is_some() => "ran under a grant",
            (Some(Outcome::Done { .. }), false) => "ran without asking",
            (Some(Outcome::Refused { .. }), _) => "refused",
            (Some(Outcome::Failed { .. }), _) => "failed",
            (Some(Outcome::Unknown { .. }), _) => "outcome unknown",
            _ => call.state.as_str(),
        };
        println!("  {}  {} {} in {}  {how}", call.id, call.plugin, call.tool, call.environment);
        if let Some(Outcome::Refused { why } | Outcome::Unknown { why }) = &call.outcome {
            println!("      {why}");
        }
    }
    if !access.refused.is_empty() {
        println!("Refused by the gateway");
        for refused in &access.refused {
            println!("  {}:{}  {}", refused.host, refused.port, refused.why);
        }
    }
    Ok(())
}
