//! Agents asking other agents (ADR 0117): an agent hands part of its work to
//! an agent on another harness or model, which works in a thread of its own
//! in its caller's family and reports back with its last message.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use silt_acp::Command;
use silt_store::AgentMessageKind;
use silt_types::ThreadId;
#[cfg(test)]
use silt_workspace::Preset;
use silt_workspace::{AgentKey, AskPolicy, agent_key};
use tokio::sync::{oneshot, watch};

use crate::CoreError;
use crate::core::{Core, now_ms, to_response};
use crate::protocol::{
    AgentChoice, AgentSetting, AskCheckout, AskInput, AskMode, AskState, AskView, Askable, BusEvent, Request, Response,
};

/// How long a call waits for a report: below the minute Silt gives a
/// request, and below MCP clients' own timeouts.
const ASK_WAIT: Duration = Duration::from_secs(50);
const ASKS_FILE: &str = "asks.json";
/// The longest prompt an agent may ask with.
const PROMPT_CHARS: usize = 20_000;

/// The asks this machine knows, kept in `asks.json`.
#[derive(Default)]
pub(crate) struct Asks {
    list: Vec<AskView>,
    /// Each running ask's state, for the calls waiting on it.
    changes: HashMap<String, Arc<watch::Sender<AskView>>>,
    /// How many calls wait on each, so a report one of them takes isn't
    /// handed over again.
    waiting: HashMap<String, Arc<AtomicUsize>>,
    /// Callers with a report to hand over once their turn ends.
    reports_waiting: HashSet<ThreadId>,
}

impl Asks {
    /// What `asks.json` holds; an ask a stopped daemon left running ended
    /// with it, as its agent did.
    pub(crate) fn load(data_dir: &Path) -> Self {
        let mut list: Vec<AskView> =
            std::fs::read(data_dir.join(ASKS_FILE)).ok().and_then(|json| serde_json::from_slice(&json).ok()).unwrap_or_default();
        for ask in list.iter_mut().filter(|ask| ask.state == AskState::Running) {
            ask.state = AskState::Stopped;
            ask.why = Some("Silt's daemon stopped while it ran".into());
            ask.ended_ms.get_or_insert(ask.started_ms);
        }
        Self { list, ..Self::default() }
    }

    fn save(&self, data_dir: &Path) {
        let path = data_dir.join(ASKS_FILE);
        let partial = data_dir.join(format!("{ASKS_FILE}.partial"));
        let written = serde_json::to_vec_pretty(&self.list)
            .map_err(std::io::Error::other)
            .and_then(|json| std::fs::write(&partial, json))
            .and_then(|()| std::fs::rename(&partial, &path));
        if let Err(error) = written {
            tracing::warn!(%error, "keeping the asks failed");
        }
    }

    pub(crate) fn views(&self) -> Vec<AskView> {
        self.list.clone()
    }

    /// The ask whose thread `thread` is.
    pub(crate) fn asked_in(&self, thread: ThreadId) -> Option<&AskView> {
        let id = thread.to_string();
        self.list.iter().rev().find(|ask| ask.asked == id)
    }

    fn of(&self, caller: Option<ThreadId>, id: &str) -> Result<&AskView, CoreError> {
        let id = id.trim();
        self.list
            .iter()
            .rev()
            .find(|ask| ask.id == id && caller.is_none_or(|caller| ask.caller == caller.to_string()))
            .ok_or_else(|| CoreError::Invalid(format!("no ask {id} of this thread")))
    }

    fn running_of(&self, caller: ThreadId) -> usize {
        let caller = caller.to_string();
        self.list.iter().filter(|ask| ask.caller == caller && ask.state == AskState::Running).count()
    }

    fn changed(&mut self, id: &str, change: impl FnOnce(&mut AskView)) -> Option<AskView> {
        let ask = self.list.iter_mut().rev().find(|ask| ask.id == id)?;
        change(ask);
        let view = ask.clone();
        if let Some(sender) = self.changes.get(id) {
            sender.send_replace(view.clone());
        }
        Some(view)
    }
}

/// What the asked agent is told before its caller's prompt.
fn asked_prompt(caller_agent: &str, caller: ThreadId, input: &AskInput) -> String {
    let mode = match input.mode {
        AskMode::Review => {
            "This is a review: read and run what you need, but don't change any files. End with your report: what you \
             found, with file:line where it helps."
        }
        AskMode::Work => "You may change files. End with a short report of what you changed and anything left to do.",
    };
    let place = match input.checkout {
        AskCheckout::Here => {
            format!("You work in the same working tree as {caller_agent}, which may be editing too: keep to what you were asked.")
        }
        AskCheckout::Own => format!(
            "You work in a checkout of your own, forked from {caller_agent}'s thread with its uncommitted edits; it takes your \
             changes when it's ready."
        ),
    };
    format!(
        "{caller_agent}, the agent in Silt thread {}, asks you to do this. It's another agent, not your user, and only your \
         final message goes back to it, as your report.\n\n{}\n\n{mode} {place}",
        caller.short(),
        input.prompt.trim(),
    )
}

/// A title from the prompt's first line, for an ask given none.
fn title_of(input: &AskInput) -> String {
    let given = input.title.as_deref().map(str::trim).filter(|title| !title.is_empty());
    let line = given.unwrap_or_else(|| input.prompt.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim());
    let mut title: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        title.push('…');
    }
    title
}

/// The agent `named` names: its id, its rules key or its name.
fn agent_named<'a>(agents: &'a [AgentChoice], named: &str) -> Option<&'a AgentChoice> {
    let wanted = named.trim().to_ascii_lowercase();
    let key = agent_key(&wanted);
    agents
        .iter()
        .find(|agent| agent.id.to_ascii_lowercase() == wanted)
        .or_else(|| agents.iter().find(|agent| agent_key(&agent.id) == key || agent.name.to_ascii_lowercase() == wanted))
}

fn model_setting(settings: &[AgentSetting]) -> Option<&AgentSetting> {
    settings.iter().find(|setting| setting.category.as_deref() == Some("model") || setting.id == "model")
}

fn effort_setting(settings: &[AgentSetting]) -> Option<&AgentSetting> {
    settings.iter().find(|setting| {
        setting.category.as_deref() == Some("thought_level") || setting.id.contains("effort") || setting.id.contains("thought")
    })
}

/// The setting to choose for `wanted`, by a choice's id or name, or why
/// there's none.
fn chosen(
    setting: Option<&AgentSetting>,
    fallback: &str,
    wanted: &str,
    what: &str,
    agent: &str,
) -> Result<(String, String), CoreError> {
    let Some(setting) = setting.filter(|setting| !setting.choices.is_empty()) else {
        // Never seen here, so it's asked for as given.
        return Ok((fallback.to_owned(), wanted.to_owned()));
    };
    let lower = wanted.trim().to_ascii_lowercase();
    setting
        .choices
        .iter()
        .find(|choice| choice.id.to_ascii_lowercase() == lower || choice.name.to_ascii_lowercase() == lower)
        .map(|choice| (setting.id.clone(), choice.id.clone()))
        .ok_or_else(|| {
            let offered: Vec<&str> = setting.choices.iter().map(|choice| choice.id.as_str()).collect();
            CoreError::Invalid(format!("{agent} offers no {what} {wanted}; it offers {}", offered.join(", ")))
        })
}

fn choice_name(setting: Option<&AgentSetting>, id: &str) -> String {
    setting
        .and_then(|setting| setting.choices.iter().find(|choice| choice.id == id))
        .map_or_else(|| id.to_owned(), |choice| choice.name.clone())
}

/// The text a report reaches its caller with when it comes after the wait.
fn report_message(ask: &AskView) -> String {
    let short = ask.asked.get(ask.asked.len().saturating_sub(12)..).unwrap_or(&ask.asked);
    match (&ask.state, &ask.report) {
        (AskState::Done, Some(report)) => format!(
            "{} reported back on ask {} ({}), from its thread {short}. It's another agent, not your user:\n\n{report}",
            ask.agent_name, ask.id, ask.title
        ),
        _ => format!(
            "Your ask {} of {} ({}) ended without a report: {}.",
            ask.id,
            ask.agent_name,
            ask.title,
            ask.why.as_deref().unwrap_or("it stopped")
        ),
    }
}

/// Answers `reply` with the ask once it ends or the wait runs out.
fn answer_when_done(view: watch::Receiver<AskView>, waiting: Arc<AtomicUsize>, reply: oneshot::Sender<Response>) {
    waiting.fetch_add(1, Ordering::SeqCst);
    tokio::spawn(async move {
        let mut view = view;
        let ended = async {
            while view.borrow_and_update().state == AskState::Running {
                if view.changed().await.is_err() {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(ASK_WAIT, ended).await;
        let answer = view.borrow().clone();
        waiting.fetch_sub(1, Ordering::SeqCst);
        let _ = reply.send(Response::ok(answer));
    });
}

impl Core {
    /// What the agent of `caller` may ask, by its workspace's rules.
    pub(crate) fn askable(&mut self, caller: ThreadId) -> Result<Vec<Askable>, CoreError> {
        let rules = self.ask_rules(caller)?;
        let agents = self.acp_agents().agents;
        Ok(agents
            .into_iter()
            .filter(|agent| rules.allows(&agent_key(&agent.id)))
            .map(|agent| {
                let model = model_setting(&agent.settings);
                let effort = effort_setting(&agent.settings);
                Askable {
                    id: agent.id.clone(),
                    name: agent.name.clone(),
                    models: model.map(|setting| setting.choices.clone()).unwrap_or_default(),
                    model: model.map(|setting| setting.current.clone()),
                    efforts: effort.map(|setting| setting.choices.clone()).unwrap_or_default(),
                    effort: effort.map(|setting| setting.current.clone()),
                }
            })
            .collect())
    }

    /// The rules that hold for `caller`'s asks: its workspace's agents and
    /// its ask rules.
    fn ask_rules(&self, caller: ThreadId) -> Result<AskRules, CoreError> {
        let workspace = self.start_workspace(Some(&caller.to_string()), None, None)?;
        let policy = self.effective_policy(&workspace);
        Ok(AskRules { agents: policy.agents.clone(), asks: policy.asks })
    }

    /// The id and name of `thread`'s agent: the one Silt runs, else the one
    /// recording there.
    fn agent_in(&self, thread: ThreadId) -> (String, String) {
        if let Some(run) = self.acp.get(&thread) {
            return (run.info.id.clone(), run.info.name.clone());
        }
        self.sessions.iter().rev().find(|session| session.record.thread == thread && session.live).map_or_else(
            || (String::new(), "Another agent".to_owned()),
            |session| (session.agent.id().to_owned(), session.agent.display_name().to_owned()),
        )
    }

    /// Where `thread`'s agent works on this machine.
    fn checkout_of(&self, thread: ThreadId) -> Option<PathBuf> {
        if let Some(run) = self.acp.get(&thread) {
            return Some(run.checkout.clone());
        }
        self.checkouts.values().find(|checkout| checkout.record.thread == thread).map(|checkout| checkout.record.path.clone())
    }

    /// `caller`'s agent asks another; `reply` gets the ask once it ends or
    /// the wait runs out, or at once without a wait.
    pub(crate) async fn ask(&mut self, caller: ThreadId, input: AskInput, reply: oneshot::Sender<Response>) {
        let wait = input.wait;
        match self.start_ask(caller, input).await {
            Ok(id) if wait => self.wait_on_ask(Some(caller), &id, reply),
            Ok(id) => {
                let view = self.asks.of(None, &id).cloned();
                let _ = reply.send(to_response(view));
            }
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
            }
        }
    }

    pub(crate) fn wait_on_ask(&mut self, caller: Option<ThreadId>, id: &str, reply: oneshot::Sender<Response>) {
        let view = match self.asks.of(caller, id) {
            Ok(view) => view.clone(),
            Err(error) => {
                let _ = reply.send(to_response::<()>(Err(error)));
                return;
            }
        };
        let changes =
            self.asks.changes.entry(view.id.clone()).or_insert_with(|| Arc::new(watch::channel(view.clone()).0)).clone();
        let waiting = self.asks.waiting.entry(view.id.clone()).or_default().clone();
        answer_when_done(changes.subscribe(), waiting, reply);
    }

    async fn start_ask(&mut self, caller: ThreadId, input: AskInput) -> Result<String, CoreError> {
        if input.prompt.trim().is_empty() {
            return Err(CoreError::Invalid("say what to do in `prompt`".into()));
        }
        if input.prompt.chars().count() > PROMPT_CHARS {
            return Err(CoreError::Invalid(format!("the prompt is longer than {PROMPT_CHARS} characters")));
        }
        if let Some(to) = input.to.clone() {
            return self.follow_up(caller, &to, &input).await;
        }
        let named = input
            .agent
            .clone()
            .ok_or_else(|| CoreError::Invalid("name the `agent` to ask, as silt_askable lists them".into()))?;
        let rules = self.ask_rules(caller)?;
        let agents = self.acp_agents().agents;
        let agent = agent_named(&agents, &named)
            .ok_or_else(|| CoreError::Invalid(format!("no agent {named} on this machine; silt_askable lists them")))?
            .clone();
        let key = agent_key(&agent.id);
        if !rules.agents.as_ref().is_none_or(|agents| agents.contains(&key)) {
            return Err(CoreError::Invalid(format!("this workspace's rules don't allow {}", agent.name)));
        }
        rules
            .asks
            .check(&key, self.asks.asked_in(caller).is_some(), self.asks.running_of(caller))
            .map_err(|refusal| CoreError::Invalid(refusal.to_string()))?;
        let model_choice = model_setting(&agent.settings);
        let effort_choice = effort_setting(&agent.settings);
        let picked = |wanted: Option<&str>, setting, fallback, what| {
            wanted
                .map(str::trim)
                .filter(|wanted| !wanted.is_empty())
                .map(|wanted| chosen(setting, fallback, wanted, what, &agent.name))
                .transpose()
        };
        let model_pick = picked(input.model.as_deref(), model_choice, "model", "model")?;
        let effort_pick = picked(input.effort.as_deref(), effort_choice, "effort", "effort level")?;
        // What it runs on, by name: what was asked for, else its default.
        let shown = |pick: &Option<(String, String)>, setting: Option<&AgentSetting>| {
            pick.as_ref()
                .map(|(_, value)| value.clone())
                .or_else(|| setting.map(|setting| setting.current.clone()))
                .map(|id| choice_name(setting, &id))
        };
        let (model, effort) = (shown(&model_pick, model_choice), shown(&effort_pick, effort_choice));
        let settings: Vec<(String, String)> = model_pick.into_iter().chain(effort_pick).collect();
        let caller_checkout =
            self.checkout_of(caller).ok_or_else(|| CoreError::Invalid("your thread has no checkout on this machine".into()))?;

        let asked = self.fork_thread_at(caller, None, None).await?;
        // Named for what it was asked, not for its first prompt, which says who asks.
        let family = self.threads[&asked].family;
        self.append_conv(family, asked, silt_log::Author::User, silt_log::ConvOp::Title(title_of(&input))).await?;
        self.register_thread_now(asked).await?;
        self.take_lease(asked).await?;
        match input.checkout {
            AskCheckout::Own => {
                let clone = self.threads[&caller].repo_root.clone();
                let clone = if clone.as_os_str().is_empty() { caller_checkout.clone() } else { clone };
                self.check_out_managed(asked, &clone).await?;
            }
            AskCheckout::Here => {
                let record = self.bind_checkout(asked, &caller_checkout, false).await?;
                self.start_checkout(record, BTreeMap::new())?;
            }
        }
        let (caller_agent_id, caller_agent) = self.agent_in(caller);
        let preset = rules.asks.preset_for(self.preset(caller));
        // Random, unlike a v7 id's start, which two asks a minute apart share.
        let id = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
        let view = AskView {
            id: id.clone(),
            caller: caller.to_string(),
            caller_agent: caller_agent.clone(),
            caller_agent_id,
            asked: asked.to_string(),
            agent: agent.id.clone(),
            agent_name: agent.name.clone(),
            model,
            effort,
            mode: input.mode,
            checkout: input.checkout,
            title: title_of(&input),
            prompt: input.prompt.trim().to_owned(),
            state: AskState::Running,
            report: None,
            why: None,
            started_ms: now_ms(),
            ended_ms: None,
            taken: false,
        };
        // Known as asked before it starts, so it isn't primed with its caller's conversation.
        self.asks.list.push(view.clone());
        self.asks.changes.insert(id.clone(), Arc::new(watch::channel(view).0));
        let options = crate::protocol::StartOptions {
            settings,
            preset: Some(preset),
            prompt: Some(asked_prompt(&caller_agent, caller, &input)),
            ..crate::protocol::StartOptions::default()
        };
        if let Err(error) = self.acp_start(Some(&asked.to_string()), None, &agent.id, options).await {
            self.end_ask(&id, AskState::Failed, None, Some(format!("{} couldn't start: {error}", agent.name))).await;
            return Err(error);
        }
        self.first_from(asked, format!("{caller_agent}, from thread {}", caller.short()));
        self.asks.save(&self.data_dir);
        self.announce(BusEvent::Asks);
        Ok(id)
    }

    /// Another prompt to an earlier ask's agent, in its thread.
    async fn follow_up(&mut self, caller: ThreadId, to: &str, input: &AskInput) -> Result<String, CoreError> {
        let ask = self.asks.of(Some(caller), to)?.clone();
        if ask.state == AskState::Running {
            return Err(CoreError::Invalid(format!("ask {} is still running; wait for its report first", ask.id)));
        }
        let asked = self.find_thread(&ask.asked)?;
        let author = format!("{}, from thread {}", ask.caller_agent, caller.short());
        let text = format!("{} follows up:\n\n{}", ask.caller_agent, input.prompt.trim());
        self.asks.changed(&ask.id, |ask| {
            ask.state = AskState::Running;
            ask.report = None;
            ask.why = None;
            ask.ended_ms = None;
        });
        let started = match self.acp.get(&asked).map(|run| run.working()) {
            Some(false) if self.agent_state(asked) == Some(silt_proto::AgentState::Idle) => {
                self.send_prompt(asked, &author, &text, Vec::new()).await
            }
            Some(_) => self.enqueue(asked, &text, false).await.map(|_| ()),
            None => {
                let options = crate::protocol::StartOptions { prompt: Some(text), ..crate::protocol::StartOptions::default() };
                let started = self.acp_start(Some(&ask.asked), None, &ask.agent, options).await.map(|_| ());
                if started.is_ok() {
                    self.first_from(asked, author);
                }
                started
            }
        };
        if let Err(error) = started {
            self.end_ask(&ask.id, AskState::Failed, None, Some(error.to_string())).await;
            return Err(error);
        }
        self.asks.save(&self.data_dir);
        self.announce(BusEvent::Asks);
        Ok(ask.id)
    }

    /// Stops an ask: its caller's agent, or the person when `caller` is
    /// unset.
    pub(crate) async fn ask_stop(&mut self, caller: Option<ThreadId>, id: &str) -> Result<AskView, CoreError> {
        let ask = self.asks.of(caller, id)?.clone();
        if ask.state != AskState::Running {
            return Ok(ask);
        }
        let why = if caller.is_some() { format!("{} stopped it", ask.caller_agent) } else { "you stopped it".to_owned() };
        self.stop_ask(&ask, why).await
    }

    async fn stop_ask(&mut self, ask: &AskView, why: String) -> Result<AskView, CoreError> {
        if let Ok(asked) = self.find_thread(&ask.asked)
            && self.acp.get(&asked).is_some_and(|run| run.working())
        {
            self.acp_command(&ask.asked, Command::Cancel).await?;
        }
        self.end_ask(&ask.id, AskState::Stopped, None, Some(why)).await;
        self.asks.of(None, &ask.id).cloned()
    }

    /// Takes an ask's own checkout's changes into its caller, as a pull.
    pub(crate) async fn ask_take(&mut self, caller: Option<ThreadId>, id: &str) -> Result<String, CoreError> {
        let ask = self.asks.of(caller, id)?.clone();
        if ask.checkout == AskCheckout::Here {
            return Ok(format!("Ask {} worked in your working tree: its changes are there already.", ask.id));
        }
        if ask.state == AskState::Running {
            return Err(CoreError::Invalid(format!("ask {} is still running; take its changes once it reports", ask.id)));
        }
        let (into, from) = (self.find_thread(&ask.caller)?, self.find_thread(&ask.asked)?);
        let files = self.take_changes(into, from).await?;
        self.asks.changed(&ask.id, |ask| ask.taken = true);
        self.asks.save(&self.data_dir);
        self.announce(BusEvent::Asks);
        Ok(if files.is_empty() {
            format!("Ask {} changed no files; there was nothing to take.", ask.id)
        } else {
            format!("Took ask {}'s changes into your thread: {}.", ask.id, files.join(", "))
        })
    }

    /// An asked agent's turn ended: its last message is its report.
    /// `stopped` says the turn was stopped, not cancelled for a steer, which
    /// stops what its agent asked too.
    pub(crate) async fn ask_turn_ended(&mut self, thread: ThreadId, reason: &silt_acp::StopReason, stopped: bool, said: String) {
        if stopped && reason == &silt_acp::StopReason::Cancelled {
            self.stop_callers_asks(thread).await;
        }
        let Some(ask) = self.asks.asked_in(thread).filter(|ask| ask.state == AskState::Running).cloned() else {
            return;
        };
        let report = said.trim().to_owned();
        match reason {
            silt_acp::StopReason::Cancelled => {
                self.end_ask(&ask.id, AskState::Stopped, None, Some("its turn was stopped".into())).await;
            }
            silt_acp::StopReason::Refusal => {
                self.end_ask(&ask.id, AskState::Failed, Some(report), Some(format!("{} refused", ask.agent_name))).await;
            }
            _ => {
                let report =
                    if report.is_empty() { format!("{} ended its turn without a report.", ask.agent_name) } else { report };
                self.end_ask(&ask.id, AskState::Done, Some(report), None).await;
            }
        }
    }

    /// An asked agent couldn't go on: it didn't start, needs signing in, or
    /// its process ended.
    pub(crate) async fn ask_ended_early(&mut self, thread: ThreadId, why: String) {
        if let Some(ask) = self.asks.asked_in(thread).filter(|ask| ask.state == AskState::Running).cloned() {
            self.end_ask(&ask.id, AskState::Failed, None, Some(why)).await;
        }
        self.stop_callers_asks(thread).await;
    }

    /// Stopping a caller's turn stops what it asked.
    async fn stop_callers_asks(&mut self, caller: ThreadId) {
        let caller_id = caller.to_string();
        let running: Vec<AskView> =
            self.asks.list.iter().filter(|ask| ask.caller == caller_id && ask.state == AskState::Running).cloned().collect();
        for ask in running {
            let why = format!("{}'s turn was stopped", ask.caller_agent);
            if let Err(error) = self.stop_ask(&ask, why).await {
                tracing::warn!(ask = %ask.id, "stopping an ask with its caller failed: {error}");
            }
        }
    }

    async fn end_ask(&mut self, id: &str, state: AskState, report: Option<String>, why: Option<String>) {
        // Whether a call waits on it is read before it's told, so a report
        // the call takes isn't handed over a second time.
        let waited = self.asks.waiting.get(id).is_some_and(|waiting| waiting.load(Ordering::SeqCst) > 0);
        let Some(view) = self.asks.changed(id, |ask| {
            ask.state = state;
            ask.report = report;
            ask.why = why;
            ask.ended_ms = Some(now_ms());
        }) else {
            return;
        };
        self.asks.save(&self.data_dir);
        self.announce(BusEvent::Asks);
        if !waited && let Err(error) = self.hand_over(&view).await {
            tracing::warn!(ask = %view.id, "handing a report over failed: {error}");
        }
    }

    /// Gives the caller a report no call took: after its next tool call or
    /// with its next prompt, waking it if Silt runs it and it waits.
    async fn hand_over(&mut self, ask: &AskView) -> Result<(), CoreError> {
        let (caller, asked) = (self.find_thread(&ask.caller)?, self.find_thread(&ask.asked)?);
        self.store.add_agent_message(caller, asked, AgentMessageKind::Report, &report_message(ask), now_ms()).await?;
        self.asks.reports_waiting.insert(caller);
        self.hand_over_reports(caller).await
    }

    /// Wakes a caller Silt runs that waits, with the reports it was handed.
    pub(crate) async fn hand_over_reports(&mut self, caller: ThreadId) -> Result<(), CoreError> {
        if !self.asks.reports_waiting.contains(&caller) {
            return Ok(());
        }
        let idle = self.acp.get(&caller).is_some_and(|run| !run.working())
            && self.agent_state(caller) == Some(silt_proto::AgentState::Idle);
        if !idle {
            // A terminal's agent takes them after its next tool call, or with its next prompt.
            if !self.acp.contains_key(&caller) {
                self.asks.reports_waiting.remove(&caller);
            }
            return Ok(());
        }
        self.asks.reports_waiting.remove(&caller);
        self.send_prompt(caller, "Silt", "The agents you asked reported back.", Vec::new()).await
    }

    /// Answers one of the ask requests: from `caller`'s agent, or from the
    /// person, who may only stop an ask or take its changes.
    pub(crate) async fn ask_request(&mut self, caller: Option<ThreadId>, request: Request, reply: oneshot::Sender<Response>) {
        let answer = match (caller, request) {
            (Some(caller), Request::Askable { .. }) => to_response(self.askable(caller)),
            (Some(caller), Request::Ask { ask, .. }) => return self.ask(caller, ask, reply).await,
            (Some(caller), Request::AskWait { ask, .. }) => return self.wait_on_ask(Some(caller), &ask, reply),
            (caller, Request::AskStop { ask, .. }) => to_response(self.ask_stop(caller, &ask).await),
            (caller, Request::AskTake { ask, .. }) => to_response(self.ask_take(caller, &ask).await),
            _ => Response::error("only an agent asks, for its own thread"),
        };
        let _ = reply.send(answer);
    }

    /// Every ask, for the web UI.
    pub(crate) fn asks_view(&self) -> Vec<AskView> {
        self.asks.views()
    }
}

/// The rules for one caller's asks.
struct AskRules {
    agents: Option<std::collections::BTreeSet<AgentKey>>,
    asks: AskPolicy,
}

impl AskRules {
    fn allows(&self, agent: &AgentKey) -> bool {
        self.asks.allowed != Some(false)
            && self.asks.allows_agent(agent)
            && self.agents.as_ref().is_none_or(|agents| agents.contains(agent))
    }
}

/// The preset an asked agent starts at, for tests of the rules' effect.
#[cfg(test)]
fn started_at(rules: &AskPolicy, caller: Preset) -> Preset {
    rules.preset_for(caller)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AgentChoiceItem;

    fn input(prompt: &str) -> AskInput {
        AskInput { prompt: prompt.into(), wait: true, ..AskInput::default() }
    }

    fn setting(id: &str, category: &str, choices: &[(&str, &str)]) -> AgentSetting {
        AgentSetting {
            id: id.into(),
            name: id.into(),
            category: Some(category.into()),
            current: choices[0].0.into(),
            choices: choices
                .iter()
                .map(|(id, name)| AgentChoiceItem { id: (*id).into(), name: (*name).into(), description: None })
                .collect(),
        }
    }

    #[test]
    fn the_asked_agent_hears_who_asks_what_and_where() {
        let caller = ThreadId(0x0193_0000_0000_7000_8000_0000_0000_0001);
        let prompt = asked_prompt("Claude Code", caller, &input("Review src/retry.rs"));
        assert!(prompt.starts_with(&format!("Claude Code, the agent in Silt thread {}", caller.short())), "{prompt}");
        assert!(prompt.contains("not your user"));
        assert!(prompt.contains("Review src/retry.rs"));
        assert!(prompt.contains("don't change any files") && prompt.contains("same working tree"));
        let own = AskInput { mode: AskMode::Work, checkout: AskCheckout::Own, ..input("Write tests") };
        let prompt = asked_prompt("Claude Code", caller, &own);
        assert!(prompt.contains("You may change files") && prompt.contains("checkout of your own"));
    }

    #[test]
    fn a_title_is_the_prompts_first_line_unless_given() {
        assert_eq!(title_of(&input("\n  Review the retry\nin detail")), "Review the retry");
        let titled = AskInput { title: Some("Second opinion".into()), ..input("x") };
        assert_eq!(title_of(&titled), "Second opinion");
        assert!(title_of(&input(&"a".repeat(100))).ends_with('…'));
    }

    #[test]
    fn agents_are_named_by_id_key_or_name() {
        let agent = |id: &str, name: &str| AgentChoice {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            modes: None,
            settings: Vec::new(),
            takes_images: None,
            takes_files: None,
            commands: Vec::new(),
        };
        let agents = [agent("claude-acp", "Claude Agent"), agent("antigravity-acp", "Antigravity")];
        assert_eq!(agent_named(&agents, "antigravity-acp").map(|a| a.name.as_str()), Some("Antigravity"));
        assert_eq!(agent_named(&agents, "agy").map(|a| a.name.as_str()), Some("Antigravity"));
        assert_eq!(agent_named(&agents, "Claude Agent").map(|a| a.id.as_str()), Some("claude-acp"));
        assert_eq!(agent_named(&agents, "claude").map(|a| a.id.as_str()), Some("claude-acp"));
        assert!(agent_named(&agents, "codex").is_none());
    }

    #[test]
    fn model_and_effort_are_chosen_from_what_the_agent_offers() {
        let model = setting("model", "model", &[("gemini-4-neon", "Gemini 4 Neon"), ("gemini-4-hydrogen", "Gemini 4 Hydrogen")]);
        let effort = setting("reasoning_effort", "thought_level", &[("high", "High"), ("low", "Low")]);
        let settings = [model, effort];
        assert_eq!(
            chosen(model_setting(&settings), "model", "Gemini 4 Neon", "model", "Antigravity").unwrap(),
            ("model".into(), "gemini-4-neon".into())
        );
        assert_eq!(
            chosen(effort_setting(&settings), "effort", "low", "effort level", "Antigravity").unwrap(),
            ("reasoning_effort".into(), "low".into()),
            "effort keeps the agent's own setting id, whatever it calls it"
        );
        let refused = chosen(model_setting(&settings), "model", "gpt-5.5", "model", "Antigravity").unwrap_err();
        assert!(refused.to_string().contains("offers gemini-4-neon, gemini-4-hydrogen"), "{refused}");
        assert_eq!(chosen(None, "model", "anything", "model", "X").unwrap(), ("model".into(), "anything".into()));
    }

    #[test]
    fn a_report_after_the_wait_says_whose_it_is() {
        let ask = AskView {
            id: "4f1a9c02".into(),
            caller: "c".into(),
            caller_agent: "Claude Code".into(),
            caller_agent_id: "claude-acp".into(),
            asked: "0193abcd00007000".into(),
            agent: "antigravity-acp".into(),
            agent_name: "Antigravity".into(),
            model: None,
            effort: None,
            mode: AskMode::Review,
            checkout: AskCheckout::Here,
            title: "Review the retry".into(),
            prompt: "Review".into(),
            state: AskState::Done,
            report: Some("Two problems.".into()),
            why: None,
            started_ms: 0,
            ended_ms: Some(1),
            taken: false,
        };
        let said = report_message(&ask);
        assert!(
            said.starts_with("Antigravity reported back on ask 4f1a9c02 (Review the retry), from its thread abcd00007000"),
            "{said}"
        );
        assert!(said.ends_with("Two problems."));
        let failed = AskView { state: AskState::Failed, report: None, why: Some("it needs you to sign in".into()), ..ask };
        assert!(report_message(&failed).ends_with("ended without a report: it needs you to sign in."));
    }

    #[test]
    fn an_asked_agent_starts_no_higher_than_its_caller() {
        let capped = AskPolicy { max_preset: Some(Preset::AskBeforeCommands), ..AskPolicy::default() };
        assert_eq!(started_at(&capped, Preset::FullAccess), Preset::AskBeforeCommands);
        assert_eq!(started_at(&AskPolicy::default(), Preset::Auto), Preset::Auto);
    }

    #[test]
    fn a_daemon_that_stopped_stopped_its_asks() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let ask = AskView {
            id: "1".into(),
            caller: "c".into(),
            caller_agent: "Claude Code".into(),
            caller_agent_id: "claude-acp".into(),
            asked: "a".into(),
            agent: "codex-acp".into(),
            agent_name: "Codex".into(),
            model: None,
            effort: None,
            mode: AskMode::Work,
            checkout: AskCheckout::Own,
            title: "t".into(),
            prompt: "p".into(),
            state: AskState::Running,
            report: None,
            why: None,
            started_ms: 5,
            ended_ms: None,
            taken: false,
        };
        let asks = Asks { list: vec![ask], ..Asks::default() };
        asks.save(dir.path());
        let loaded = Asks::load(dir.path());
        assert_eq!(loaded.list[0].state, AskState::Stopped);
        assert_eq!(loaded.list[0].ended_ms, Some(5));
    }
}
