//! `silt workspace`: workspaces, the projects in them and their rules
//! (ADR 0065).

use std::collections::BTreeSet;
use std::env;

use clap::Subcommand;
use exn::{Exn, ResultExt};
use silt_daemon::protocol::{Request, WorkspacesView};
use silt_workspace::{AgentKey, MatchRule, Preset, ServerBinding, Why, Workspace, WorkspaceId, Workspaces};

use crate::CliError;
use crate::commands::{call, ensure_daemon, fail};

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// List the workspaces, their servers, rules and projects
    List,
    /// Make a workspace
    New {
        name: String,
        /// The server its threads sync with, one signed in to
        #[arg(long, conflicts_with = "local")]
        server: Option<String>,
        /// The org on that server its projects belong to, whose policy then applies
        #[arg(long, requires = "server")]
        org: Option<String>,
        /// Keep its threads on this machine
        #[arg(long)]
        local: bool,
        /// Keep its threads on this machine, with only local models answering
        #[arg(long, conflicts_with = "server")]
        local_only: bool,
        /// Put projects whose remote matches in it, such as `github.com/acme/*`; repeatable
        #[arg(long = "remote")]
        remotes: Vec<String>,
        /// Put clones at or under this folder in it, such as `~/work`; repeatable
        #[arg(long = "folder")]
        folders: Vec<String>,
        /// Make it where projects nothing matches go
        #[arg(long)]
        default: bool,
    },
    /// Add or remove a workspace's match rule
    Rule {
        workspace: String,
        #[arg(long, required_unless_present = "folder", conflicts_with = "folder")]
        remote: Option<String>,
        #[arg(long)]
        folder: Option<String>,
        /// Remove the rule rather than add it
        #[arg(long)]
        remove: bool,
    },
    /// Put this clone's project, or `--project`'s, in a workspace; `--rules` puts it back under the match rules
    Assign {
        #[arg(required_unless_present = "rules")]
        workspace: Option<String>,
        #[arg(long, conflicts_with = "workspace")]
        rules: bool,
        /// The project, by its normalized remote; this clone's by default
        #[arg(long)]
        project: Option<String>,
    },
    /// Say which workspace this clone is in, and the rules in effect there
    Which,
    /// Change a workspace's rules; those its org sets apply as well
    Policy {
        workspace: String,
        /// The agents allowed, comma-separated (claude, codex, copilot, gemini, agy, opencode), or `any`
        #[arg(long)]
        agents: Option<String>,
        /// Whether agents from agents.json are allowed: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        custom_agents: Option<bool>,
        /// The most an agent may do without asking: ask_for_everything, plan, ask_before_commands, full_access, or `any`
        #[arg(long)]
        max_preset: Option<String>,
        /// Agents run only with their own sandbox on: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        sandbox: Option<bool>,
        /// The sandbox also hides Silt's data and other workspaces: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        isolated: Option<bool>,
        /// Projects keep the workspace's server: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        home_locked: Option<bool>,
        /// Nothing Silt holds of its threads leaves this machine, and only local models answer: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        local_only: Option<bool>,
    },
    /// Set or unset a variable given to every agent started in a workspace, such as its own CLAUDE_CONFIG_DIR
    Env {
        workspace: String,
        /// `NAME=value` to set, or `NAME` with --unset
        variable: String,
        #[arg(long)]
        unset: bool,
    },
    /// Make a workspace the default, where projects nothing matches go
    Default { workspace: String },
    /// Remove a workspace; its projects go to the others by the rules
    Remove { workspace: String },
}

pub fn run(command: WorkspaceCommand) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    let view: WorkspacesView = call(&Request::Workspaces)?;
    match command {
        WorkspaceCommand::List => {
            list(&view);
            Ok(())
        }
        WorkspaceCommand::Which => which(&view),
        WorkspaceCommand::Assign { workspace, project, .. } => {
            let repo = match project {
                Some(project) => project,
                None => project_here()?.0,
            };
            if let Some(workspace) = &workspace {
                find(&editable(&view), workspace)?;
            }
            let workspace = workspace.map(|workspace| resolve_id(&view, &workspace)).transpose()?;
            let said: String = call(&Request::AssignProject { repo, workspace })?;
            println!("{said}");
            Ok(())
        }
        change => {
            let mut all = editable(&view);
            let said = edit(&mut all, change)?;
            let _: String = call(&Request::SetWorkspaces { workspaces: all })?;
            println!("{said}");
            Ok(())
        }
    }
}

fn editable(view: &WorkspacesView) -> Workspaces {
    Workspaces {
        default: view.default.clone(),
        workspaces: view.workspaces.iter().map(|shown| shown.workspace.clone()).collect(),
    }
}

/// A workspace by its id or its name.
fn find<'a>(all: &'a Workspaces, wanted: &str) -> Result<&'a Workspace, Exn<CliError>> {
    all.workspaces
        .iter()
        .find(|workspace| workspace.id.as_str() == wanted || workspace.name.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| fail(format!("no workspace {wanted}; `silt workspace list` lists them")))
}

fn resolve_id(view: &WorkspacesView, wanted: &str) -> Result<String, Exn<CliError>> {
    Ok(find(&editable(view), wanted)?.id.to_string())
}

fn find_mut<'a>(all: &'a mut Workspaces, wanted: &str) -> Result<&'a mut Workspace, Exn<CliError>> {
    let id = find(all, wanted)?.id.clone();
    Ok(all.get_mut(&id).expect("just found"))
}

fn folder(text: &str) -> Result<MatchRule, Exn<CliError>> {
    silt_daemon::workspaces::folder_rule(text).or_raise(|| CliError(format!("can't use the folder {text}")))
}

fn edit(all: &mut Workspaces, change: WorkspaceCommand) -> Result<String, Exn<CliError>> {
    match change {
        WorkspaceCommand::New { name, server, org, local, local_only, remotes, folders, default } => {
            let id = WorkspaceId::from_name(&name).map_err(|error| fail(error.to_string()))?;
            if all.get(&id).is_some() {
                return Err(fail(format!("there's a workspace {id} already")));
            }
            let mut workspace = Workspace::new(id.clone(), name.trim());
            workspace.policy.local_only = local_only;
            workspace.server = match (server, local || local_only) {
                (Some(url), _) => Some(ServerBinding { url: url.trim_end_matches('/').to_owned(), org }),
                (None, true) => None,
                // Like the default workspace, unless told to stay here.
                (None, false) => all.default_workspace().server.clone(),
            };
            workspace.rules = remotes.into_iter().map(MatchRule::Remote).collect();
            for text in folders {
                workspace.rules.push(folder(&text)?);
            }
            let said = format!("made the workspace {} ({id}), {}", workspace.name, server_text(&workspace));
            all.workspaces.push(workspace);
            if default {
                all.default = id;
            }
            Ok(said)
        }
        WorkspaceCommand::Rule { workspace, remote, folder: path, remove } => {
            let rule = match (remote, path) {
                (Some(pattern), _) => MatchRule::Remote(pattern.trim_matches('/').to_owned()),
                (None, Some(path)) => folder(&path)?,
                (None, None) => return Err(fail("give --remote or --folder")),
            };
            let workspace = find_mut(all, &workspace)?;
            if remove {
                let before = workspace.rules.len();
                workspace.rules.retain(|kept| *kept != rule);
                if workspace.rules.len() == before {
                    return Err(fail(format!("{} has no such rule", workspace.name)));
                }
                Ok(format!("{} no longer takes {}", workspace.name, rule_text(&rule)))
            } else {
                if !workspace.rules.contains(&rule) {
                    workspace.rules.push(rule.clone());
                }
                Ok(format!("{} takes {}", workspace.name, rule_text(&rule)))
            }
        }
        WorkspaceCommand::Policy { workspace, agents, custom_agents, max_preset, sandbox, isolated, home_locked, local_only } => {
            let workspace = find_mut(all, &workspace)?;
            let policy = &mut workspace.policy;
            if let Some(agents) = agents {
                policy.agents = match agents.trim() {
                    "any" | "" => None,
                    list => Some(
                        list.split(',').map(str::trim).filter(|key| !key.is_empty()).map(AgentKey::new).collect::<BTreeSet<_>>(),
                    ),
                };
            }
            if let Some(custom) = custom_agents {
                policy.custom_agents = (!custom).then_some(false);
            }
            if let Some(cap) = max_preset {
                policy.max_preset = match cap.as_str() {
                    "any" => None,
                    name => Some(Preset::parse(name).ok_or_else(|| fail(format!("no preset {name}")))?),
                };
            }
            if let Some(sandbox) = sandbox {
                policy.sandbox = sandbox;
            }
            if let Some(isolated) = isolated {
                policy.isolated = isolated;
            }
            if let Some(local_only) = local_only {
                if local_only && workspace.server.is_some() {
                    return Err(fail(format!(
                        "{} syncs with a server; a Local only workspace has none: make one with `silt workspace new <name> --local-only`",
                        workspace.name
                    )));
                }
                workspace.policy.local_only = local_only;
            }
            let policy = &mut workspace.policy;
            if let Some(locked) = home_locked {
                policy.home_locked = locked;
            }
            Ok(format!("{}'s rules: {}", workspace.name, policy_text(policy)))
        }
        WorkspaceCommand::Env { workspace, variable, unset } => {
            let workspace = find_mut(all, &workspace)?;
            if unset {
                workspace.env.remove(&variable);
                return Ok(format!("{} no longer sets {variable}", workspace.name));
            }
            let (name, value) =
                variable.split_once('=').ok_or_else(|| fail("give the variable as NAME=value, or NAME with --unset"))?;
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(fail(format!("{name:?} isn't a variable name")));
            }
            let value = match value.strip_prefix("~/") {
                Some(rest) => match silt_os::home_dir() {
                    Some(home) => home.join(rest).to_string_lossy().into_owned(),
                    None => value.to_owned(),
                },
                None => value.to_owned(),
            };
            workspace.env.insert(name.to_owned(), value.clone());
            Ok(format!("agents in {} get {name}={value}", workspace.name))
        }
        WorkspaceCommand::Default { workspace } => {
            let workspace = find(all, &workspace)?;
            let said = format!("projects nothing matches go to {}", workspace.name);
            all.default = workspace.id.clone();
            Ok(said)
        }
        WorkspaceCommand::Remove { workspace } => {
            let id = find(all, &workspace)?.id.clone();
            if id == all.default {
                return Err(fail("that's the default workspace; make another the default first"));
            }
            all.workspaces.retain(|workspace| workspace.id != id);
            Ok(format!("removed {id}"))
        }
        WorkspaceCommand::List | WorkspaceCommand::Which | WorkspaceCommand::Assign { .. } => unreachable!("not an edit"),
    }
}

fn server_text(workspace: &Workspace) -> String {
    match &workspace.server {
        Some(ServerBinding { url, org: Some(org) }) => format!("syncs with {url}, org {org}"),
        Some(ServerBinding { url, org: None }) => format!("syncs with {url}"),
        None => "this machine only".into(),
    }
}

fn rule_text(rule: &MatchRule) -> String {
    match rule {
        MatchRule::Remote(pattern) => format!("remote {pattern}"),
        MatchRule::Folder(folder) => format!("folder {folder}"),
    }
}

fn policy_text(policy: &silt_workspace::Policy) -> String {
    let mut parts = vec![
        match &policy.agents {
            None => "any agent".to_owned(),
            Some(agents) if agents.is_empty() => "no agent".to_owned(),
            Some(agents) => agents.iter().map(AgentKey::as_str).collect::<Vec<_>>().join(", ") + " only",
        },
        match policy.max_preset {
            None => "up to Full access".to_owned(),
            Some(cap) => format!("at most {}", cap.label()),
        },
    ];
    if policy.custom_agents == Some(false) {
        parts.push("no custom agents".into());
    }
    if policy.isolated {
        parts.push("isolated".into());
    } else if policy.sandbox {
        parts.push("sandboxed".into());
    }
    if policy.home_locked {
        parts.push("home locked".into());
    }
    if policy.local_only {
        parts.push("local only".into());
    }
    parts.join(" · ")
}

fn list(view: &WorkspacesView) {
    for shown in &view.workspaces {
        let workspace = &shown.workspace;
        let default = if workspace.id == view.default { " (default)" } else { "" };
        println!("{}{default}  [{}]", workspace.name, workspace.id);
        println!("  {}", server_text(workspace));
        for rule in &workspace.rules {
            println!("  takes {}", rule_text(rule));
        }
        println!("  rules: {}", policy_text(&shown.effective));
        if let Some(org) = &shown.org_policy {
            println!("    {}'s policy: {}", org.org, policy_text(&org.policy));
        }
        if shown.waiting_for_policy {
            println!("    waiting for the org's policy; agents don't start here until it arrives");
        }
        for (name, value) in &workspace.env {
            println!("  agents get {name}={value}");
        }
        for project in &shown.projects {
            println!("  {}  {}", project.repo, why_text(&project.why));
        }
    }
    if !view.saved {
        println!("\none workspace for everything; `silt workspace new <name> --remote <pattern>` makes another");
    }
}

fn why_text(why: &Why) -> String {
    match why {
        Why::Chosen => "set by you".into(),
        Why::Rule(rule) => format!("by rule {}", rule_text(rule)),
        Why::Default => "matched nothing".into(),
    }
}

/// This clone's project and its root.
fn project_here() -> Result<(String, std::path::PathBuf), Exn<CliError>> {
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let repo = silt_git::Repo::discover(&cwd).or_raise(|| CliError("not in a git repository".into()))?;
    let project = repo.project().or_raise(|| CliError("cannot tell this clone's project".into()))?;
    Ok((project, repo.root().to_owned()))
}

fn which(view: &WorkspacesView) -> Result<(), Exn<CliError>> {
    let (project, root) = project_here()?;
    // A project the person put somewhere is there from any clone; otherwise this clone's folder counts, as it does
    // for the threads started in it.
    let chosen = view
        .workspaces
        .iter()
        .find(|shown| shown.projects.iter().any(|placed| placed.repo == project && placed.why == Why::Chosen));
    let (shown, why) = match chosen {
        Some(shown) => (shown, Why::Chosen),
        None => {
            let all = editable(view);
            let root = root.to_string_lossy().replace('\\', "/");
            let resolved = all.resolve(&project, Some(&root), None);
            let shown = view.workspaces.iter().find(|shown| shown.workspace.id == resolved.workspace.id).expect("listed");
            (shown, resolved.why)
        }
    };
    let workspace = &shown.workspace;
    println!("{} ({})", workspace.name, why_text(&why));
    println!("  syncs        {}", server_text(workspace).trim_start_matches("syncs with "));
    let policy = &shown.effective;
    println!(
        "  agents       {}",
        policy.agents.as_ref().map_or("any".to_owned(), |agents| agents
            .iter()
            .map(AgentKey::as_str)
            .collect::<Vec<_>>()
            .join(", "))
    );
    println!(
        "  permissions  {}",
        policy.max_preset.map_or("any, up to Full access".to_owned(), |cap| format!("at most {}", cap.label()))
    );
    println!(
        "  sandbox      {}",
        if policy.isolated {
            "required, isolated"
        } else if policy.sandbox {
            "required"
        } else {
            "not required"
        }
    );
    if let Some(org) = &shown.org_policy {
        println!("  from {}'s policy on {}: {}", org.org, org.server, policy_text(&org.policy));
    }
    Ok(())
}
