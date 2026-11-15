//! Workspaces on this machine (ADR 0065): which one a project and a family
//! are in, the server that decides their home, and the rules for the agents
//! started in them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use silt_types::FamilyId;
use silt_workspace::{
    InvalidWorkspaces, MatchRule, OrgPolicy, Policy, Rules, ServerBinding, Source, Why, Workspace, WorkspaceId, Workspaces,
};

use crate::core::{Core, CoreError};
use crate::homes::Home;
use crate::protocol::{Preset, WorkspaceProject, WorkspaceView, WorkspacesView};

const FILE: &str = "workspaces.json";

/// The workspaces kept in `data_dir`; `None` when the person never made one,
/// so the one Default workspace follows the default sign-in.
pub fn load(data_dir: &Path) -> Option<Workspaces> {
    let bytes = std::fs::read(data_dir.join(FILE)).ok()?;
    match serde_json::from_slice::<Workspaces>(&bytes).map(tidied) {
        Ok(workspaces) if workspaces.validate().is_ok() => Some(workspaces),
        Ok(workspaces) => {
            tracing::warn!(error = %workspaces.validate().unwrap_err(), "ignoring {FILE}");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "ignoring {FILE}");
            None
        }
    }
}

fn tidied(mut workspaces: Workspaces) -> Workspaces {
    workspaces.tidy();
    workspaces
}

pub(crate) fn save(data_dir: &Path, workspaces: &Workspaces) -> Result<(), CoreError> {
    workspaces.validate().map_err(invalid)?;
    let bytes = serde_json::to_vec_pretty(workspaces).map_err(std::io::Error::other)?;
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(FILE);
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(partial, path)?;
    Ok(())
}

fn invalid(error: InvalidWorkspaces) -> CoreError {
    CoreError::Invalid(error.to_string())
}

/// A folder rule as the person wrote it, absolute and `/`-separated, with
/// `~` for their home.
pub fn folder_rule(folder: &str) -> Result<MatchRule, CoreError> {
    let expanded = match folder.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            let home = silt_os::home_dir().ok_or_else(|| CoreError::Invalid("no home folder to expand ~ in".into()))?;
            format!("{}{rest}", home.to_string_lossy().trim_end_matches('/'))
        }
        _ => folder.to_owned(),
    };
    if !Path::new(&expanded).is_absolute() && !expanded.starts_with('/') {
        return Err(CoreError::Invalid(format!("the folder {folder} must be absolute or start with ~")));
    }
    Ok(MatchRule::folder(&expanded.replace('\\', "/")))
}

/// Why a project is in its workspace, as the Projects page says it.
pub(crate) fn why_text(why: &Why) -> String {
    match why {
        Why::Chosen => "set by you".into(),
        Why::Rule(MatchRule::Remote(pattern) | MatchRule::Folder(pattern)) => format!("by rule {pattern}"),
        Why::Default => "matched nothing".into(),
    }
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Core {
    /// Every workspace: those the person made, or the one Default with the
    /// default sign-in.
    pub(crate) fn workspaces(&self) -> Workspaces {
        self.workspaces.clone().unwrap_or_else(|| {
            Workspaces::single(self.default_remote().map(|remote| ServerBinding { url: remote.config.url.clone(), org: None }))
        })
    }

    /// The workspace of `repo`, cloned at `clone` if it is here.
    pub(crate) fn project_workspace(&self, repo: &str, clone: Option<&Path>) -> (Workspace, Why) {
        let all = self.workspaces();
        let clone = clone.map(slashed).or_else(|| self.clones.get(repo).map(|path| slashed(path)));
        let resolved = all.resolve(repo, clone.as_deref(), self.project_workspaces.get(repo));
        (resolved.workspace.clone(), resolved.why)
    }

    /// The workspace a family was made in, or for one from before
    /// workspaces, its project's.
    pub(crate) fn family_workspace(&self, family: FamilyId) -> Workspace {
        let all = self.workspaces();
        if let Some(workspace) = self.family_workspaces.get(&family).and_then(|id| all.get(id)) {
            return workspace.clone();
        }
        match self.threads.values().find(|record| record.family == family) {
            Some(record) => self.project_workspace(&record.repo, Some(&record.repo_root)).0,
            None => all.default_workspace().clone(),
        }
    }

    /// The workspace a new family started in `repo` is made in: a chat's
    /// own, else its project's.
    pub(crate) fn new_family_workspace(&self, repo: &silt_git::Repo, project: &str) -> WorkspaceId {
        let all = self.workspaces();
        if repo.is_chat()
            && let Some(id) = repo.chat_workspace().and_then(|id| WorkspaceId::new(id).ok())
            && all.get(&id).is_some()
        {
            return id;
        }
        self.project_workspace(project, Some(repo.root())).0.id
    }

    pub(crate) async fn pin_workspace(&mut self, family: FamilyId, workspace: &WorkspaceId) -> Result<(), CoreError> {
        self.store.set_family_workspace(family, workspace.as_str()).await?;
        self.family_workspaces.insert(family, workspace.clone());
        Ok(())
    }

    /// Where a new family of a project in `workspace` syncs.
    pub(crate) fn workspace_home(&self, workspace: &Workspace) -> Option<Home> {
        if self.workspaces.is_none() {
            // As before workspaces: the default server, or the first signed in to.
            return self.default_remote().map(|remote| Home::Server(remote.config.url.clone()));
        }
        Some(match &workspace.server {
            Some(binding) => Home::Server(binding.url.clone()),
            None => Home::Local,
        })
    }

    /// The org policy that applies in `workspace`: `Err` while it's bound to
    /// an org whose policy this machine hasn't fetched yet.
    fn org_policy(&self, workspace: &Workspace) -> Option<Result<&OrgPolicy, Source>> {
        let binding = workspace.server.as_ref()?;
        let org = binding.org.as_ref()?;
        Some(
            self.org_policies
                .get(&(binding.url.clone(), org.clone()))
                .ok_or_else(|| Source::Org { org: org.clone(), server: binding.url.clone() }),
        )
    }

    /// The rules of `workspace`: its own and its org's.
    pub(crate) fn rules<'a>(&'a self, workspace: &'a Workspace) -> Rules<'a> {
        Rules { workspace: &workspace.name, local: &workspace.policy, org: self.org_policy(workspace) }
    }

    /// The rules in effect for the agent of a thread's family.
    pub(crate) fn effective_policy(&self, workspace: &Workspace) -> Policy {
        self.rules(workspace).effective()
    }

    pub(crate) fn workspaces_view(&self) -> WorkspacesView {
        let all = self.workspaces();
        let mut projects: BTreeSet<String> = self.clones.keys().cloned().collect();
        projects.extend(self.threads.values().map(|record| record.repo.clone()).filter(|repo| !repo.is_empty()));
        let placed: Vec<(String, Workspace, Why)> = projects
            .into_iter()
            .map(|repo| {
                let (workspace, why) = self.project_workspace(&repo, None);
                (repo, workspace, why)
            })
            .collect();
        WorkspacesView {
            default: all.default.clone(),
            saved: self.workspaces.is_some(),
            workspaces: all
                .workspaces
                .iter()
                .map(|workspace| {
                    let rules = self.rules(workspace);
                    WorkspaceView {
                        workspace: workspace.clone(),
                        org_policy: rules.org.as_ref().and_then(|org| org.as_ref().ok().map(|org| (*org).clone())),
                        waiting_for_policy: matches!(rules.org, Some(Err(_))),
                        effective: rules.effective(),
                        projects: placed
                            .iter()
                            .filter(|(_, placed, _)| placed.id == workspace.id)
                            .map(|(repo, _, why)| WorkspaceProject { repo: repo.clone(), why: why.clone() })
                            .collect(),
                    }
                })
                .collect(),
        }
    }

    /// Replaces the workspaces with `workspaces`, checked first.
    pub(crate) fn set_workspaces(&mut self, workspaces: Workspaces) -> Result<(), CoreError> {
        let workspaces = tidied(workspaces);
        for workspace in workspaces.workspaces.iter().filter(|workspace| workspace.policy.local_only) {
            let syncing = self.family_workspaces.iter().filter(|(_, id)| **id == workspace.id).find_map(|(family, _)| match self
                .homes
                .get(family)
            {
                Some(Home::Server(url)) => Some(url.clone()),
                _ => None,
            });
            if let Some(url) = syncing {
                return Err(CoreError::Invalid(format!(
                    "{} has threads that sync with {url}; a Local only workspace keeps everything on this machine, so make a new one for them",
                    workspace.name
                )));
            }
        }
        for workspace in &workspaces.workspaces {
            if let Some(binding) = &workspace.server
                && !self.remotes.iter().any(|remote| remote.config.url == binding.url)
            {
                return Err(CoreError::Invalid(format!(
                    "not signed in to {url}; run `silt auth login --server {url}` first",
                    url = binding.url
                )));
            }
        }
        save(&self.data_dir, &workspaces)?;
        self.workspaces = Some(workspaces);
        self.share_workspace_homes();
        // Fetches a newly bound org's policy now rather than with the next listing.
        self.last_listings.clear();
        self.drive_sync();
        self.announce(crate::protocol::BusEvent::Threads);
        Ok(())
    }

    /// Puts `repo` in `workspace`, or back under the rules with `None`.
    /// Refused when its threads on a server would end up in a workspace with
    /// another one.
    pub(crate) async fn assign_project(&mut self, repo: &str, workspace: Option<WorkspaceId>) -> Result<String, CoreError> {
        let all = self.workspaces();
        let target = match &workspace {
            Some(id) => all.get(id).cloned().ok_or_else(|| CoreError::Invalid(format!("no workspace {id}")))?,
            None => {
                let rules_only = all.resolve(repo, self.clones.get(repo).map(|path| slashed(path)).as_deref(), None);
                rules_only.workspace.clone()
            }
        };
        let families: BTreeSet<FamilyId> =
            self.threads.values().filter(|record| record.repo == repo).map(|record| record.family).collect();
        let servers: BTreeSet<String> = families
            .iter()
            .filter_map(|family| match self.homes.get(family) {
                Some(Home::Server(url)) => Some(url.clone()),
                Some(Home::Local) => None,
                None => self.default_remote().map(|remote| remote.config.url.clone()),
            })
            .collect();
        let servers: Vec<String> = servers.into_iter().collect();
        silt_workspace::check_move(repo, &servers, &target.name, target.server.as_ref().map(|binding| binding.url.as_str()))
            .map_err(|refusal| CoreError::Invalid(refusal.to_string()))?;
        self.store.set_project_workspace(repo, workspace.as_ref().map(WorkspaceId::as_str)).await?;
        match &workspace {
            Some(id) => self.project_workspaces.insert(repo.to_owned(), id.clone()),
            None => self.project_workspaces.remove(repo),
        };
        let home = self.workspace_home(&target);
        for family in families {
            self.pin_workspace(family, &target.id).await?;
            if let (Some(home @ Home::Server(_)), Some(Home::Local)) = (&home, self.homes.get(&family)) {
                self.set_home(family, home.clone()).await?;
            }
        }
        self.share_workspace_homes();
        self.drive_sync();
        self.announce(crate::protocol::BusEvent::Threads);
        Ok(format!("{repo} is in {}", target.name))
    }

    /// Tells the agent adapters which agent homes each workspace's folders
    /// use, so sessions there are found where their agent keeps them.
    pub(crate) fn share_workspace_homes(&self) {
        let all = self.workspaces();
        let homes_of = |workspace: &Workspace| -> Vec<(String, PathBuf)> {
            workspace
                .env
                .iter()
                .filter(|(name, _)| silt_agents::HOME_VARIABLES.contains(&name.as_str()) && name.as_str() != "HOME")
                .map(|(name, value)| (name.clone(), PathBuf::from(value)))
                .collect()
        };
        let mut homes = Vec::new();
        for workspace in &all.workspaces {
            for rule in &workspace.rules {
                if let MatchRule::Folder(folder) = rule {
                    homes.extend(homes_of(workspace).into_iter().map(|(name, home)| (PathBuf::from(folder), name, home)));
                }
            }
        }
        for (repo, path) in &self.clones {
            let (workspace, _) = self.project_workspace(repo, Some(path));
            homes.extend(homes_of(&workspace).into_iter().map(|(name, home)| (path.clone(), name, home)));
        }
        silt_agents::set_workspace_homes(homes);
    }

    /// Whether a workspace is bound to an org on `server`, whose policy is
    /// then fetched with its listing.
    pub(crate) fn binds_an_org(&self, server: &str) -> bool {
        self.workspaces.as_ref().is_some_and(|all| {
            all.workspaces
                .iter()
                .any(|workspace| workspace.server.as_ref().is_some_and(|binding| binding.url == server && binding.org.is_some()))
        })
    }

    /// Keeps the org policies `server` answered with, for the orgs a
    /// workspace is bound to.
    pub(crate) async fn org_policies_fetched(&mut self, server: &str, policies: Vec<(String, Policy)>) {
        let bound: BTreeSet<String> = self
            .workspaces()
            .workspaces
            .iter()
            .filter_map(|workspace| workspace.server.as_ref())
            .filter(|binding| binding.url == server)
            .filter_map(|binding| binding.org.clone())
            .collect();
        let mut changed = false;
        for (org, policy) in policies.into_iter().filter(|(org, _)| bound.contains(org)) {
            let key = (server.to_owned(), org.clone());
            if self.org_policies.get(&key).is_some_and(|kept| kept.policy == policy) {
                continue;
            }
            let json = serde_json::to_string(&policy).unwrap_or_else(|_| "{}".into());
            if let Err(error) = self.store.set_org_policy(server, &org, &json, crate::core::now_ms()).await {
                tracing::warn!(%error, "could not keep an org's policy");
                continue;
            }
            self.org_policies.insert(key, OrgPolicy { org, server: server.to_owned(), policy });
            changed = true;
        }
        if changed {
            self.announce(crate::protocol::BusEvent::Threads);
        }
    }

    pub(crate) async fn load_workspaces(&mut self) -> Result<(), CoreError> {
        self.workspaces = load(&self.data_dir);
        let ids = |text: String| WorkspaceId::new(text).ok();
        self.project_workspaces =
            self.store.project_workspaces().await?.into_iter().filter_map(|(repo, id)| Some((repo, ids(id)?))).collect();
        self.family_workspaces =
            self.store.family_workspaces().await?.into_iter().filter_map(|(family, id)| Some((family, ids(id)?))).collect();
        self.org_policies = self
            .store
            .org_policies()
            .await?
            .into_iter()
            .filter_map(|(server, org, policy)| {
                let policy = serde_json::from_str::<Policy>(&policy).ok()?;
                Some(((server.clone(), org.clone()), OrgPolicy { org, server, policy }))
            })
            .collect();
        Ok(())
    }

    /// Makes a token for an agent's run in `thread`, kept by its hash, for
    /// its history tools to read its workspace only.
    pub(crate) async fn mint_run_token(
        &self,
        thread: silt_types::ThreadId,
        workspace: &WorkspaceId,
        project: Option<&str>,
    ) -> Result<String, CoreError> {
        mint_run_token(&self.store, thread, workspace, project).await
    }

    /// What a scoped request's token may read, if it asks only what history
    /// tools ask.
    pub(crate) async fn scope_of(&self, token: &str, asking: &crate::protocol::Request) -> Result<Scope, CoreError> {
        use crate::protocol::Request;
        if !matches!(
            asking,
            Request::HistoryRead { .. }
                | Request::HistorySearch { .. }
                | Request::FileHistory { .. }
                | Request::InFlight { .. }
                | Request::Search { .. }
        ) {
            return Err(CoreError::Invalid("an agent's history tools may only read history".into()));
        }
        let hash = blake3::hash(token.as_bytes());
        let made = self
            .store
            .run_token(hash.as_bytes())
            .await?
            .ok_or_else(|| CoreError::Invalid("this run's token isn't known here".into()))?;
        let workspace = WorkspaceId::new(made.workspace).map_err(|error| CoreError::Invalid(error.to_string()))?;
        let by_project = self.workspaces().get(&workspace).is_some_and(|kept| self.effective_policy(kept).history_by_project());
        // A run whose project wasn't known reads none in a workspace kept by project.
        let project = by_project.then(|| made.project.unwrap_or_default());
        Ok(Scope { workspace, project })
    }

    /// The project a family belongs to, by its threads here.
    fn family_project(&self, family: FamilyId) -> Option<&str> {
        self.threads.values().find(|record| record.family == family).map(|record| record.repo.as_str())
    }

    /// Folders an agent in `workspace` may not show the person files from:
    /// Silt's data, and in an isolated workspace, the others' folders.
    pub(crate) fn artifact_denies(&self, workspace: &str) -> Vec<std::path::PathBuf> {
        let all = self.workspaces();
        let found = WorkspaceId::new(workspace.to_owned()).ok().and_then(|id| all.get(&id).cloned());
        let mut denied = match found {
            Some(workspace) => locks_for(&self.data_dir, &all, &workspace, &self.effective_policy(&workspace)).deny,
            None => Vec::new(),
        };
        denied.push(self.data_dir.clone());
        denied
    }

    /// Whether the request being handled may read a family: always, unless
    /// an agent's token scopes it to a workspace, or a project of one.
    pub(crate) fn in_scope(&self, family: FamilyId) -> bool {
        self.scope.as_ref().is_none_or(|scope| {
            self.family_workspace(family).id == scope.workspace && scope.allows_project(self.family_project(family))
        })
    }

    /// Search results the request may read: threads of its workspace, and
    /// threads known only from a server whose project is in it.
    pub(crate) fn scoped_results(&self, results: Vec<silt_proto::view::SearchResult>) -> Vec<silt_proto::view::SearchResult> {
        let Some(scope) = &self.scope else {
            return results;
        };
        results
            .into_iter()
            .filter(|result| {
                let here = self.threads.iter().find(|(thread, _)| thread.to_string() == result.thread);
                match (here, &result.repo) {
                    (Some((_, record)), _) => {
                        self.family_workspace(record.family).id == scope.workspace && scope.allows_project(Some(&record.repo))
                    }
                    (None, Some(repo)) => {
                        self.project_workspace(repo, None).0.id == scope.workspace && scope.allows_project(Some(repo))
                    }
                    (None, None) => false,
                }
            })
            .collect()
    }

    pub(crate) fn scope_allows_clone(&self, cwd: &Path) -> bool {
        let Some(scope) = &self.scope else {
            return true;
        };
        silt_git::Repo::discover(cwd)
            .ok()
            .and_then(|repo| {
                let project = repo.project().ok()?;
                Some(
                    self.project_workspace(&project, Some(repo.root())).0.id == scope.workspace
                        && scope.allows_project(Some(&project)),
                )
            })
            .unwrap_or(false)
    }

    /// The workspace an agent starts in: the thread's, the clone's project's,
    /// or for a new chat the one named, else the default.
    pub(crate) fn start_workspace(
        &self,
        thread: Option<&str>,
        clone: Option<&Path>,
        chat_workspace: Option<&str>,
    ) -> Result<Workspace, CoreError> {
        if let Some(selector) = thread {
            let thread = self.find_thread(selector)?;
            let family = self
                .threads
                .get(&thread)
                .map(|record| record.family)
                .ok_or_else(|| CoreError::Invalid("no such thread".into()))?;
            return Ok(self.family_workspace(family));
        }
        // A clone that isn't one is refused later, with what's wrong with it.
        if let Some((repo, project)) = clone
            .and_then(|clone| silt_git::Repo::discover(clone).ok())
            .and_then(|repo| Some((repo.project().ok()?, repo)).map(|(project, repo)| (repo, project)))
        {
            return Ok(self.project_workspace(&project, Some(repo.root())).0);
        }
        let all = self.workspaces();
        match chat_workspace {
            Some(id) => {
                let id = WorkspaceId::new(id).map_err(|error| CoreError::Invalid(error.to_string()))?;
                all.get(&id).cloned().ok_or_else(|| CoreError::Invalid(format!("no workspace {id}")))
            }
            None => Ok(all.default_workspace().clone()),
        }
    }

    /// Whether the web UI may start `agent` in `workspace` with `preset`
    /// and `mode` (ADR 0065). A preset nobody chose that the rules don't
    /// allow comes down to the most they do.
    pub(crate) fn check_acp_start(
        &self,
        workspace: &Workspace,
        agent: &silt_acp::registry::AgentInfo,
        preset: Option<Preset>,
        chosen: bool,
        mode: Option<&str>,
        on_local_model: bool,
    ) -> Result<Option<Preset>, CoreError> {
        let rules = self.rules(workspace);
        let policy = rules.effective();
        let refused = |refusal: silt_workspace::Refusal| CoreError::Invalid(refusal.to_string());
        let custom = silt_acp::registry::custom(&self.data_dir).iter().any(|own| own.id == agent.id);
        let choice = if custom {
            silt_workspace::AgentChoice::Custom { name: agent.name.clone() }
        } else {
            silt_workspace::AgentChoice::Known { key: silt_workspace::agent_key(&agent.id), name: agent.name.clone() }
        };
        rules.check_agent(&choice, can_sandbox(silt_agents::SandboxSupport::None, &policy)).map_err(refused)?;
        rules.check_model(&choice, on_local_model).map_err(refused)?;
        let preset = match (preset, policy.max_preset) {
            (Some(preset), Some(cap)) if !chosen && !preset.within(cap) => Some(cap),
            (preset, _) => preset,
        };
        if let Some(preset) = preset {
            rules.check_preset(preset).map_err(refused)?;
        }
        if let Some(mode) = mode {
            rules.check_mode(mode, crate::acp::mode_meaning(&agent.id, mode, false)).map_err(refused)?;
        }
        Ok(preset)
    }

    /// The preset Silt answers a thread's agent with: its own, held to its
    /// workspace's cap.
    pub(crate) fn capped_preset(&self, thread: silt_types::ThreadId, preset: Preset) -> Preset {
        let Some(family) = self.threads.get(&thread).map(|record| record.family) else {
            return preset;
        };
        match self.effective_policy(&self.family_workspace(family)).max_preset {
            Some(cap) if !preset.within(cap) => cap,
            _ => preset,
        }
    }

    /// The rules of a thread's workspace, for a change to its running agent.
    pub(crate) fn check_thread_change(
        &self,
        thread: silt_types::ThreadId,
        check: impl FnOnce(&Rules<'_>) -> Result<(), silt_workspace::Refusal>,
    ) -> Result<(), CoreError> {
        let Some(family) = self.threads.get(&thread).map(|record| record.family) else {
            return Ok(());
        };
        let workspace = self.family_workspace(family);
        check(&self.rules(&workspace)).map_err(|refusal| CoreError::Invalid(refusal.to_string()))
    }
}

/// The folders other workspaces own, which an isolated workspace's sandbox
/// denies: their folder rules and their agent homes.
fn others_folders(all: &Workspaces, workspace: &WorkspaceId) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for other in all.workspaces.iter().filter(|other| other.id != *workspace) {
        for rule in &other.rules {
            if let MatchRule::Folder(folder) = rule {
                folders.push(PathBuf::from(folder));
            }
        }
        folders.extend(other.env.values().map(PathBuf::from).filter(|value| value.is_absolute()));
    }
    folders
}

/// The agent's own settings a workspace's rules ask for: no skipping asking
/// under a cap below Full access, its sandbox, and in an isolated workspace
/// Silt's data and other workspaces' folders denied.
fn locks_for(data_dir: &Path, all: &Workspaces, workspace: &Workspace, policy: &Policy) -> silt_agents::Locks {
    let mut deny = Vec::new();
    if policy.isolated {
        deny.push(data_dir.to_owned());
        deny.extend(others_folders(all, &workspace.id));
    }
    silt_agents::Locks { no_bypass: !policy.allows_full_access(), sandbox: policy.sandbox_required(), deny }
}

/// Whether `agent` can run under `policy`'s sandbox rules.
pub(crate) fn can_sandbox(support: silt_agents::SandboxSupport, policy: &Policy) -> bool {
    use silt_agents::SandboxSupport;
    match support {
        SandboxSupport::Isolate => true,
        SandboxSupport::Sandbox => !policy.isolated,
        SandboxSupport::None => !policy.sandbox_required(),
    }
}

/// What an agent's history tools may read (ADR 0065): its workspace, and
/// in a Local only one its own project only (ADR 0070).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scope {
    pub(crate) workspace: WorkspaceId,
    pub(crate) project: Option<String>,
}

impl Scope {
    fn allows_project(&self, project: Option<&str>) -> bool {
        self.project.as_deref().is_none_or(|own| project == Some(own))
    }
}

/// What an agent run from a terminal is held to (ADR 0065).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunRules {
    pub workspace: Workspace,
    pub locks: silt_agents::Locks,
    pub env: Vec<(String, String)>,
    /// For its history tools, which read its workspace only.
    pub token: String,
    /// The arguments to run it with, when they differ from those given: a
    /// local model's flag goes in its environment instead (ADR 0070).
    #[serde(default)]
    pub args: Option<Vec<String>>,
}

async fn mint_run_token(
    store: &silt_store::Store,
    thread: silt_types::ThreadId,
    workspace: &WorkspaceId,
    project: Option<&str>,
) -> Result<String, CoreError> {
    let token = format!("{:032x}{:032x}", uuid::Uuid::new_v4().as_u128(), uuid::Uuid::new_v4().as_u128());
    let hash = blake3::hash(token.as_bytes());
    store.add_run_token(hash.as_bytes(), thread, workspace.as_str(), project, crate::core::now_ms()).await?;
    Ok(token)
}

/// The rules for running `agent` with `args` in `thread`, from the files
/// and the store, so a terminal's agent is checked whether a daemon runs or
/// not (ADR 0018). A refusal says which rule and whose.
pub async fn run_rules(
    data_dir: &Path,
    thread: Option<&str>,
    cwd: Option<&Path>,
    agent: &dyn silt_agents::Agent,
    args: &[String],
) -> Result<RunRules, CoreError> {
    let store = silt_store::Store::open(data_dir).await?;
    let all = load(data_dir).unwrap_or_else(|| {
        Workspaces::single(
            crate::remote::load_sign_ins(data_dir)
                .into_iter()
                .next()
                .map(|sign_in| ServerBinding { url: sign_in.url, org: None }),
        )
    });
    let record = match thread {
        Some(thread) => store.threads().await?.into_iter().find(|record| record.id.to_string() == thread),
        None => None,
    };
    let here = cwd.and_then(|cwd| silt_git::Repo::discover(cwd).ok());
    let pinned: Vec<(FamilyId, String)> = store.family_workspaces().await?;
    let chosen: Vec<(String, String)> = store.project_workspaces().await?;
    let workspace = record
        .as_ref()
        .and_then(|record| pinned.iter().find(|(family, _)| *family == record.family))
        .and_then(|(_, id)| WorkspaceId::new(id.clone()).ok())
        .and_then(|id| all.get(&id).cloned())
        .unwrap_or_else(|| match &record {
            Some(record) => {
                let chosen =
                    chosen.iter().find(|(repo, _)| *repo == record.repo).and_then(|(_, id)| WorkspaceId::new(id.clone()).ok());
                let root = slashed(&record.repo_root);
                all.resolve(&record.repo, Some(&root), chosen.as_ref()).workspace.clone()
            }
            None => match &here {
                Some(repo) => {
                    let project = repo.project().unwrap_or_default();
                    let chat = repo.is_chat().then(|| repo.chat_workspace()).flatten().and_then(|id| WorkspaceId::new(id).ok());
                    let chosen = chat.or_else(|| {
                        chosen.iter().find(|(repo, _)| *repo == project).and_then(|(_, id)| WorkspaceId::new(id.clone()).ok())
                    });
                    all.resolve(&project, Some(&slashed(repo.root())), chosen.as_ref()).workspace.clone()
                }
                None => all.default_workspace().clone(),
            },
        });
    let org = match &workspace.server {
        Some(ServerBinding { url, org: Some(org) }) => {
            let fetched = store.org_policies().await?.into_iter().find(|(server, name, _)| server == url && name == org);
            Some(match fetched.and_then(|(_, _, policy)| serde_json::from_str::<Policy>(&policy).ok()) {
                Some(policy) => Ok(OrgPolicy { org: org.clone(), server: url.clone(), policy }),
                None => Err(Source::Org { org: org.clone(), server: url.clone() }),
            })
        }
        _ => None,
    };
    let rules = Rules {
        workspace: &workspace.name,
        local: &workspace.policy,
        org: org.as_ref().map(|org| org.as_ref().map_err(Clone::clone)),
    };
    if silt_workspace::agent_key(agent.id()).as_str() == "claude" && silt_llm::local_model_in_args(args).is_some() {
        return Err(CoreError::Invalid("a local model runs through Silt's daemon; start it with `silt daemon start`".into()));
    }
    let (locks, env) = check_run(data_dir, &all, &workspace, &rules, agent, args)?;
    let thread = record.as_ref().map_or(silt_types::ThreadId(0), |record| record.id);
    let project =
        record.as_ref().map(|record| record.repo.clone()).or_else(|| here.as_ref().and_then(|repo| repo.project().ok()));
    let token = mint_run_token(&store, thread, &workspace.id, project.as_deref()).await?;
    Ok(RunRules { workspace, locks, env, token, args: None })
}

/// Whether `agent` may run with `args` under `rules`, and if so the locks
/// and variables it runs with.
fn check_run(
    data_dir: &Path,
    all: &Workspaces,
    workspace: &Workspace,
    rules: &Rules<'_>,
    agent: &dyn silt_agents::Agent,
    args: &[String],
) -> Result<(silt_agents::Locks, Vec<(String, String)>), CoreError> {
    let refused = |refusal: silt_workspace::Refusal| CoreError::Invalid(refusal.to_string());
    let policy = rules.effective();
    let choice =
        silt_workspace::AgentChoice::Known { key: silt_workspace::agent_key(agent.id()), name: agent.display_name().to_owned() };
    rules.check_agent(&choice, can_sandbox(agent.sandbox_support(), &policy)).map_err(refused)?;
    rules.check_model(&choice, silt_llm::local_model_in_args(args).is_some()).map_err(refused)?;
    if let Some(flag) = agent.bypass_flag(args) {
        rules.check_flag(&flag, agent.display_name()).map_err(refused)?;
    }
    let locks = locks_for(data_dir, all, workspace, &policy);
    let env = workspace.env.iter().map(|(name, value)| (name.clone(), value.clone())).collect();
    Ok((locks, env))
}

impl Core {
    /// [`run_rules`] as the daemon answers it, from what it holds, so a
    /// terminal's agent doesn't open the store beside it.
    pub(crate) async fn run_rules_here(
        &self,
        thread: Option<&str>,
        cwd: Option<&Path>,
        agent: &str,
        args: &[String],
    ) -> Result<RunRules, CoreError> {
        let agent = silt_agents::AGENTS
            .iter()
            .copied()
            .find(|candidate| candidate.id() == agent)
            .ok_or_else(|| CoreError::Invalid(format!("no agent {agent}")))?;
        let all = self.workspaces();
        let (thread, workspace, project) = match (thread, cwd) {
            (Some(selector), _) => {
                let thread = self.find_thread(selector)?;
                let record = self.threads.get(&thread).ok_or_else(|| CoreError::Invalid("no such thread".into()))?;
                (thread, self.family_workspace(record.family), record.repo.clone())
            }
            (None, Some(cwd)) => {
                let repo = silt_git::Repo::discover(cwd)?;
                let project = repo.project()?;
                let id = self.new_family_workspace(&repo, &project);
                (silt_types::ThreadId(0), all.get(&id).cloned().unwrap_or_else(|| all.default_workspace().clone()), project)
            }
            (None, None) => return Err(CoreError::Invalid("name the thread or the folder the agent runs in".into())),
        };
        let (locks, mut env) = check_run(&self.data_dir, &all, &workspace, &self.rules(&workspace), agent, args)?;
        let local =
            match silt_llm::local_model_in_args(args).filter(|_| silt_workspace::agent_key(agent.id()).as_str() == "claude") {
                Some((model, rest)) => Some((self.models.installed(&model).await?, rest)),
                None => None,
            };
        let token = self.mint_run_token(thread, &workspace.id, Some(&project)).await?;
        let args = match local {
            Some((model, rest)) => {
                env.extend(crate::models::claude_env(&self.models.gateway().await?, &token, &model));
                Some(rest)
            }
            None => None,
        };
        Ok(RunRules { workspace, locks, env, token, args })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_expand_the_home() {
        let home = silt_os::home_dir().expect("a home");
        let home = home.to_string_lossy().replace('\\', "/");
        assert_eq!(folder_rule("~/work/").unwrap(), MatchRule::Folder(format!("{}/work", home.trim_end_matches('/'))));
        assert_eq!(folder_rule("/srv/code").unwrap(), MatchRule::Folder("/srv/code".into()));
        assert!(folder_rule("work").is_err());
        assert!(folder_rule("~other/work").is_err());
    }

    #[test]
    fn a_broken_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), None);
        std::fs::write(dir.path().join(FILE), "{not json").unwrap();
        assert_eq!(load(dir.path()), None);
        let all = Workspaces::single(None);
        save(dir.path(), &all).unwrap();
        assert_eq!(load(dir.path()), Some(all));
    }
}
