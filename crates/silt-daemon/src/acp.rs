//! Agents the web UI starts over ACP (ADR 0014): one per thread, run in the
//! thread's checkout on this machine and recorded as a session of the `acp`
//! adapter, so history, search, forks and handoffs treat it like any other.

use std::path::{Path, PathBuf};

use silt_acp::registry::{self, AgentInfo};
use silt_acp::{AgentHandle, Command, Permission, Start, Update};
use silt_agents::acp;
use silt_log::{ConvOp, Op};
use silt_types::ThreadId;
use tokio::sync::mpsc;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::protocol::{AgentChoice, AgentChoices, AgentStatus, CloneInfo, PermissionAsked};
use crate::sessions::NewSession;

pub(crate) struct AcpRun {
    run: u64,
    info: AgentInfo,
    checkout_id: i64,
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
    state: RunState,
    text: String,
    thought: String,
    asking: Vec<Permission>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
    Starting,
    Ready,
    Working,
    Exited,
}

impl RunState {
    /// The name the web UI shows.
    fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Working => "working",
            Self::Exited => "exited",
        }
    }
}

/// What a running agent reports, tagged with its run so a stopped one's
/// late events are ignored.
pub enum AcpEvent {
    Started(Result<AgentHandle, String>),
    Agent(silt_acp::Event),
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
        AgentChoices {
            agents: registry::all(&self.data_dir, &silt_exe())
                .into_iter()
                .map(|agent| AgentChoice { id: agent.id, name: agent.name, description: agent.description })
                .collect(),
            clones: self.clones.iter().map(|(repo, path)| CloneInfo { repo: repo.clone(), path: path.clone() }).collect(),
        }
    }

    /// Starts `agent` in the thread's checkout, making one from a known
    /// clone if this machine has none, or in a new thread of `clone`.
    pub(crate) async fn acp_start(
        &mut self,
        thread: Option<&str>,
        clone: Option<&Path>,
        agent: &str,
    ) -> Result<String, CoreError> {
        let info = registry::all(&self.data_dir, &silt_exe())
            .into_iter()
            .find(|candidate| candidate.id == agent)
            .ok_or_else(|| CoreError::Invalid(format!("no agent called {agent}")))?;
        let (thread, checkout_id, checkout) = match (thread, clone) {
            (Some(selector), _) => {
                let thread = self.find_thread(selector)?;
                if self.acp.get(&thread).is_some_and(|run| run.state != RunState::Exited) {
                    return Err(CoreError::Invalid("an agent is already running in this thread".into()));
                }
                let (checkout_id, checkout) = self.checkout_for_agent(thread).await?;
                (thread, checkout_id, checkout)
            }
            (None, Some(clone)) => {
                let created = self.create_thread(clone, true, false).await?;
                if let Some(commit) = created.head {
                    self.record_bundle(created.family, created.thread, &created.repo, commit, &[]).await?;
                }
                (created.thread, created.checkout_id, created.checkout)
            }
            (None, None) => return Err(CoreError::Invalid("pick a thread or a clone to start the agent in".into())),
        };
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
                state: RunState::Starting,
                text: String::new(),
                thought: String::new(),
                asking: Vec::new(),
                error: None,
            },
        );
        Ok(thread.to_string())
    }

    async fn checkout_for_agent(&mut self, thread: ThreadId) -> Result<(i64, PathBuf), CoreError> {
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

    pub(crate) async fn acp_prompt(&mut self, selector: &str, text: &str) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        let run = self.running(thread)?;
        if run.state == RunState::Starting {
            return Err(CoreError::Invalid("the agent is still starting".into()));
        }
        if run.state == RunState::Working {
            return Err(CoreError::Invalid("the agent is still working; wait for it or stop it".into()));
        }
        let me = self.own_user();
        let mut authors = vec![me];
        let mut said = text.trim().to_owned();
        for (user, draft) in self.teammates_draft_list(thread).await {
            said.push_str(&format!("\n\n{user}: {draft}"));
            authors.push(user);
        }
        if said.trim().is_empty() {
            return Err(CoreError::Invalid("there is nothing to send".into()));
        }
        let mut prompt = said.clone();
        if let Some(notices) = self.take_notices(thread) {
            prompt = format!("{prompt}\n\n{notices}");
        }
        if let Some(primer) = self.acp.get_mut(&thread).and_then(|run| run.primer.take()) {
            prompt = format!("{primer}\n\n{prompt}");
        }
        self.record_acp(thread, &acp::prompt(&said, &authors.join(", "))).await?;
        let run = self.acp.get_mut(&thread).expect("checked above");
        run.state = RunState::Working;
        run.error = None;
        send(run, Command::Prompt(prompt));
        Ok("sent")
    }

    pub(crate) fn acp_command(&mut self, selector: &str, command: Command) -> Result<&'static str, CoreError> {
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
        Ok("ok")
    }

    fn running(&self, thread: ThreadId) -> Result<&AcpRun, CoreError> {
        self.acp
            .get(&thread)
            .filter(|run| run.state != RunState::Exited)
            .ok_or_else(|| CoreError::Invalid("no agent is running in this thread".into()))
    }

    pub(crate) fn agent_status(&self, thread: ThreadId) -> Option<AgentStatus> {
        let run = self.acp.get(&thread)?;
        Some(AgentStatus {
            agent: run.info.name.clone(),
            state: run.state.name().to_owned(),
            streaming: run.text.clone(),
            thinking: run.thought.clone(),
            permissions: run
                .asking
                .iter()
                .map(|asked| PermissionAsked { id: asked.id, title: asked.title.clone(), options: asked.options.clone() })
                .collect(),
            error: run.error.clone(),
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
                run.state = RunState::Exited;
            }
            AcpEvent::Agent(silt_acp::Event::Ready { session, loaded, agent }) => {
                self.acp_ready(thread, &session, loaded, &agent).await?
            }
            AcpEvent::Agent(silt_acp::Event::Update(update)) => self.acp_update(thread, update).await?,
            AcpEvent::Agent(silt_acp::Event::Permission(permission)) => run.asking.push(permission),
            AcpEvent::Agent(silt_acp::Event::TurnEnded { reason }) => {
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::stop(&reason)).await?;
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.state = RunState::Ready;
                run.asking.clear();
            }
            AcpEvent::Agent(silt_acp::Event::Failed(error)) => run.error = Some(error),
            AcpEvent::Agent(silt_acp::Event::Exited) => {
                self.flush_acp(thread).await?;
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.state = RunState::Exited;
                run.handle = None;
                run.asking.clear();
                let session = run.session.clone();
                if let Some(session) = session.and_then(|id| self.session_mut(&id)) {
                    session.live = false;
                    session.in_turn = false;
                    session.open_tool = None;
                }
            }
        }
        Ok(())
    }

    /// The agent's session is up: a loaded one continues its recording, a
    /// new one starts its own, primed with the thread's conversation.
    async fn acp_ready(&mut self, thread: ThreadId, native: &str, loaded: bool, name: &str) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.state = RunState::Ready;
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
                self.acp.get_mut(&thread).expect("caller checked").text.push_str(&text);
            }
            Update::Thought(text) => {
                if !run.text.is_empty() {
                    self.flush_acp(thread).await?;
                }
                self.acp.get_mut(&thread).expect("caller checked").thought.push_str(&text);
            }
            Update::UserText(_) => {}
            Update::ToolCall { id, title, kind, input } => {
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::tool_call(&id, &title, &kind, input.as_ref())).await?;
            }
            Update::ToolUpdate { id, status, output } => {
                if let Some(status @ ("completed" | "failed")) = status.as_deref() {
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
        if !thought.is_empty() {
            self.record_acp(thread, &acp::thought(&thought)).await?;
        }
        if !text.is_empty() {
            self.record_acp(thread, &acp::message(&text)).await?;
        }
        Ok(())
    }

    async fn record_acp(&mut self, thread: ThreadId, line: &str) -> Result<(), CoreError> {
        let Some(session) = self.acp.get(&thread).and_then(|run| run.session.clone()) else {
            return Ok(());
        };
        self.record_line(&session, "main", line.to_owned(), None).await
    }
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
            let exited = event == silt_acp::Event::Exited;
            if events.send(Event::Acp { thread, run, event: AcpEvent::Agent(event) }).is_err() || exited {
                break;
            }
        }
    });
}
