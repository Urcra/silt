//! Agents the web UI starts over ACP (ADR 0014): one per thread, run in the
//! thread's checkout on this machine and recorded as a session of the `acp`
//! adapter, so history, search, forks and handoffs treat it like any other.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_acp::registry::{self, AgentInfo};
use silt_acp::{Accepts, AgentHandle, Attachment, Command, Permission, Start, Update};
use silt_agents::acp;
use silt_log::{Author, ConvOp, Op};
use silt_proto::AgentState;
use silt_store::CheckoutId;
use silt_types::ThreadId;
use tokio::sync::mpsc;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::protocol::{
    AgentChoice, AgentChoices, AgentStatus, CloneInfo, PermissionAsked, Preset, PromptAttachment, StartOptions, TextKind,
};
use crate::sessions::NewSession;

pub(crate) struct AcpRun {
    run: u64,
    info: AgentInfo,
    checkout_id: CheckoutId,
    checkout: PathBuf,
    handle: Option<AgentHandle>,
    /// Commands given before the agent was up.
    queued: Vec<Command>,
    session: Option<String>,
    /// An earlier session of the same agent to load, by its own id.
    load: Option<(String, String)>,
    /// The conversation so far, for the first prompt of a session that
    /// could not be loaded.
    primer: Option<String>,
    state: AgentState,
    text: String,
    thought: String,
    asking: Vec<Permission>,
    error: Option<String>,
    /// Each running tool call's output so far, to tell browsers what's new.
    tool_output: HashMap<String, String>,
    /// What to tell the agent once the turn being cancelled for it ends.
    steering: Option<String>,
    /// Each tool call's kind as the agent announced it, for permission
    /// requests that don't repeat it.
    tool_kinds: HashMap<String, String>,
    modes: Option<silt_acp::Modes>,
    settings: Vec<silt_acp::Setting>,
    /// The turn running is a Plan first one.
    planning: bool,
    /// The mode Plan first switched away from, to go back to once the plan
    /// is approved.
    back_to: Option<String>,
    /// The prompt to send once the agent is ready, and whether to plan
    /// first.
    first: Option<(String, bool)>,
    accepts: Accepts,
    /// The merge request whose change guide this turn writes, and what the
    /// agent has said in it so far (ADR 0025).
    guiding: Option<(silt_types::DeltaId, String)>,
}

/// What the fork's agent is asked for a change guide.
const GUIDE_PROMPT: &str = "Write a change guide for the reviewers of the merge request offered from this thread: \
                            walk through what changed and why, in the order that's easiest to read, naming files and \
                            the reasons behind each choice. Don't change any files; answer with the guide alone.";

/// What a Plan first prompt starts with when the agent has no plan mode.
const PLAN_FIRST: &str = "Before you change anything, write a plan for this and stop: don't edit files or run commands \
                          that change them until I approve the plan.";

impl AcpRun {
    fn plan_mode(&self) -> Option<&str> {
        self.modes.as_ref().and_then(silt_acp::plan_mode).map(|mode| mode.id.as_str())
    }

    /// Whether the agent is still where Plan first put it: in its plan
    /// mode, or told by the prompt to plan when it has none.
    fn still_planning(&self) -> bool {
        match self.plan_mode() {
            Some(plan) => self.modes.as_ref().is_some_and(|modes| modes.current == plan),
            None => true,
        }
    }
}

/// What a running agent reports, tagged with its run so a stopped one's
/// late events are ignored.
pub enum AcpEvent {
    Started(Result<AgentHandle, String>),
    Agent(silt_acp::Event),
}

/// The state shown for a run: a working agent with a request open waits on
/// the person (ADR 0022).
fn shown_state(state: AgentState, asking: bool) -> AgentState {
    match state {
        AgentState::Working if asking => AgentState::AwaitingPermission,
        other => other,
    }
}

fn silt_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("silt"))
}

impl Core {
    /// The agents the web UI can start. The registry's copy is brought up to
    /// date in the background the first time someone looks, not whenever a
    /// daemon starts; the bundled one answers meanwhile.
    pub(crate) fn acp_agents(&mut self) -> AgentChoices {
        if !self.registry_checked {
            self.registry_checked = true;
            tokio::spawn(registry::refresh(self.data_dir.clone()));
        }
        let mut clones: Vec<CloneInfo> =
            self.clones.iter().map(|(repo, path)| CloneInfo { repo: repo.clone(), path: path.clone() }).collect();
        clones.sort_by(|a, b| a.repo.cmp(&b.repo));
        AgentChoices {
            agents: registry::all(&self.data_dir)
                .into_iter()
                .map(|agent| AgentChoice { id: agent.id, name: agent.name, description: agent.description })
                .collect(),
            clones,
        }
    }

    /// Starts `agent` in the thread's checkout, making one from a known
    /// clone if this machine has none, or in a new thread of `clone`.
    pub(crate) async fn acp_start(
        &mut self,
        thread: Option<&str>,
        clone: Option<&Path>,
        agent: &str,
        options: StartOptions,
    ) -> Result<String, CoreError> {
        let info = registry::all(&self.data_dir)
            .into_iter()
            .find(|candidate| candidate.id == agent)
            .ok_or_else(|| CoreError::Invalid(format!("no agent called {agent}")))?;
        if options.prompt.as_ref().is_some_and(|prompt| prompt.trim().is_empty()) {
            return Err(CoreError::Invalid("the first prompt is empty".into()));
        }
        let (thread, checkout_id, checkout) = match (thread, clone) {
            (Some(selector), _) => {
                if options.base.is_some() || options.here || options.visibility.is_some() {
                    return Err(CoreError::Invalid(
                        "a thread keeps its checkout and visibility; those are for a new thread".into(),
                    ));
                }
                let thread = self.find_thread(selector)?;
                if self.acp.get(&thread).is_some_and(|run| run.state.running()) {
                    return Err(CoreError::Invalid("an agent is already running in this thread".into()));
                }
                let (checkout_id, checkout) = self.checkout_for_agent(thread).await?;
                (thread, checkout_id, checkout)
            }
            (None, Some(clone)) => {
                let created = self.create_thread(clone, !options.here, false, options.base.as_deref()).await?;
                if let Some(commit) = created.head {
                    self.record_bundle(created.family, created.thread, &created.repo, commit, &[]).await?;
                }
                if let Some(visibility) = options.visibility.clone() {
                    self.register_thread_now(created.thread).await?;
                    self.set_visibility(clone, Some(&created.thread.to_string()), visibility).await?;
                }
                (created.thread, created.checkout_id, created.checkout)
            }
            (None, None) => return Err(CoreError::Invalid("pick a thread or a clone to start the agent in".into())),
        };
        if let Some(preset) = options.preset {
            self.keep_choice(thread, |record| record.preset = preset.as_str().to_owned()).await?;
        }
        if let Some(mode) = options.mode.clone() {
            self.keep_choice(thread, |record| record.mode = Some(mode)).await?;
        }
        self.next_token += 1;
        let run = self.next_token;
        let load = self.earlier_session(thread, &info.id);
        let start = Start {
            program: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: checkout.clone(),
            mcp: Some((
                silt_exe(),
                vec![
                    "mcp".into(),
                    "--thread".into(),
                    thread.to_string(),
                    "--checkout".into(),
                    checkout.to_string_lossy().into_owned(),
                ],
            )),
            load: load.as_ref().map(|(_, native)| native.clone()),
        };
        spawn_agent(self.events.clone(), thread, run, info.clone(), self.data_dir.clone(), start);
        self.acp.insert(
            thread,
            AcpRun {
                run,
                info,
                checkout_id,
                checkout,
                handle: None,
                queued: Vec::new(),
                session: None,
                load,
                primer: None,
                state: AgentState::Starting,
                text: String::new(),
                thought: String::new(),
                asking: Vec::new(),
                error: None,
                tool_output: HashMap::new(),
                steering: None,
                tool_kinds: HashMap::new(),
                modes: None,
                settings: Vec::new(),
                planning: false,
                back_to: None,
                first: options.prompt.map(|prompt| (prompt, options.plan)),
                accepts: Accepts::default(),
                guiding: None,
            },
        );
        self.agent_changed(thread).await;
        Ok(thread.to_string())
    }

    /// The branches a new thread of a clone this machine knows can start from.
    pub(crate) fn branches(&self, clone: &Path) -> Result<Vec<String>, CoreError> {
        if !self.clones.values().any(|known| known == clone) {
            return Err(CoreError::Invalid(format!("this machine knows no clone at {}", clone.display())));
        }
        Ok(silt_git::Repo::discover(clone)?.branches()?)
    }

    async fn checkout_for_agent(&mut self, thread: ThreadId) -> Result<(CheckoutId, PathBuf), CoreError> {
        if let Some(checkout) = self.checkouts.values().find(|checkout| checkout.record.thread == thread) {
            return Ok((checkout.record.id, checkout.record.path.clone()));
        }
        let record = &self.threads[&thread];
        let clone = self
            .clones
            .get(&record.repo)
            .cloned()
            .or_else(|| (!record.repo_root.as_os_str().is_empty()).then(|| record.repo_root.clone()))
            .ok_or_else(|| {
                CoreError::Invalid(format!("this machine knows no clone of {}; run `silt open` in one first", record.repo))
            })?;
        self.check_out_managed(thread, &clone).await
    }

    /// The last session `agent` recorded in the thread: Silt's id for it and
    /// the agent's own.
    fn earlier_session(&self, thread: ThreadId, agent: &str) -> Option<(String, String)> {
        let family = self.threads.get(&thread)?.family;
        self.local_log(family).iter().rev().find_map(|delta| match &delta.op {
            Op::Conv(ConvOp::Native { session, line, .. }) if delta.thread == thread && session.agent == acp::AGENT => {
                acp::session_of(line).filter(|(id, _)| id == agent).map(|(_, native)| (session.id.clone(), native))
            }
            _ => None,
        })
    }

    /// Sends `text` to the thread's agent when it's ready for a prompt, and
    /// queues it otherwise (ADR 0022). An agent awaiting input takes it as
    /// the answer to its plan.
    pub(crate) async fn acp_prompt(&mut self, selector: &str, text: &str, plan: bool) -> Result<&'static str, CoreError> {
        self.acp_prompt_with(selector, text, plan, Vec::new()).await
    }

    /// [`Self::acp_prompt`] with files attached, which only go with a prompt
    /// the agent takes now.
    pub(crate) async fn acp_prompt_with(
        &mut self,
        selector: &str,
        text: &str,
        plan: bool,
        attachments: Vec<Attachment>,
    ) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        if text.trim().is_empty() {
            return Err(CoreError::Invalid("there is nothing to send".into()));
        }
        let ready = self.acp.get(&thread).is_some_and(|run| matches!(run.state, AgentState::Idle | AgentState::AwaitingInput));
        if !ready {
            if plan {
                return Err(CoreError::Invalid("Plan first starts a turn of its own; send it once this one ends".into()));
            }
            if !attachments.is_empty() {
                return Err(CoreError::Invalid(
                    "attached files go with a prompt the agent takes now; send it once this turn ends".into(),
                ));
            }
            self.enqueue(thread, text, false).await?;
            return Ok("queued");
        }
        let run = self.acp.get_mut(&thread).expect("checked above");
        if let Some(refused) = attachments.iter().find(|attachment| !run.accepts.takes(attachment)) {
            return Err(CoreError::Invalid(format!("{} doesn't take files like {}", run.info.name, refused.name)));
        }
        if plan {
            plan_first(run);
        } else if let Some(mode) = run.back_to.take() {
            send(run, Command::SetMode(mode));
        }
        let me = self.own_user();
        self.send_prompt(thread, &me, text, attachments).await?;
        Ok("sent")
    }

    /// Interrupts the agent's turn to tell it `text`: ACP can't add to a
    /// running turn, so the turn is cancelled and `text` goes first once it
    /// ends. An agent that isn't working hears it as its next prompt.
    pub(crate) async fn acp_steer(&mut self, thread: ThreadId, text: &str) -> Result<&'static str, CoreError> {
        let run = self.running(thread)?;
        if !matches!(run.state, AgentState::Working) {
            return self.acp_prompt(&thread.to_string(), text, false).await;
        }
        let run = self.acp.get_mut(&thread).expect("checked above");
        run.steering = Some(text.to_owned());
        send(run, Command::Cancel);
        self.agent_changed(thread).await;
        Ok("steering")
    }

    /// Sends the next prompt once a turn ended: a steer first, then the
    /// queue's front.
    async fn next_prompt(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let Some(run) = self.acp.get_mut(&thread).filter(|run| run.state == AgentState::Idle) else {
            return Ok(());
        };
        if let Some(steer) = run.steering.take() {
            let me = self.own_user();
            let said = format!("I interrupted your last turn to say: {steer}");
            return self.send_prompt(thread, &me, &said, Vec::new()).await;
        }
        let Some(next) = self.store.take_next(thread).await? else {
            return Ok(());
        };
        self.announce(crate::protocol::BusEvent::Queue { thread: thread.to_string() });
        self.send_prompt(thread, &next.author, &next.text, Vec::new()).await
    }

    /// Records and sends a prompt by `author`, with teammates' drafts, the
    /// thread's notices and, first in a new session, its conversation.
    async fn send_prompt(
        &mut self,
        thread: ThreadId,
        author: &str,
        text: &str,
        attachments: Vec<Attachment>,
    ) -> Result<(), CoreError> {
        let mut authors = vec![author.to_owned()];
        let mut said = text.trim().to_owned();
        for (user, draft) in self.teammates_draft_list(thread).await {
            said.push_str(&format!("\n\n{user}: {draft}"));
            authors.push(user);
        }
        if said.trim().is_empty() {
            return Err(CoreError::Invalid("there is nothing to send".into()));
        }
        let mut prompt = said.clone();
        if let Some(notices) = self.take_notices(thread).await {
            prompt = format!("{prompt}\n\n{notices}");
        }
        if self.acp.get(&thread).is_some_and(|run| run.planning && run.plan_mode().is_none()) {
            prompt = format!("{PLAN_FIRST}\n\n{prompt}");
        }
        if let Some(primer) = self.acp.get_mut(&thread).and_then(|run| run.primer.take()) {
            prompt = format!("{primer}\n\n{prompt}");
        }
        let attached = self.record_attachments(thread, &attachments).await?;
        self.record_acp(thread, &acp::prompt(&said, &authors.join(", "), &attached)).await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.state = AgentState::Working;
        run.error = None;
        send(run, Command::Prompt { text: prompt, attachments });
        self.agent_changed(thread).await;
        Ok(())
    }

    pub(crate) async fn acp_command(&mut self, selector: &str, command: Command) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        self.running(thread)?;
        let run = self.acp.get_mut(&thread).expect("checked above");
        if let Command::Answer { id, option } = &command {
            let asked = run
                .asking
                .iter()
                .find(|asked| asked.id == *id)
                .ok_or_else(|| CoreError::Invalid("the agent isn't waiting for that answer".into()))?;
            if option.as_ref().is_some_and(|option| !asked.options.iter().any(|(offered, _)| offered == option)) {
                return Err(CoreError::Invalid("the agent didn't offer that option".into()));
            }
            run.asking.retain(|asked| asked.id != *id);
        }
        send(run, command);
        self.agent_changed(thread).await;
        Ok("ok")
    }

    pub(crate) fn running(&self, thread: ThreadId) -> Result<&AcpRun, CoreError> {
        self.acp
            .get(&thread)
            .filter(|run| run.state.running())
            .ok_or_else(|| CoreError::Invalid("no agent is running in this thread".into()))
    }

    /// How the thread's agent stands: as it runs here, or as last stored.
    pub(crate) fn agent_state(&self, thread: ThreadId) -> Option<AgentState> {
        match self.acp.get(&thread) {
            Some(run) => Some(shown_state(run.state, !run.asking.is_empty())),
            None => self.agent_runs.get(&thread).and_then(|stored| AgentState::parse(&stored.state)),
        }
    }

    /// Tells browsers how the thread's agent stands, and stores it when its
    /// state or error changed.
    pub(crate) async fn agent_changed(&mut self, thread: ThreadId) {
        self.announce_agent(thread);
        let Some(run) = self.acp.get(&thread) else {
            return;
        };
        let stored = self.agent_runs.get(&thread);
        let record = silt_store::AgentRunRecord {
            agent: run.info.name.clone(),
            state: shown_state(run.state, !run.asking.is_empty()).as_str().to_owned(),
            error: run.error.clone(),
            preset: stored.map_or_else(|| "ask_before_commands".to_owned(), |stored| stored.preset.clone()),
            mode: stored.and_then(|stored| stored.mode.clone()),
            updated_ms: crate::core::now_ms(),
        };
        let unchanged = stored
            .is_some_and(|stored| (&stored.agent, &stored.state, &stored.error) == (&record.agent, &record.state, &record.error));
        if unchanged {
            return;
        }
        if let Err(error) = self.store.put_agent_run(thread, &record).await {
            warn!(%error, "could not store how the agent stands");
        }
        self.agent_runs.insert(thread, record);
        self.announce(crate::protocol::BusEvent::Threads);
    }

    /// Answers a permission request by the thread's preset when it can, and
    /// leaves it for the person otherwise (ADR 0022).
    fn permission_asked(&mut self, thread: ThreadId, permission: Permission) {
        let preset = self.preset(thread);
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let kind = permission.tool_kind.clone().or_else(|| run.tool_kinds.get(&permission.tool_call_id).cloned());
        match crate::presets::decide(preset, kind.as_deref(), &permission.option_kinds) {
            Some(option) => send(run, Command::Answer { id: permission.id, option: Some(option.to_owned()) }),
            None => run.asking.push(permission),
        }
    }

    pub(crate) fn preset(&self, thread: ThreadId) -> Preset {
        self.agent_runs.get(&thread).and_then(|run| Preset::parse(&run.preset)).unwrap_or_default()
    }

    pub(crate) async fn set_preset(&mut self, thread: ThreadId, preset: Preset) -> Result<&'static str, CoreError> {
        self.keep_choice(thread, |record| record.preset = preset.as_str().to_owned()).await?;
        self.announce_agent(thread);
        Ok("set")
    }

    /// Switches the running agent to `mode` and keeps it for the thread's
    /// later runs.
    pub(crate) async fn set_mode(&mut self, thread: ThreadId, mode: &str) -> Result<&'static str, CoreError> {
        let run = self.running(thread)?;
        if !run.modes.as_ref().is_some_and(|modes| modes.available.iter().any(|offered| offered.id == mode)) {
            return Err(CoreError::Invalid(format!("the agent has no mode called {mode}")));
        }
        let run = self.acp.get_mut(&thread).expect("checked above");
        run.back_to = None;
        send(run, Command::SetMode(mode.to_owned()));
        self.keep_choice(thread, |record| record.mode = Some(mode.to_owned())).await?;
        Ok("set")
    }

    pub(crate) async fn set_setting(&mut self, thread: ThreadId, id: String, value: String) -> Result<&'static str, CoreError> {
        let run = self.running(thread)?;
        let offered = run
            .settings
            .iter()
            .find(|setting| setting.id == id)
            .is_some_and(|setting| setting.choices.iter().any(|choice| choice.id == value));
        if !offered {
            return Err(CoreError::Invalid(format!("the agent doesn't offer {value} for {id}")));
        }
        send(self.acp.get_mut(&thread).expect("checked above"), Command::SetSetting { id, value });
        Ok("set")
    }

    /// Stores a choice the person made for the thread's agent.
    async fn keep_choice(
        &mut self,
        thread: ThreadId,
        choose: impl FnOnce(&mut silt_store::AgentRunRecord),
    ) -> Result<(), CoreError> {
        let mut record = self.agent_runs.get(&thread).cloned().unwrap_or_else(|| silt_store::AgentRunRecord {
            agent: String::new(),
            state: AgentState::Stopped.as_str().to_owned(),
            error: None,
            preset: Preset::default().as_str().to_owned(),
            mode: None,
            updated_ms: 0,
        });
        choose(&mut record);
        record.updated_ms = crate::core::now_ms();
        self.store.put_agent_run(thread, &record).await?;
        self.agent_runs.insert(thread, record);
        Ok(())
    }

    /// Reads how each thread's agent stood. One that ran went with the
    /// daemon's last process, so it stopped.
    pub(crate) async fn load_agent_runs(&mut self) -> Result<(), CoreError> {
        self.agent_runs = self.store.agent_runs().await?;
        for (thread, run) in &mut self.agent_runs {
            if AgentState::parse(&run.state).is_some_and(AgentState::running) {
                run.state = AgentState::Stopped.as_str().to_owned();
                self.store.put_agent_run(*thread, run).await?;
            }
        }
        Ok(())
    }

    pub(crate) fn agent_status(&self, thread: ThreadId) -> Option<AgentStatus> {
        let run = self.acp.get(&thread)?;
        Some(AgentStatus {
            agent: run.info.name.clone(),
            state: shown_state(run.state, !run.asking.is_empty()),
            streaming: run.text.clone(),
            thinking: run.thought.clone(),
            permissions: run
                .asking
                .iter()
                .map(|asked| PermissionAsked { id: asked.id, title: asked.title.clone(), options: asked.options.clone() })
                .collect(),
            error: run.error.clone(),
            modes: run.modes.as_ref().map(Into::into),
            settings: run.settings.iter().map(Into::into).collect(),
            takes_images: run.accepts.image,
            takes_files: run.accepts.embedded,
        })
    }

    pub(crate) async fn acp_event(&mut self, thread: ThreadId, run: u64, event: AcpEvent) {
        if self.acp.get(&thread).is_none_or(|current| current.run != run) {
            // Its run was replaced or its thread purged while it started.
            if let AcpEvent::Started(Ok(handle)) = event {
                handle.send(Command::Stop);
            }
            return;
        }
        if let Err(error) = self.handle_acp(thread, event).await {
            warn!(%error, "could not record the agent's work");
            if let Some(current) = self.acp.get_mut(&thread) {
                current.error = Some(error.to_string());
            }
        }
        self.agent_changed(thread).await;
    }

    async fn handle_acp(&mut self, thread: ThreadId, event: AcpEvent) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        match event {
            AcpEvent::Started(Ok(handle)) => {
                for command in run.queued.drain(..) {
                    handle.send(command);
                }
                run.handle = Some(handle);
            }
            AcpEvent::Started(Err(error)) => {
                run.error = Some(error);
                run.state = AgentState::Error;
            }
            AcpEvent::Agent(silt_acp::Event::Ready { session, loaded, agent, modes, settings, accepts }) => {
                let kept = self.agent_runs.get(&thread).and_then(|record| record.mode.clone());
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.modes = modes;
                run.settings = settings;
                run.accepts = accepts;
                if let Some(mode) = kept.filter(|mode| {
                    run.modes
                        .as_ref()
                        .is_some_and(|modes| modes.current != *mode && modes.available.iter().any(|offered| offered.id == *mode))
                }) {
                    send(run, Command::SetMode(mode.clone()));
                    // Taken as set, so a Plan first prompt right after knows
                    // which mode to come back to.
                    if let Some(modes) = &mut run.modes {
                        modes.current = mode;
                    }
                }
                let first = run.first.take();
                self.acp_ready(thread, &session, loaded, &agent).await?;
                if let Some((prompt, plan)) = first {
                    self.acp_prompt(&thread.to_string(), &prompt, plan).await?;
                }
                self.next_prompt(thread).await?;
            }
            AcpEvent::Agent(silt_acp::Event::ModeChanged(mode)) => {
                if let Some(modes) = &mut run.modes {
                    modes.current = mode;
                }
            }
            AcpEvent::Agent(silt_acp::Event::Settings(settings)) => run.settings = settings,
            AcpEvent::Agent(silt_acp::Event::Refused(why)) => run.error = Some(why),
            AcpEvent::Agent(silt_acp::Event::Update(update)) => self.acp_update(thread, update).await?,
            AcpEvent::Agent(silt_acp::Event::Permission(permission)) => self.permission_asked(thread, permission),
            AcpEvent::Agent(silt_acp::Event::TurnEnded { reason }) => {
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::stop(reason.as_str())).await?;
                self.guide_written(thread, &reason).await?;
                let run = self.acp.get_mut(&thread).expect("caller checked");
                let planned = std::mem::take(&mut run.planning) && reason == silt_acp::StopReason::EndTurn;
                run.state = if planned && run.still_planning() {
                    AgentState::AwaitingInput
                } else {
                    // An agent that left its plan mode by itself chose
                    // where to go on.
                    if !run.still_planning() {
                        run.back_to = None;
                    }
                    AgentState::Idle
                };
                run.asking.clear();
                run.tool_kinds.clear();
                self.next_prompt(thread).await?;
            }
            AcpEvent::Agent(silt_acp::Event::Failed(error)) => run.error = Some(crate::protocol::error_chain(&*error)),
            AcpEvent::Agent(silt_acp::Event::Exited) => {
                self.flush_acp(thread).await?;
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.state = AgentState::Stopped;
                run.handle = None;
                run.asking.clear();
                if let Some(id) = run.session.clone() {
                    self.close_open_tool(&id).await?;
                    if let Some(session) = self.session_mut(&id) {
                        session.live = false;
                        session.in_turn = false;
                    }
                }
            }
        }
        Ok(())
    }

    /// The agent's session is up: a loaded one continues its recording, a
    /// new one starts its own, primed with the thread's conversation.
    async fn acp_ready(&mut self, thread: ThreadId, native: &str, loaded: bool, name: &str) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.state = AgentState::Idle;
        let (checkout_id, checkout, agent_id) = (run.checkout_id, run.checkout.clone(), run.info.id.clone());
        let earlier = run.load.take().filter(|(_, earlier)| loaded && earlier == native);
        if let Some(session) = earlier.and_then(|(id, _)| self.session_mut(&id)) {
            session.live = true;
            session.record.checkout_id = checkout_id;
            let id = session.record.session_id.clone();
            self.acp.get_mut(&thread).expect("caller checked").session = Some(id);
            return Ok(());
        }
        let primer = self.primer(thread);
        let id = uuid::Uuid::now_v7().simple().to_string();
        self.register_session(NewSession {
            agent: &acp::Acp,
            id: id.clone(),
            thread,
            family: self.threads[&thread].family,
            checkout_id,
            agent_cwd: checkout.clone(),
            checkout_root: checkout,
            transcript: None,
            restored: Vec::new(),
            restored_side: Vec::new(),
        })
        .await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.session = Some(id);
        run.primer = primer;
        self.record_acp(thread, &acp::session(&agent_id, name, native)).await
    }

    async fn acp_update(&mut self, thread: ThreadId, update: Update) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        match update {
            Update::AgentText(text) => {
                if !run.thought.is_empty() {
                    self.flush_acp(thread).await?;
                }
                self.announce_text(thread, TextKind::Message, &text);
                self.acp.get_mut(&thread).expect("caller checked").text.push_str(&text);
            }
            Update::Thought(text) => {
                if !run.text.is_empty() {
                    self.flush_acp(thread).await?;
                }
                self.announce_text(thread, TextKind::Thought, &text);
                self.acp.get_mut(&thread).expect("caller checked").thought.push_str(&text);
            }
            Update::UserText(_) => {}
            Update::ToolCall { id, title, kind, input } => {
                run.tool_kinds.insert(id.clone(), kind.clone());
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::tool_call(&id, &title, &kind, input.as_ref())).await?;
            }
            Update::ToolUpdate { id, status, output } => {
                if let Some(output) = &output {
                    let before = run.tool_output.insert(id.clone(), output.clone()).unwrap_or_default();
                    self.announce_tool_output(thread, &id, &before, output);
                }
                if let Some(status @ ("completed" | "failed")) = status.as_deref() {
                    self.acp.get_mut(&thread).expect("caller checked").tool_output.remove(&id);
                    self.flush_acp(thread).await?;
                    self.record_acp(thread, &acp::tool_result(&id, status, output.as_deref())).await?;
                }
            }
            Update::Plan(entries) => {
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::plan(&entries)).await?;
            }
        }
        Ok(())
    }

    /// Records the message or thought streamed so far.
    async fn flush_acp(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let thought = std::mem::take(&mut run.thought);
        let text = std::mem::take(&mut run.text);
        if let Some((_, said)) = &mut run.guiding
            && !text.is_empty()
        {
            if !said.is_empty() {
                said.push_str("\n\n");
            }
            said.push_str(&text);
        }
        if !thought.is_empty() {
            self.record_acp(thread, &acp::thought(&thought)).await?;
        }
        if !text.is_empty() {
            self.record_acp(thread, &acp::message(&text)).await?;
        }
        Ok(())
    }

    /// Keeps `text` as the request's change guide, or asks the fork's agent
    /// running here to write it, and keeps its answer once its turn ends.
    pub(crate) async fn mr_guide(&mut self, selector: &str, text: Option<String>) -> Result<&'static str, CoreError> {
        let (family_id, request) = self.find_mr(selector)?;
        if let Some(text) = text {
            if text.trim().is_empty() {
                return Err(CoreError::Invalid("the guide is empty".into()));
            }
            let by = match self.own_user() {
                user if user.is_empty() => "you".to_owned(),
                user => user,
            };
            self.keep_guide(family_id, &request, text, by).await?;
            return Ok("kept");
        }
        let ready = self.acp.get(&request.fork).is_some_and(|run| run.state == AgentState::Idle);
        if !ready {
            return Err(CoreError::Invalid(
                "start an agent in the fork and let it finish its turn to have it write the guide, or write one yourself".into(),
            ));
        }
        self.acp.get_mut(&request.fork).expect("checked above").guiding = Some((request.id, String::new()));
        let me = self.own_user();
        self.send_prompt(request.fork, &me, GUIDE_PROMPT, Vec::new()).await?;
        Ok("asked")
    }

    /// Keeps what the agent wrote for a guide, once its turn ended well.
    async fn guide_written(&mut self, thread: ThreadId, reason: &silt_acp::StopReason) -> Result<(), CoreError> {
        let Some((mr, said)) = self.acp.get_mut(&thread).and_then(|run| run.guiding.take()) else {
            return Ok(());
        };
        if *reason != silt_acp::StopReason::EndTurn || said.trim().is_empty() {
            return Ok(());
        }
        let (family_id, request) = self.find_mr(&mr.to_string())?;
        let by = self.acp.get(&thread).map_or_else(|| "an agent".to_owned(), |run| run.info.name.clone());
        self.keep_guide(family_id, &request, said, by).await
    }

    async fn record_acp(&mut self, thread: ThreadId, line: &str) -> Result<(), CoreError> {
        let Some(session) = self.acp.get(&thread).and_then(|run| run.session.clone()) else {
            return Ok(());
        };
        self.record_line(&session, "main", line.to_owned(), None).await
    }

    /// Keeps each file beside the session, as text redacted like the rest of
    /// the conversation, and answers with the names it's kept under.
    async fn record_attachments(&mut self, thread: ThreadId, attachments: &[Attachment]) -> Result<Vec<String>, CoreError> {
        let Some(id) = self.acp.get(&thread).and_then(|run| run.session.clone()) else {
            return Ok(Vec::new());
        };
        let Some(reference) = self.session_mut(&id).map(|session| session.reference()) else {
            return Ok(Vec::new());
        };
        let family = self.threads[&thread].family;
        let mut names = Vec::new();
        for attachment in attachments {
            let bytes = match std::str::from_utf8(&attachment.data) {
                Ok(text) => {
                    let (redacted, found) = self.redactor.replace_found(text);
                    self.learn_secrets(found);
                    redacted.into_bytes()
                }
                Err(_) => attachment.data.clone(),
            };
            let blob = self.store.blobs().put(&bytes).await?;
            let name = attachment_name(&blob.to_string(), &attachment.name);
            let op = ConvOp::Attachment { session: reference.clone(), name: name.clone(), blob };
            self.append_conv(family, thread, Author::User, op).await?;
            names.push(name);
        }
        Ok(names)
    }
}

/// The largest file a prompt takes, and all of a prompt's files together.
const ATTACHMENT_BYTES: usize = 10 << 20;
const ATTACHMENTS_BYTES: usize = 25 << 20;

/// The prompt's files from the web UI's base64.
pub(crate) fn decode(attachments: Vec<PromptAttachment>) -> Result<Vec<Attachment>, CoreError> {
    let mut total = 0;
    attachments
        .into_iter()
        .map(|attachment| {
            let data = BASE64
                .decode(attachment.data.as_bytes())
                .map_err(|_| CoreError::Invalid(format!("{} didn't arrive whole", attachment.name)))?;
            total += data.len();
            if data.len() > ATTACHMENT_BYTES || total > ATTACHMENTS_BYTES {
                return Err(CoreError::Invalid(format!(
                    "{} is too large: a prompt takes files of up to 10 MiB, 25 MiB together",
                    attachment.name
                )));
            }
            Ok(Attachment { name: attachment.name, mime: attachment.mime, data })
        })
        .collect()
}

/// Where a prompt's file is kept beside its session: under its content's
/// hash, so names can repeat, and as one plain path segment whatever the
/// browser called it.
fn attachment_name(hash: &str, name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let safe: String = base.chars().map(|c| if c.is_control() { '_' } else { c }).collect();
    let safe = if safe.is_empty() || safe == "." || safe == ".." { "file".to_owned() } else { safe };
    format!("prompt/{}/{safe}", &hash[..hash.len().min(12)])
}

impl AcpRun {
    /// Stops the agent. One still starting stops when it comes up and finds
    /// its run gone.
    pub(crate) fn stop(self) {
        if let Some(handle) = &self.handle {
            handle.send(Command::Stop);
        }
    }
}

/// Makes the next turn a Plan first one: in the agent's plan mode when it
/// has one, and by the prompt otherwise.
fn plan_first(run: &mut AcpRun) {
    run.planning = true;
    let Some(plan) = run.plan_mode().map(str::to_owned) else {
        return;
    };
    let current = run.modes.as_ref().map(|modes| modes.current.clone()).unwrap_or_default();
    if current != plan {
        run.back_to.get_or_insert(current);
        send(run, Command::SetMode(plan));
    }
}

fn send(run: &mut AcpRun, command: Command) {
    match &run.handle {
        Some(handle) => {
            handle.send(command);
        }
        None => run.queued.push(command),
    }
}

/// Finds the agent's command, downloading it if needed, then runs it and
/// forwards what it does to the daemon's event loop.
fn spawn_agent(
    events: mpsc::UnboundedSender<Event>,
    thread: ThreadId,
    run: u64,
    info: AgentInfo,
    data_dir: PathBuf,
    mut start: Start,
) {
    tokio::spawn(async move {
        let registry::AgentCommand { program, args, env } = match registry::command(&info, &data_dir).await {
            Ok(command) => command,
            Err(error) => {
                let _ = events.send(Event::Acp { thread, run, event: AcpEvent::Started(Err(error.to_string())) });
                return;
            }
        };
        start.program = program;
        start.args = args;
        start.env = registry::agent_environment(&info.id, &data_dir).into_iter().chain(env).collect();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let handle = silt_acp::start(start, sender);
        let _ = events.send(Event::Acp { thread, run, event: AcpEvent::Started(Ok(handle)) });
        while let Some(event) = receiver.recv().await {
            let exited = matches!(event, silt_acp::Event::Exited);
            if events.send(Event::Acp { thread, run, event: AcpEvent::Agent(event) }).is_err() || exited {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use silt_store::{AgentRunRecord, Store, ThreadRecord};
    use silt_types::{ActorId, FamilyId};

    use super::*;

    #[test]
    fn a_working_agent_with_a_request_open_waits_on_the_person() {
        assert_eq!(shown_state(AgentState::Working, true), AgentState::AwaitingPermission);
        assert_eq!(shown_state(AgentState::Working, false), AgentState::Working);
        assert_eq!(shown_state(AgentState::Idle, true), AgentState::Idle);
    }

    #[test]
    fn a_prompts_files_are_kept_under_one_plain_segment_whatever_they_were_called() {
        let hash = "0123456789abcdef0123";
        assert_eq!(attachment_name(hash, "shot.png"), "prompt/0123456789ab/shot.png");
        assert_eq!(attachment_name(hash, "../../etc/passwd"), "prompt/0123456789ab/passwd");
        assert_eq!(attachment_name(hash, r"C:\Users\me\notes.txt"), "prompt/0123456789ab/notes.txt");
        assert_eq!(attachment_name(hash, ".."), "prompt/0123456789ab/file");
        assert_eq!(attachment_name(hash, "a/"), "prompt/0123456789ab/file");
        assert_eq!(attachment_name(hash, "line\nbreak"), "prompt/0123456789ab/line_break");
        assert!(silt_agents::is_side_file_name(&attachment_name(hash, "dir/.")));
    }

    #[test]
    fn files_arrive_whole_and_within_bounds() {
        let file = |data: String| PromptAttachment { name: "f".into(), mime: "text/plain".into(), data };
        let decoded = decode(vec![file(BASE64.encode("hi"))]).expect("decodes");
        assert_eq!(decoded[0].data, b"hi");
        assert!(decode(vec![file("not base64!".into())]).is_err());
        assert!(decode(vec![file(BASE64.encode(vec![0; ATTACHMENT_BYTES + 1]))]).is_err());
        let large = BASE64.encode(vec![0; ATTACHMENT_BYTES]);
        assert!(decode(vec![file(large.clone()), file(large.clone()), file(large)]).is_err());
    }

    #[tokio::test]
    async fn an_agent_that_ran_before_a_restart_reads_as_stopped() {
        let data = tempfile::tempdir().expect("temp dir");
        let store = Store::open(data.path()).await.expect("opens");
        let (thread, family) = (ThreadId(7), FamilyId(9));
        let record = ThreadRecord {
            id: thread,
            family,
            repo_root: PathBuf::new(),
            repo: String::new(),
            created_ms: 1,
            owner: String::new(),
        };
        store.create_thread(&record).await.expect("thread");
        let run = |state: &str| AgentRunRecord {
            agent: "Fake agent".into(),
            state: state.into(),
            error: None,
            preset: "ask_before_commands".into(),
            mode: None,
            updated_ms: 1,
        };
        store.put_agent_run(thread, &run("working")).await.expect("saves");
        let (events, _inbox) = mpsc::unbounded_channel();
        let core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        assert_eq!(core.agent_state(thread), Some(AgentState::Stopped));
        assert_eq!(core.store.agent_runs().await.expect("reads")[&thread].state, "stopped");
    }
}
