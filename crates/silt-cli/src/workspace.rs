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
        /// Agents run in Silt's own sandbox, and don't start on a machine that can't make one: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        silt_sandbox: Option<bool>,
        /// What makes Silt's sandbox once it's on: bubblewrap, the only one so far
        #[arg(long, value_parser = parse_backend)]
        backend: Option<silt_workspace::Backend>,
        /// The sandbox also keeps agents out of Silt's data and other workspaces' folders: on or off
        #[arg(long = "hide-other-workspaces", alias = "isolated", value_parser = crate::admin::parse_on_off)]
        isolated: Option<bool>,
        /// Projects keep the workspace's server: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        home_locked: Option<bool>,
        /// Nothing Silt holds of its threads leaves this machine, and only local models answer: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        local_only: Option<bool>,
        /// In a Local only workspace, agents' history tools read every project in it, not only their own: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        wide_history: Option<bool>,
        /// The page connectors its pages may call, comma-separated, or `any`
        #[arg(long)]
        connectors: Option<String>,
        /// What its sandboxes connect to: none, gateway, full (the default) or host
        #[arg(long)]
        network: Option<String>,
        /// The hosts "through Silt only" reaches, comma-separated, such as `*.github.com`, or `any`
        #[arg(long)]
        hosts: Option<String>,
        /// Which devices its sandboxes get: none, relayed, named or all_usb
        #[arg(long)]
        devices: Option<String>,
        /// The device nodes `--devices named` gives, comma-separated, or `none`
        #[arg(long)]
        device_nodes: Option<String>,
        /// Address ranges its sandboxes reach past those always blocked, such as a VPN's, comma-separated, or `none`
        #[arg(long)]
        ranges: Option<String>,
        /// Ports on this machine its sandboxes reach, comma-separated, or `none`
        #[arg(long)]
        ports: Option<String>,
        /// More folders for its sandboxes, comma-separated, each read-only or with `:rw` writable, or `none`
        #[arg(long)]
        paths: Option<String>,
        /// Its sandboxes get the GPU: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        gpu: Option<bool>,
        /// Its sandboxes get the Docker socket, which is root on this machine: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        docker: Option<bool>,
        /// Its sandboxes get your SSH agent, which signs as you anywhere: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        ssh_agent: Option<bool>,
        /// What may never be handed its sandboxes, comma-separated: ranges, ports, paths, gpu, docker, ssh_agent; or `none`
        #[arg(long)]
        refuse: Option<String>,
        /// Agents' own sandboxes may run inside Silt's: on or off
        #[arg(long, value_parser = crate::admin::parse_on_off)]
        nested: Option<bool>,
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
        WorkspaceCommand::Policy {
            workspace,
            agents,
            custom_agents,
            max_preset,
            sandbox,
            silt_sandbox,
            backend,
            isolated,
            home_locked,
            connectors,
            local_only,
            wide_history,
            network,
            hosts,
            devices,
            device_nodes,
            ranges,
            ports,
            paths,
            gpu,
            docker,
            ssh_agent,
            refuse,
            nested,
        } => {
            let workspace = find_mut(all, &workspace)?;
            let policy = &mut workspace.policy;
            let reach = parse_reach(network.as_deref(), devices.as_deref(), refuse.as_deref())?;
            if network.is_some() {
                policy.network = reach.network;
            }
            if devices.is_some() {
                policy.devices = reach.devices;
            }
            if refuse.is_some() {
                policy.refuse = reach.refuse;
            }
            if let Some(hosts) = hosts {
                policy.hosts = match hosts.trim() {
                    "any" | "" => None,
                    list => Some(entries(list).map(|host| host.to_ascii_lowercase()).collect()),
                };
            }
            if let Some(nodes) = device_nodes {
                policy.device_nodes = entries(&nodes).collect();
            }
            if let Some(ranges) = ranges {
                policy.ranges = entries(&ranges)
                    .map(|range| match silt_workspace::Cidr::parse(&range) {
                        Some(cidr) => Ok(cidr.to_string()),
                        None => Err(fail(format!("{range} isn't an address range, such as 10.20.0.0/16"))),
                    })
                    .collect::<Result<_, _>>()?;
            }
            if let Some(ports) = ports {
                policy.ports = entries(&ports)
                    .map(|port| port.parse::<u16>().map_err(|_| fail(format!("{port} isn't a port"))))
                    .collect::<Result<_, _>>()?;
            }
            if let Some(paths) = paths {
                policy.paths = entries(&paths)
                    .map(|path| match path.strip_suffix(":rw") {
                        Some(path) => (expand_home(path), true),
                        None => (expand_home(path.strip_suffix(":ro").unwrap_or(&path)), false),
                    })
                    .collect();
            }
            for (setting, value) in [(&mut policy.gpu, gpu), (&mut policy.docker, docker), (&mut policy.ssh_agent, ssh_agent)] {
                if let Some(value) = value {
                    *setting = value;
                }
            }
            if let Some(nested) = nested {
                policy.no_nested = !nested;
            }
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
            if let Some(silt_sandbox) = silt_sandbox {
                policy.silt_sandbox = silt_sandbox;
            }
            if backend.is_some() {
                policy.backend = backend;
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
            if let Some(wide) = wide_history {
                policy.wide_history = wide;
            }
            if let Some(locked) = home_locked {
                policy.home_locked = locked;
            }
            if let Some(connectors) = connectors {
                policy.connectors = match connectors.trim() {
                    "any" | "" => None,
                    list => Some(list.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect()),
                };
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

/// A comma-separated list's entries; `none` or nothing is an empty one.
fn entries(text: &str) -> impl Iterator<Item = String> + '_ {
    let text = if text.trim() == "none" { "" } else { text };
    text.split(',').map(str::trim).filter(|entry| !entry.is_empty()).map(str::to_owned)
}

fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), silt_os::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path.to_owned(),
    }
}

pub(crate) fn parse_backend(name: &str) -> Result<silt_workspace::Backend, String> {
    silt_workspace::Backend::parse(name).ok_or_else(|| format!("no backend {name:?}: bubblewrap is the only one so far"))
}

/// The reach rules a command names: the network, the devices, and what may
/// never be handed over.
pub(crate) struct ParsedReach {
    pub network: Option<silt_workspace::Network>,
    pub devices: Option<silt_workspace::Devices>,
    pub refuse: BTreeSet<silt_workspace::Extra>,
}

pub(crate) fn parse_reach(
    network: Option<&str>,
    devices: Option<&str>,
    refuse: Option<&str>,
) -> Result<ParsedReach, Exn<CliError>> {
    let network = match network {
        None | Some("any") => None,
        Some(name) => Some(
            silt_workspace::Network::parse(name)
                .ok_or_else(|| fail(format!("no network {name}: none, gateway, full or host")))?,
        ),
    };
    let devices = match devices {
        None | Some("any") => None,
        Some(name) => Some(
            silt_workspace::Devices::parse(name)
                .ok_or_else(|| fail(format!("no devices {name}: none, relayed, named or all_usb")))?,
        ),
    };
    let refuse = entries(refuse.unwrap_or_default())
        .map(|name| {
            silt_workspace::Extra::parse(&name)
                .ok_or_else(|| fail(format!("nothing called {name} to refuse: ranges, ports, paths, gpu, docker or ssh_agent")))
        })
        .collect::<Result<_, _>>()?;
    Ok(ParsedReach { network, devices, refuse })
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
        parts.push("other workspaces' files hidden".into());
    } else if policy.sandbox {
        parts.push("agent's sandbox required".into());
    }
    if policy.home_locked {
        parts.push("home locked".into());
    }
    if policy.local_only {
        parts.push(if policy.wide_history { "local only, history across it" } else { "local only, history by project" }.into());
    }
    if policy.silt_sandbox {
        parts.push(format!(
            "in Silt's sandbox ({}): {}",
            policy.backend.unwrap_or_default().label(),
            silt_workspace::Reach::of(policy).summary()
        ));
    }
    if !policy.refuse.is_empty() {
        let refused: Vec<&str> = policy.refuse.iter().map(|extra| extra.label()).collect();
        parts.push(format!("never: {}", refused.join(", ")));
    }
    if let Some(connectors) = &policy.connectors {
        parts.push(if connectors.is_empty() {
            "no page connector".to_owned()
        } else {
            format!("page connectors {}", connectors.iter().cloned().collect::<Vec<_>>().join(", "))
        });
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
    let backend = policy.backend.unwrap_or_default().label();
    println!(
        "  sandbox      {}",
        match (policy.silt_sandbox, policy.isolated, policy.sandbox) {
            (true, true, _) => {
                format!("Silt's ({backend}), hiding other workspaces' files: {}", silt_workspace::Reach::of(policy).summary())
            }
            (true, false, _) => format!("Silt's ({backend}): {}", silt_workspace::Reach::of(policy).summary()),
            (false, true, _) => "the agent's own, hiding other workspaces' files".to_owned(),
            (false, false, true) => "the agent's own".to_owned(),
            (false, false, false) => "not required".to_owned(),
        }
    );
    if let Some(org) = &shown.org_policy {
        println!("  from {}'s policy on {}: {}", org.org, org.server, policy_text(&org.policy));
    }
    Ok(())
}
