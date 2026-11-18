//! `silt plugin` and `silt approvals`: what a workspace's sandboxed agents
//! reach production through, and the person's answers to the calls that
//! wait for them (ADR 0093).

use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::PathBuf;

use clap::Subcommand;
use exn::{Exn, ResultExt as _};
use silt_daemon::paths;
use silt_daemon::plugins::{AccessView, ApprovalState, ApprovalView, GrantScope, Outcome, install, installed};
use silt_daemon::protocol::Request;

use crate::CliError;
use crate::commands::{call, fail};

#[derive(Debug, Subcommand)]
pub enum PluginCommand {
    /// Add a plugin from a git repository (`URL#tag` to pin one), showing everything it gets first
    Add {
        source: String,
        /// Turn it on in this workspace too; may be given more than once
        #[arg(long)]
        workspace: Vec<String>,
        /// Agree to what it gets without being asked
        #[arg(long)]
        yes: bool,
    },
    /// Add or rebuild a plugin from a folder you're developing
    Dev {
        folder: PathBuf,
        #[arg(long)]
        workspace: Vec<String>,
        /// Keep watching it, building it again whenever it changes, until stopped
        #[arg(long)]
        watch: bool,
    },
    /// Update a plugin from where it came from, showing what the new version adds
    Update {
        id: String,
        /// Agree to what it adds without being asked
        #[arg(long)]
        yes: bool,
    },
    /// Trust a publisher, so its plugins' actions start from their own defaults rather than asking
    Trust {
        publisher: String,
        /// Stop trusting it
        #[arg(long)]
        off: bool,
    },
    /// Set the patterns that pick one of a plugin's environments, such as `acme/*,acme-labs/*`; `none` removes it
    Environment { id: String, name: String, patterns: String },
    /// Say which of `silt credential`'s credentials is one of a plugin's own
    Credential {
        id: String,
        /// The plugin's own name for it, from its manifest
        credential: String,
        /// The credential `silt credential add` keeps
        name: String,
        /// For this environment only
        #[arg(long)]
        environment: Option<String>,
    },
    /// Allow, ask or never one of a plugin's actions; with `--where field=pattern`, only for calls that match
    Rule {
        id: String,
        action: String,
        /// allow, ask or never; `reset` removes your rule
        rule: String,
        #[arg(long = "where")]
        matching: Option<String>,
    },
    /// Set one of a plugin's settings to a JSON value; `null` resets it
    Set {
        id: String,
        key: String,
        value: String,
        /// For this environment, for a setting that's set per environment
        #[arg(long)]
        environment: Option<String>,
        /// For this workspace, for a setting that's set per workspace
        #[arg(long)]
        workspace: Option<String>,
    },
    /// Keep one of a plugin's credentials as its own secret, read from stdin into the keyring, or printed by `--command` when it's needed; Silt adds it to that plugin's requests alone
    Secret {
        id: String,
        /// The plugin's own name for it, from its manifest
        credential: String,
        /// For this environment only
        #[arg(long)]
        environment: Option<String>,
        /// A command run on this machine that prints it, such as `gh auth token`
        #[arg(long)]
        command: Option<String>,
        /// Forget it
        #[arg(long)]
        remove: bool,
    },
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
}

fn core_error(error: silt_daemon::CoreError) -> Exn<CliError> {
    fail(error.to_string())
}

pub fn run(command: PluginCommand) -> Result<(), Exn<CliError>> {
    let data = paths::data_dir();
    installed::ensure_shipped(&data).map_err(core_error)?;
    match command {
        PluginCommand::Add { source, workspace, yes } => add_from_git(&data, &source, &workspace, yes)?,
        PluginCommand::Dev { folder, workspace, watch } => {
            let folder = std::fs::canonicalize(&folder).or_raise(|| CliError(format!("no folder {}", folder.display())))?;
            let build = |workspace: &[String]| -> Result<(), Exn<CliError>> {
                let package = runtime()?.block_on(install::package(&folder, true)).map_err(core_error)?;
                let source = installed::PluginSource::Dev { path: folder.to_string_lossy().into_owned() };
                let record = install::install(&data, &package, source, workspace, true).map_err(core_error)?;
                println!("{} {} built from {} (dev)", record.id, record.version, folder.display());
                Ok(())
            };
            build(&workspace)?;
            if !watch {
                return Ok(());
            }
            let mut seen = install::changed_at(&folder);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                let now = install::changed_at(&folder);
                if now != seen {
                    seen = now;
                    // A broken build leaves the last good one running.
                    if let Err(error) = build(&[]) {
                        eprintln!("silt: {error:?}");
                    }
                }
            }
        }
        PluginCommand::Update { id, yes } => update(&data, &id, yes)?,
        PluginCommand::Trust { publisher, off } => {
            installed::set_trusted(&data, &publisher, !off).map_err(core_error)?;
            println!(
                "{publisher} is {}",
                if off {
                    "no longer trusted: its plugins' actions ask"
                } else {
                    "trusted: its plugins' actions start from their own defaults"
                }
            );
        }
        PluginCommand::Environment { id, name, patterns } => {
            let patterns: Vec<String> =
                if patterns == "none" { Vec::new() } else { patterns.split(',').map(str::to_owned).collect() };
            installed::set_environment(&data, &id, &name, &patterns).map_err(core_error)?;
            println!("{id}'s {name}: {}", if patterns.is_empty() { "removed".to_owned() } else { patterns.join(", ") });
        }
        PluginCommand::Credential { id, credential, name, environment } => {
            let manifest = installed::manifest(&data, &id).map_err(core_error)?;
            if !manifest.credentials.iter().any(|spec| spec.id == credential) {
                return Err(fail(format!("{id} has no credential {credential}")));
            }
            let key = environment.as_ref().map_or(credential.clone(), |environment| format!("{credential}@{environment}"));
            installed::update(&data, &id, |record| {
                record.credentials.insert(key.clone(), name.clone());
                Ok(())
            })
            .map_err(core_error)?;
            println!("{id}'s {key} is the credential {name}");
        }
        PluginCommand::Rule { id, action, rule, matching } => set_rule(&data, &id, &action, &rule, matching.as_deref())?,
        PluginCommand::Secret { id, credential, environment, command, remove } => {
            secret(&data, &id, &credential, environment.as_deref(), command.as_deref(), remove)?;
        }
        PluginCommand::Set { id, key, value, environment, workspace } => {
            let value: serde_json::Value = serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));
            installed::set_setting(&data, &id, &key, &value, environment.as_deref(), workspace.as_deref()).map_err(core_error)?;
            println!("{id}'s {key} is {}", if value.is_null() { "reset" } else { "set" });
        }
        PluginCommand::List => {
            for record in installed::load(&data) {
                let source = match &record.source {
                    installed::PluginSource::Shipped => "shipped with Silt".to_owned(),
                    installed::PluginSource::Git { url, commit, .. } => {
                        format!("from {url} at {}", &commit[..commit.len().min(7)])
                    }
                    installed::PluginSource::Dev { path } => format!("dev, from {path}"),
                    installed::PluginSource::Marketplace { server } => format!("from the marketplace at {server}"),
                    installed::PluginSource::Org { org, .. } => format!("from {org}'s policy"),
                };
                let on = if record.workspaces.is_empty() {
                    "on in no workspace".to_owned()
                } else if record.workspaces.iter().any(|on| on == "*") {
                    "on in every workspace".to_owned()
                } else {
                    format!("on in {}", record.workspaces.join(", "))
                };
                println!("{} {}  {source}  {on}", record.id, record.version);
                let granted: Vec<&str> = record.granted.iter().map(String::as_str).collect();
                println!("  granted: {}", if granted.is_empty() { "nothing".to_owned() } else { granted.join(", ") });
                for (name, patterns) in &record.environments {
                    println!("  {name}: {}", patterns.join(", "));
                }
            }
        }
        PluginCommand::Remove { name } => {
            install::remove(&data, &name).map_err(core_error)?;
            println!("removed {name}");
        }
        PluginCommand::Enable { name, workspace } => turn(&data, &name, &workspace, true)?,
        PluginCommand::Disable { name, workspace } => turn(&data, &name, &workspace, false)?,
    }
    Ok(())
}

/// Keeps plugin `id`'s credential `credential` as its own secret, sent by
/// Silt with its requests alone (ADR 0103).
fn secret(
    data: &std::path::Path,
    id: &str,
    credential: &str,
    environment: Option<&str>,
    command: Option<&str>,
    remove: bool,
) -> Result<(), Exn<CliError>> {
    use silt_daemon::credentials::{self, Source, StoredCredential};
    let manifest = installed::manifest(data, id).map_err(core_error)?;
    let spec = manifest
        .credentials
        .iter()
        .find(|spec| spec.id == credential && spec.same_as.is_none())
        .ok_or_else(|| fail(format!("{id} has no credential {credential} of its own")))?;
    if environment.is_some() && !spec.per_environment {
        return Err(fail(format!("{credential} is the same in every environment")));
    }
    let mapped = environment.map_or_else(|| credential.to_owned(), |environment| format!("{credential}@{environment}"));
    let name = format!("plugin:{id}:{mapped}");
    let mut all = credentials::load(data);
    all.retain(|stored| stored.name != name);
    if remove {
        // One printed by a command was never in the keyring.
        let _ = credentials::set_secret(&name, None);
        credentials::save(data, &all).map_err(|error| fail(error.to_string()))?;
        installed::update(data, id, |record| {
            record.credentials.retain(|key, value| !(key == &mapped && value == &name));
            Ok(())
        })
        .map_err(core_error)?;
        println!("{id}'s {credential} is forgotten");
        return Ok(());
    }
    let source = match command {
        Some(command) => Source::Command { argv: command.split_whitespace().map(str::to_owned).collect() },
        None => {
            let mut secret = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut secret)
                .or_raise(|| CliError("cannot read the secret".into()))?;
            let secret = secret.trim();
            if secret.is_empty() {
                return Err(fail("give the secret on stdin, or `--command` that prints it"));
            }
            credentials::set_secret(&name, Some(secret)).map_err(|error| fail(error.to_string()))?;
            Source::Keyring
        }
    };
    all.push(StoredCredential {
        name: name.clone(),
        workspace: "*".into(),
        host: spec.hosts.first().cloned().unwrap_or_default(),
        path: "/".into(),
        header: "authorization".into(),
        scheme: String::new(),
        source,
        placeholder: None,
        plugin: Some(id.to_owned()),
    });
    credentials::save(data, &all).map_err(|error| fail(error.to_string()))?;
    installed::update(data, id, |record| {
        record.credentials.insert(mapped.clone(), name.clone());
        Ok(())
    })
    .map_err(core_error)?;
    println!("{id}'s {credential} is kept; Silt adds it to {id}'s requests to {} alone", spec.hosts.join(", "));
    Ok(())
}

/// An org's plugin policy from `silt org policy`'s flags.
pub(crate) fn org_policy(
    install: &[String],
    allow: Option<&str>,
    pin: &[String],
    rules: &[String],
    refuse_opaque: bool,
    refuse_reach: bool,
) -> Result<silt_workspace::PluginPolicy, Exn<CliError>> {
    let install = install
        .iter()
        .map(|entry| {
            let shape = || fail(format!("{entry}: give a plugin to install as <id>=<git-url>#<commit>"));
            let (id, rest) = entry.split_once('=').ok_or_else(shape)?;
            let (git, commit) = rest.rsplit_once('#').ok_or_else(shape)?;
            if !commit.chars().all(|c| c.is_ascii_hexdigit()) || commit.len() < 7 {
                return Err(fail(format!("{entry}: name the commit by its id, so every member builds the same")));
            }
            Ok(silt_workspace::OrgPlugin { id: id.to_owned(), git: git.to_owned(), commit: commit.to_owned() })
        })
        .collect::<Result<_, _>>()?;
    let pinned = pin
        .iter()
        .map(|entry| {
            entry
                .split_once('=')
                .map(|(id, version)| (id.to_owned(), version.to_owned()))
                .ok_or_else(|| fail(format!("{entry}: give <id>=<version>")))
        })
        .collect::<Result<_, _>>()?;
    let rules = rules.iter().map(|rule| silt_workspace::OrgPluginRule::parse(rule).map_err(fail)).collect::<Result<_, _>>()?;
    Ok(silt_workspace::PluginPolicy {
        install,
        allow: allow.map(|list| list.split(',').map(str::trim).filter(|entry| !entry.is_empty()).map(str::to_owned).collect()),
        pinned,
        refuse_opaque,
        refuse_reach,
        rules,
    })
}

fn runtime() -> Result<tokio::runtime::Runtime, Exn<CliError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .or_raise(|| CliError("cannot start the async runtime".into()))
}

/// Whether the person agrees to `lines`: asked at a terminal, else only
/// with `--yes`.
fn agreed(lines: &[String], yes: bool) -> Result<bool, Exn<CliError>> {
    for line in lines {
        println!("{line}");
    }
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(fail("agree to what it gets with --yes, or run this at a terminal"));
    }
    print!("Add it? [y/N] ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer).ok();
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}

fn add_from_git(data: &std::path::Path, source: &str, workspaces: &[String], yes: bool) -> Result<(), Exn<CliError>> {
    let runtime = runtime()?;
    let preview = runtime.block_on(install::preview(data, source)).map_err(core_error)?;
    if !agreed(&preview.consent, yes)? {
        println!("nothing added");
        return Ok(());
    }
    let record = runtime.block_on(install::add_from_git(data, source, &preview.commit, workspaces)).map_err(core_error)?;
    println!("added {} {} at {}", record.id, record.version, &preview.commit[..preview.commit.len().min(7)]);
    if record.workspaces.is_empty() {
        println!("turn it on with `silt plugin enable {} --workspace <id>`", record.id);
    }
    Ok(())
}

fn update(data: &std::path::Path, id: &str, yes: bool) -> Result<(), Exn<CliError>> {
    let record = installed::find(data, id).ok_or_else(|| fail(format!("no plugin {id}")))?;
    let package = match &record.source {
        installed::PluginSource::Git { url, follow, commit: old } => {
            let checkout = tempfile::tempdir().or_raise(|| CliError("cannot make a folder to clone into".into()))?;
            let commit = install::clone(url, follow.as_deref(), checkout.path()).map_err(core_error)?;
            if commit == *old {
                println!("{id} is up to date at {}", &commit[..commit.len().min(7)]);
                return Ok(());
            }
            match install::commits_between(checkout.path(), old, &commit) {
                Ok(commits) => {
                    println!(
                        "{} since {}:",
                        if commits.len() == 1 { "1 commit".to_owned() } else { format!("{} commits", commits.len()) },
                        &old[..old.len().min(7)]
                    );
                    for line in commits {
                        println!("  {line}");
                    }
                }
                Err(_) => println!(
                    "{} isn't an ancestor of {}: its history was rewritten",
                    &old[..old.len().min(7)],
                    &commit[..commit.len().min(7)]
                ),
            }
            let package = runtime()?.block_on(install::package(checkout.path(), false)).map_err(core_error)?;
            (package, installed::PluginSource::Git { url: url.clone(), commit, follow: follow.clone() })
        }
        installed::PluginSource::Dev { path } => {
            let package = runtime()?.block_on(install::package(std::path::Path::new(path), true)).map_err(core_error)?;
            (package, record.source.clone())
        }
        installed::PluginSource::Shipped => return Err(fail(format!("{id} updates with Silt"))),
        installed::PluginSource::Marketplace { .. } | installed::PluginSource::Org { .. } => {
            return Err(fail(format!("{id} updates from where it came from; `silt plugin search` finds it")));
        }
    };
    let (package, source) = package;
    let before = installed::manifest(data, id).map_err(core_error)?;
    let added = before.added_in(&package.manifest);
    if !added.is_empty() {
        let mut lines = vec![format!("{} {} asks for more:", package.manifest.name, package.manifest.version)];
        lines.extend(added.iter().map(|line| format!("  {line}")));
        if !agreed(&lines, yes)? {
            println!("not updated; {} {} keeps running", record.id, record.version);
            return Ok(());
        }
    }
    let updated = install::install(data, &package, source, &[], true).map_err(core_error)?;
    println!("{} is now {}", updated.id, updated.version);
    Ok(())
}

fn set_rule(data: &std::path::Path, id: &str, action: &str, rule: &str, matching: Option<&str>) -> Result<(), Exn<CliError>> {
    let parsed = match matching {
        Some(matching) => {
            let (field, pattern) =
                matching.split_once('=').ok_or_else(|| fail("--where is `field=pattern`, such as `branch=ada/*`"))?;
            Some((field.to_owned(), pattern.to_owned()))
        }
        None => None,
    };
    let decided = match rule {
        "allow" => Some(silt_plugin::Rule::Allow),
        "ask" => Some(silt_plugin::Rule::Ask),
        "never" => Some(silt_plugin::Rule::Never),
        "reset" => None,
        other => return Err(fail(format!("{other} isn't allow, ask, never or reset"))),
    };
    installed::set_rule(data, id, action, decided, parsed).map_err(core_error)?;
    println!("{id}'s {action}{}: {rule}", matching.map(|matching| format!(" where {matching}")).unwrap_or_default());
    Ok(())
}

fn turn(data: &std::path::Path, name: &str, workspace: &str, on: bool) -> Result<(), Exn<CliError>> {
    let every: Vec<String> = silt_daemon::workspaces::load(data)
        .map(|all| all.workspaces.iter().map(|workspace| workspace.id.as_str().to_owned()).collect())
        .unwrap_or_else(|| vec!["default".to_owned()]);
    installed::turn(data, name, workspace, on, &every).map_err(core_error)?;
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
        Some("workspace") => Some(GrantScope::Workspace),
        Some("always") => Some(GrantScope::Always),
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
            Some(GrantScope::Workspace) => print!("Run exactly this, and make its folder one of the workspace's own? [y/N] "),
            Some(GrantScope::Always) => {
                print!("Run exactly this, and allow calls like it from now on, by a rule of yours? [y/N] ")
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
    if !access.folders.is_empty() {
        println!("Folders");
        for folder in &access.folders {
            let access = if folder.writable { "read and write" } else { "read-only" };
            let scope = if folder.run_only { "its run" } else { "the thread" };
            println!("  {}  {access}, for {scope}", folder.path);
        }
    }
    if !access.devices.is_empty() {
        println!("Devices relayed");
        for device in &access.devices {
            println!("  {}  {}", device.device, device.kind);
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
