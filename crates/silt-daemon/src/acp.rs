//! Agents the web UI starts over ACP (ADR 0014): one per thread, run in the
//! thread's checkout on this machine and recorded as a session of the `acp`
//! adapter, so history, search, forks and handoffs treat it like any other.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};
use silt_acp::registry::{self, AgentInfo};
use silt_acp::{Accepts, AgentHandle, Attachment, Command, Permission, Question, Start, Update};
use silt_agents::acp;
use silt_log::{Author, ConvOp, Op, SessionFile};
use silt_proto::AgentState;
use silt_store::CheckoutId;
use silt_types::ThreadId;
use tokio::sync::mpsc;
use tracing::warn;

use crate::core::{Core, CoreError, Event};
use crate::protocol::{
    AgentChoice, AgentChoices, AgentStatus, CloneInfo, ContextUse, PermissionAsked, Preset, PromptAttachment, StartOptions,
    TextKind,
};
use crate::sessions::NewSession;

mod sign_in;

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
    /// When the thought being streamed began, which its record keeps.
    thought_ms: i64,
    asking: Vec<Permission>,
    /// Questions the agent asked and waits on.
    questions: Vec<Question>,
    context: Option<ContextUse>,
    error: Option<String>,
    /// Each running tool call's output so far, to tell browsers what's new.
    tool_output: HashMap<String, String>,
    /// What to tell the agent once the turn being cancelled for it ends.
    steering: Option<String>,
    /// Each tool call's kind as the agent announced it, for permission
    /// requests that don't repeat it.
    tool_kinds: HashMap<String, String>,
    /// Each tool call's title and input as last recorded, so an update
    /// that says no more is left out.
    tool_said: HashMap<String, (String, Option<serde_json::Value>)>,
    /// A subagent's message so far, by the call that started it; it is
    /// recorded apart from the agent's own.
    within: Option<(String, String)>,
    modes: Option<silt_acp::Modes>,
    settings: Vec<silt_acp::Setting>,
    commands: Vec<silt_acp::SlashCommand>,
    /// Settings chosen before the agent was up, by id and value, set once it
    /// says which it offers.
    chosen_settings: Vec<(String, String)>,
    /// The turn running is a Plan first one.
    planning: bool,
    /// The agent puts its plan to the person itself, so a turn it ends
    /// while planning isn't one to approve.
    presents_plans: bool,
    /// The mode Plan first switched away from, to go back to once the plan
    /// is approved.
    back_to: Option<String>,
    /// The prompt to send once the agent is ready.
    first: Option<First>,
    accepts: Accepts,
    /// The merge request whose change guide this turn writes, and what the
    /// agent has said in it so far (ADR 0025).
    guiding: Option<(silt_types::DeltaId, String)>,
    /// The agent says when it works and stops (ACP v2), so work it takes up
    /// by itself runs like a prompt's turn (ADR 0050).
    reports_work: bool,
    /// An agent that doesn't say when it works went on by itself: it works
    /// until it has been quiet a while, as nothing else says it stopped.
    own_turn: bool,
    /// When the agent last said something.
    said_at: Option<Instant>,
    /// Tool calls the agent started and hasn't finished.
    calls_running: HashSet<String>,
    /// What each call is, as a person names it: its command, or its
    /// subagent's description, by the call.
    labels: HashMap<String, String>,
    /// What ended between turns, such as a command left in the background,
    /// which is likely what the agent goes on about when it goes on by
    /// itself.
    woke_by: Vec<String>,
    /// Commands left running in the background, by the agent's id for each.
    background: HashMap<String, Background>,
    /// Subagents running, by the call that started each, which may go on
    /// after the turn.
    subagents: HashSet<String>,
    /// How many commands and subagents the thread list was last told of.
    background_listed: usize,
    /// Background commands the person stopped, by the call that started
    /// each: the agent's later news of their end is no news.
    stopped_tasks: HashSet<String>,
    /// A command the person stopped has ended since the agent last went on,
    /// by what it ran: the agent going on now goes on about that, which
    /// they've seen to.
    hush: Option<String>,
    /// The person stopped the agent and hasn't prompted it since: what it
    /// left running in the background doesn't get to start it again.
    stopped: bool,
    /// The agent needs signing in before it goes on (ADR 0056).
    sign_in: Option<sign_in::SignIn>,
    /// The last prompt sent, as the agent was given it.
    last_prompt: Option<(String, Vec<Attachment>)>,
    /// A prompt the agent refused for want of a sign-in, to send once it is
    /// up again.
    resend: Option<(String, Vec<Attachment>)>,
}

/// A command the agent left running in the background.
struct Background {
    /// The call that started it.
    call: String,
    started: Instant,
    /// What it runs, as the agent gave it.
    command: String,
}

/// How long an agent that went on by itself, and won't say when it stops,
/// is quiet, with no tool call running and nothing asked, before its turn
/// is taken to have ended. Copilot and Claude pause 2–4s between steps.
const OWN_TURN_QUIET: Duration = Duration::from_secs(5);

/// A prompt an agent was started with.
struct First {
    prompt: String,
    plan: bool,
    attachments: Vec<Attachment>,
}

/// What the fork's agent is asked for a change guide.
const GUIDE_PROMPT: &str = "Write a change guide for the reviewers of the merge request offered from this thread: \
                            walk through what changed and why, in the order that's easiest to read, naming files and \
                            the reasons behind each choice. Don't change any files; answer with the guide alone.";

/// What a Plan first prompt starts with when the agent has no plan mode.
const PLAN_FIRST: &str = "Before you change anything, write a plan for this and stop: don't edit files or run commands \
                          that change them until I approve the plan.";

impl AcpRun {
    /// The state shown for the run: a working agent with a request or a
    /// question open waits on the person (ADR 0022).
    fn shown(&self) -> AgentState {
        if self.sign_in.is_some() {
            return AgentState::AwaitingPermission;
        }
        match self.state {
            // Awaiting input stays the plan's to approve.
            AgentState::Working if !self.asking.is_empty() || !self.questions.is_empty() => AgentState::AwaitingPermission,
            other => other,
        }
    }

    /// How many commands and subagents it left running.
    fn in_background(&self) -> usize {
        self.background.len() + self.subagents.len()
    }

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
    /// What the sign-in `token` reports (ADR 0056).
    Login {
        token: u64,
        event: silt_agents::sign_in::LoginEvent,
    },
}

/// A question as browsers and other devices are told it.
pub(crate) fn question_asked(question: &Question) -> crate::protocol::QuestionAsked {
    use crate::protocol::{ChoiceAsked, QuestionFieldAsked};
    crate::protocol::QuestionAsked {
        id: question.id,
        message: question.message.clone(),
        fields: question
            .fields
            .iter()
            .map(|field| QuestionFieldAsked {
                key: field.key.clone(),
                title: field.title.clone(),
                description: field.description.clone(),
                options: field
                    .options
                    .iter()
                    .map(|choice| ChoiceAsked {
                        value: choice.value.clone(),
                        label: choice.label.clone(),
                        description: choice.description.clone(),
                    })
                    .collect(),
                multi: field.multi,
                required: field.required,
            })
            .collect(),
    }
}

/// The tool call id a question is recorded under.
fn question_call(id: u64) -> String {
    format!("question-{id}")
}

/// A question as Claude Code's `AskUserQuestion` records one, so it reads
/// back alike whichever agent asked it.
fn question_input(question: &Question) -> serde_json::Value {
    let asked: Vec<serde_json::Value> = question
        .fields
        .iter()
        .filter(|field| !optional_text(field))
        .map(|field| {
            serde_json::json!({
                "header": field.title,
                "question": asked_text(question, field),
                "multiSelect": field.multi,
                "options": field
                    .options
                    .iter()
                    .map(|choice| serde_json::json!({"label": choice.label, "description": choice.description}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({"questions": asked})
}

/// The answers to `question` as Claude Code words them in its result, by
/// each choice's label; refused when a field isn't asked or a value isn't
/// offered.
fn answer_text(question: &Question, answers: &[(String, Vec<String>)]) -> Result<String, CoreError> {
    if let Some((key, _)) = answers.iter().find(|(key, _)| !question.fields.iter().any(|field| field.key == *key)) {
        return Err(CoreError::Invalid(format!("the agent didn't ask for {key}")));
    }
    let mut said: Vec<(String, Vec<String>)> = Vec::new();
    for (index, field) in question.fields.iter().enumerate() {
        let Some((_, values)) = answers.iter().find(|(key, _)| *key == field.key) else {
            continue;
        };
        if !field.multi && values.len() > 1 {
            return Err(CoreError::Invalid(format!("{} takes one answer", field.title)));
        }
        let labels = values
            .iter()
            .map(|value| match field.options.iter().find(|choice| choice.value == *value) {
                Some(choice) => Ok(choice.label.clone()),
                None if field.options.is_empty() => Ok(value.clone()),
                None => Err(CoreError::Invalid(format!("the agent didn't offer {value} for {}", field.title))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A box of one's own answers the question before it.
        let of = match index.checked_sub(1).map(|before| &question.fields[before]) {
            Some(before) if optional_text(field) && !optional_text(before) => before,
            _ => field,
        };
        let asked = asked_text(question, of);
        match said.last_mut() {
            Some((last, given)) if *last == asked => given.extend(labels),
            _ => said.push((asked, labels)),
        }
    }
    let said: Vec<String> = said.iter().map(|(asked, given)| format!("\"{asked}\"=\"{}\"", given.join(", "))).collect();
    Ok(format!("User has answered your questions: {}. You can now continue with the user's answers in mind.", said.join(", ")))
}

/// What `field` asks: the form's message when it's the only question.
fn asked_text(question: &Question, field: &silt_acp::QuestionField) -> String {
    let questions = question.fields.iter().filter(|each| !optional_text(each)).count();
    if questions == 1 && !optional_text(field) {
        question.message.clone()
    } else {
        field.description.clone().unwrap_or_else(|| field.title.clone())
    }
}

/// A box for an answer of one's own beside a question, as Claude's adapter
/// follows each with, rather than a question itself.
fn optional_text(field: &silt_acp::QuestionField) -> bool {
    !field.required && field.options.is_empty()
}

/// An answer in the person's own words, as Claude Code words a result.
fn own_answer(question: &Question, text: &str) -> String {
    format!(
        "User has answered your questions: \"{}\"=\"{}\". You can now continue with the user's answers in mind.",
        question.message,
        text.trim()
    )
}

fn silt_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("silt"))
}

impl AcpRun {
    /// Whether its agent is busy, or someone is to answer it: stopping it
    /// now would lose that. An idle one is started again on the next prompt.
    pub(crate) fn working(&self) -> bool {
        self.handle.is_some()
            && matches!(
                self.state,
                AgentState::Starting | AgentState::Working | AgentState::AwaitingPermission | AgentState::AwaitingInput
            )
    }
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
        let mut seen = read_seen(&self.data_dir);
        let now = now_ms();
        let installed = registry::installed_here(&self.data_dir);
        // What each offers as last seen here, else as Silt knows it; one not asked lately is asked again, behind the answer.
        let asking = std::env::var_os("SILT_ASK_AGENTS").is_none_or(|value| value != "0");
        // One that didn't answer isn't asked again before then either, rather than on every look.
        for agent in &installed {
            if asking
                && seen.get(&agent.id).is_none_or(|seen| now - seen.seen_ms > ASK_AGAIN_MS)
                && self.asked.get(&agent.id).is_none_or(|asked| now - asked > ASK_AGAIN_MS)
            {
                self.asked.insert(agent.id.clone(), now);
                probe_agent(self.events.clone(), agent.clone(), self.data_dir.clone());
            }
        }
        AgentChoices {
            agents: installed
                .into_iter()
                .map(|agent| {
                    let Seen { modes, settings, takes, commands, .. } = seen.remove(&agent.id).unwrap_or_else(|| {
                        silt_acp::built_in(&agent.id).map_or_else(Seen::default, |(modes, settings)| Seen {
                            modes: Some((&modes).into()),
                            settings: settings.iter().map(Into::into).collect(),
                            takes: None,
                            seen_ms: 0,
                            presents_plans: false,
                            commands: Vec::new(),
                        })
                    });
                    AgentChoice {
                        id: agent.id,
                        name: agent.name,
                        description: agent.description,
                        modes,
                        settings,
                        takes_images: takes.map(|takes| takes.images),
                        takes_files: takes.map(|takes| takes.files),
                        commands,
                    }
                })
                .collect(),
            clones,
        }
    }

    /// Keeps what an agent said it offers when asked, and tells the web UI.
    pub(crate) fn agent_probed(&mut self, agent: String, offered: Option<Offered>) {
        let Some(Offered { modes, settings, accepts }) = offered else {
            return;
        };
        let seen = Seen {
            modes: modes.as_ref().map(Into::into),
            settings: settings.iter().map(Into::into).collect(),
            takes: Some(accepts.into()),
            seen_ms: now_ms(),
            presents_plans: false,
            commands: Vec::new(),
        };
        self.keep_seen(agent, seen);
        self.announce(crate::protocol::BusEvent::Agents);
    }

    fn keep_seen(&self, agent: String, mut kept: Seen) {
        let mut seen = read_seen(&self.data_dir);
        // Asking what an agent offers shows neither how it plans nor its commands.
        if let Some(old) = seen.get(&agent) {
            kept.presents_plans |= old.presents_plans;
            if kept.commands.is_empty() {
                kept.commands.clone_from(&old.commands);
            }
        }
        seen.insert(agent, kept);
        let written = serde_json::to_vec_pretty(&seen)
            .map_err(std::io::Error::other)
            .and_then(|json| std::fs::write(self.data_dir.join(SEEN), json));
        if let Err(error) = written {
            warn!(%error, "could not keep the agent's modes and settings");
        }
    }

    /// Keeps the thread's agent's modes and settings as it lists them now, for
    /// a new thread to offer before that agent runs again.
    fn remember_seen(&self, thread: ThreadId) {
        let Some(run) = self.acp.get(&thread) else {
            return;
        };
        let seen = Seen {
            modes: run.modes.as_ref().map(Into::into),
            settings: run.settings.iter().map(Into::into).collect(),
            takes: Some(run.accepts.into()),
            seen_ms: now_ms(),
            presents_plans: run.presents_plans,
            commands: run.commands.iter().map(Into::into).collect(),
        };
        self.keep_seen(run.info.id.clone(), seen);
    }

    /// Starts `agent` in the thread's checkout, making one from a known
    /// clone if this machine has none, or in a new thread of `clone`.
    pub(crate) async fn acp_start(
        &mut self,
        thread: Option<&str>,
        clone: Option<&Path>,
        agent: &str,
        mut options: StartOptions,
    ) -> Result<String, CoreError> {
        let info = registry::all(&self.data_dir)
            .into_iter()
            .find(|candidate| candidate.id == agent)
            .ok_or_else(|| CoreError::Invalid(format!("no agent called {agent}")))?;
        if options.prompt.as_ref().is_some_and(|prompt| prompt.trim().is_empty()) {
            return Err(CoreError::Invalid("the first prompt is empty".into()));
        }
        if options.prompt.is_none() && !options.attachments.is_empty() {
            return Err(CoreError::Invalid("attached files go with the first prompt".into()));
        }
        let workspace = self.start_workspace(thread, clone, options.workspace.as_deref())?;
        let chosen = options.preset.is_some();
        let asked = options.preset.or_else(|| match (thread, options.chat) {
            (Some(selector), _) => self.find_thread(selector).ok().map(|thread| self.preset(thread)),
            (None, true) => Some(crate::chats::settings(&self.data_dir).preset),
            (None, false) => Some(Preset::default()),
        });
        let allowed = self.check_acp_start(&workspace, &info, asked, chosen, options.mode.as_deref())?;
        if allowed != asked {
            options.preset = allowed;
        }
        let attachments = decode(std::mem::take(&mut options.attachments))?;
        let seen_accepts = read_seen(&self.data_dir).remove(&info.id).and_then(|seen| seen.takes).map(Accepts::from);
        if let Some(refused) = seen_accepts.and_then(|accepts| attachments.iter().find(|attachment| !accepts.takes(attachment))) {
            return Err(CoreError::Invalid(format!("{} doesn't take files like {}", info.name, refused.name)));
        }
        let continues = options.continues.as_deref().map(|selector| self.find_thread(selector)).transpose()?;
        if continues.is_some_and(|chat| !self.is_chat(chat)) {
            return Err(CoreError::Invalid("only a chat continues in a project".into()));
        }
        if options.copy_to.is_some() && continues.is_none() {
            return Err(CoreError::Invalid("files are copied from the chat a thread continues".into()));
        }
        let copy_to = options.copy_to.as_deref().map(copy_target).transpose()?;
        if options.chat && (thread.is_some() || clone.is_some() || options.base.is_some() || options.here || continues.is_some())
        {
            return Err(CoreError::Invalid("a chat starts in a folder of its own, not in a thread or clone".into()));
        }
        let (thread, checkout_id, checkout) = match (thread, clone) {
            (Some(selector), _) => {
                if options.base.is_some() || options.here || options.visibility.is_some() || continues.is_some() {
                    return Err(CoreError::Invalid(
                        "a thread keeps its checkout and visibility; those are for a new thread".into(),
                    ));
                }
                let thread = self.find_thread(selector)?;
                if self.acp.get(&thread).is_some_and(|run| run.state.running()) {
                    return Err(CoreError::Invalid("an agent is already running in this thread".into()));
                }
                let (checkout_id, checkout) = self.checkout_for_agent(thread).await?;
                self.take_in_what_changed_meanwhile(checkout_id).await?;
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
            (None, None) if options.chat => {
                let folder = crate::chats::new_folder_in(
                    &self.data_dir,
                    options.prompt.as_deref().unwrap_or_default(),
                    Some(workspace.id.as_str()),
                )?;
                let created = self.create_thread(&folder, false, false, None).await?;
                if options.preset.is_none() {
                    options.preset = Some(crate::chats::settings(&self.data_dir).preset);
                }
                (created.thread, created.checkout_id, created.checkout)
            }
            (None, None) => return Err(CoreError::Invalid("pick a thread or a clone to start the agent in".into())),
        };
        if let Some(chat) = continues
            && let (Some(to), Some(from)) = (&copy_to, self.chat_folder_here(chat))
        {
            copy_chat_files(&from, &checkout.join(to))?;
        }
        let primer = continues.and_then(|chat| {
            self.primer(chat).map(|transcript| {
                format!("This thread continues chat {} in this project. The chat so far:\n\n{transcript}", chat.short())
            })
        });
        if let Some(preset) = options.preset {
            self.keep_choice(thread, |record| record.preset = preset.as_str().to_owned()).await?;
        }
        if let Some(mode) = options.mode.clone() {
            // The preset follows the mode chosen, as it does once the agent runs (ADR 0048).
            let plan = read_seen(&self.data_dir).remove(&info.id).and_then(|seen| seen.modes?.plan);
            let follows = preset_following(&mode, plan.as_deref() == Some(mode.as_str()), self.preset(thread));
            self.keep_choice(thread, |record| {
                record.mode = Some(mode);
                record.preset = follows.as_str().to_owned();
            })
            .await?;
        }
        self.next_token += 1;
        let run = self.next_token;
        let load = self.earlier_session(thread, &info.id);
        let token = self.mint_run_token(thread, &workspace.id).await?;
        let mut start = self.agent_start(thread, &checkout, load.as_ref().map(|(_, native)| native.clone()));
        if let Some((_, args)) = &mut start.mcp {
            args.extend(["--token".to_owned(), token]);
        }
        start.env.extend(workspace.env.iter().map(|(name, value)| (name.clone(), value.clone())));
        let presents_plans =
            silt_acp::presents_plans(&info.id) || read_seen(&self.data_dir).get(&info.id).is_some_and(|seen| seen.presents_plans);
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
                primer,
                state: AgentState::Starting,
                text: String::new(),
                thought: String::new(),
                thought_ms: 0,
                asking: Vec::new(),
                questions: Vec::new(),
                context: None,
                error: None,
                tool_output: HashMap::new(),
                steering: None,
                tool_kinds: HashMap::new(),
                tool_said: HashMap::new(),
                within: None,
                modes: None,
                settings: Vec::new(),
                commands: Vec::new(),
                chosen_settings: options.settings,
                planning: false,
                presents_plans,
                back_to: None,
                first: options.prompt.map(|prompt| First { prompt, plan: options.plan, attachments }),
                accepts: Accepts::default(),
                guiding: None,
                reports_work: false,
                own_turn: false,
                said_at: None,
                calls_running: HashSet::new(),
                labels: HashMap::new(),
                woke_by: Vec::new(),
                background: HashMap::new(),
                subagents: HashSet::new(),
                background_listed: 0,
                stopped_tasks: HashSet::new(),
                hush: None,
                stopped: false,
                sign_in: None,
                last_prompt: None,
                resend: None,
            },
        );
        self.agent_changed(thread).await;
        Ok(thread.to_string())
    }

    /// How the thread's agent starts in `checkout`: with Silt's MCP server,
    /// and loading its session `load` when there is one.
    fn agent_start(&self, thread: ThreadId, checkout: &Path, load: Option<String>) -> Start {
        Start {
            program: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: checkout.to_path_buf(),
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
            load,
            sign_in_first: None,
        }
    }

    /// The branches a new thread of a clone this machine knows can start from.
    pub(crate) fn branches(&self, clone: &Path) -> Result<Vec<String>, CoreError> {
        if !self.clones.values().any(|known| known == clone) {
            return Err(CoreError::Invalid(format!("this machine knows no clone at {}", clone.display())));
        }
        Ok(silt_git::Repo::discover(clone)?.branches()?)
    }

    pub(crate) async fn checkout_for_agent(&mut self, thread: ThreadId) -> Result<(CheckoutId, PathBuf), CoreError> {
        if let Some(checkout) = self.checkouts.values().find(|checkout| checkout.record.thread == thread) {
            return Ok((checkout.record.id, checkout.record.path.clone()));
        }
        if self.is_chat(thread) {
            return self.check_out_chat(thread).await;
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
    /// the agent's own. A session a revert rewound past is none, as the agent
    /// would load it with the rewound turns.
    fn earlier_session(&self, thread: ThreadId, agent: &str) -> Option<(String, String)> {
        let family = self.threads.get(&thread)?.family;
        self.local_log(family)
            .iter()
            .rev()
            .filter(|delta| delta.thread == thread)
            .find_map(|delta| match &delta.op {
                Op::Conv(ConvOp::Rewound { .. }) => Some(None),
                Op::Conv(ConvOp::Native { session, line, .. }) if session.agent == acp::AGENT => {
                    acp::session_of(line).filter(|(id, _)| id == agent).map(|(_, native)| Some((session.id.clone(), native)))
                }
                _ => None,
            })
            .flatten()
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
        // Under the Plan preset every new prompt plans first; a prompt to an
        // agent awaiting input answers its plan instead.
        let answering = self.acp.get(&thread).is_some_and(|run| run.state == AgentState::AwaitingInput);
        let plan = plan || (self.preset(thread) == Preset::Plan && !answering);
        // One waiting to be signed in takes it once it is.
        let ready = self
            .acp
            .get(&thread)
            .is_some_and(|run| matches!(run.state, AgentState::Idle | AgentState::AwaitingInput) && run.sign_in.is_none());
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
        let me = self.own_user(thread);
        self.send_prompt(thread, &me, text, attachments).await?;
        Ok("sent")
    }

    /// Sends the prompt the agent was started with. Files it turned out not
    /// to take are left out and named, rather than losing the prompt.
    async fn send_first(&mut self, thread: ThreadId, first: First) -> Result<(), CoreError> {
        let accepts = self.running(thread)?.accepts;
        let (taken, refused): (Vec<_>, Vec<_>) = first.attachments.into_iter().partition(|attachment| accepts.takes(attachment));
        self.acp_prompt_with(&thread.to_string(), &first.prompt, first.plan, taken).await?;
        if !refused.is_empty() {
            let run = self.acp.get_mut(&thread).expect("checked above");
            let names: Vec<&str> = refused.iter().map(|attachment| attachment.name.as_str()).collect();
            run.error = Some(format!("{} doesn't take files like {}; sent without them", run.info.name, names.join(", ")));
        }
        Ok(())
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
        // Steering while the agent asks answers its question in your own words.
        let asked = std::mem::take(&mut run.questions);
        run.steering = Some(match asked.first() {
            Some(question) => format!("to your question \"{}\": {text}", question.message),
            None => text.to_owned(),
        });
        send(run, Command::Cancel);
        for question in asked {
            let said = own_answer(&question, text);
            self.record_acp(thread, &acp::tool_result(&question_call(question.id), "completed", Some(&said))).await?;
        }
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
            let me = self.own_user(thread);
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
        // What the agent said before the prompt, as when it went on by
        // itself, comes before it.
        self.flush_acp(thread).await?;
        self.own_turn_ended(thread).await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.stopped = false;
        run.hush = None;
        run.woke_by.clear();
        let attached = self.record_attachments(thread, &attachments).await?;
        let images: Vec<(String, String)> = attachments
            .iter()
            .filter(|attachment| attachment.mime.starts_with("image/"))
            .map(|attachment| (attachment.mime.clone(), BASE64.encode(&attachment.data)))
            .collect();
        self.record_acp(thread, &acp::prompt(&said, &authors.join(", "), &attached, &images)).await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.state = AgentState::Working;
        run.error = None;
        run.last_prompt = Some((prompt.clone(), attachments.clone()));
        send(run, Command::Prompt { text: prompt, attachments });
        self.agent_changed(thread).await;
        Ok(())
    }

    pub(crate) async fn acp_command(&mut self, selector: &str, command: Command) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        // Stopping an agent waiting to be signed in drops the sign-in with it.
        if command == Command::Stop
            && let Some(run) = self.acp.get_mut(&thread).filter(|run| run.sign_in.is_some())
        {
            run.sign_in = None;
            run.resend = None;
            if !run.state.running() {
                self.agent_changed(thread).await;
                return Ok("ok");
            }
        }
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
        if command == Command::Cancel {
            run.stopped = true;
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
            Some(run) => Some(run.shown()),
            None => self.agent_runs.get(&thread).and_then(|stored| AgentState::parse(&stored.state)),
        }
    }

    /// How many commands and subagents the thread's agent here left
    /// running.
    pub(crate) fn background_here(&self, thread: ThreadId) -> u32 {
        self.acp.get(&thread).map_or(0, |run| u32::try_from(run.in_background()).unwrap_or(u32::MAX))
    }

    /// Tells browsers how the thread's agent stands, and stores it when its
    /// state or error changed.
    pub(crate) async fn agent_changed(&mut self, thread: ThreadId) {
        self.announce_agent(thread);
        let Some(run) = self.acp.get_mut(&thread) else {
            return;
        };
        // The thread list shows what still runs in the background, which
        // doesn't change the agent's state.
        if run.background_listed != run.in_background() {
            run.background_listed = run.in_background();
            self.announce(crate::protocol::BusEvent::Threads);
        }
        let run = &self.acp[&thread];
        let stored = self.agent_runs.get(&thread);
        let record = silt_store::AgentRunRecord {
            agent: run.info.name.clone(),
            state: run.shown().as_str().to_owned(),
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
        let preset = self.capped_preset(thread, self.preset(thread));
        let run = self.acp.get_mut(&thread).expect("caller checked");
        if !run.presents_plans && plan_put(run, &permission.tool_call_id).is_some() {
            run.presents_plans = true;
            self.remember_seen(thread);
        }
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let kind = permission.tool_kind.clone().or_else(|| run.tool_kinds.get(&permission.tool_call_id).cloned());
        match crate::presets::decide(preset, kind.as_deref(), &permission.option_kinds) {
            Some(option) => send(run, Command::Answer { id: permission.id, option: Some(option.to_owned()) }),
            None => run.asking.push(permission),
        }
    }

    /// Records a question the agent asked as its tool call, and leaves it
    /// for the person.
    async fn question_asked(&mut self, thread: ThreadId, question: Question) -> Result<(), CoreError> {
        self.flush_acp(thread).await?;
        let input = question_input(&question);
        self.record_acp(thread, &acp::tool_call(&question_call(question.id), "AskUserQuestion", "other", Some(&input))).await?;
        self.acp.get_mut(&thread).expect("caller checked").questions.push(question);
        Ok(())
    }

    /// Answers the question `id` with each field's chosen values, or
    /// declines it, and records the answer as its tool's result.
    pub(crate) async fn acp_reply(
        &mut self,
        selector: &str,
        id: u64,
        answers: Option<Vec<(String, Vec<String>)>>,
    ) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        self.running(thread)?;
        let run = self.acp.get_mut(&thread).expect("checked above");
        let question = run
            .questions
            .iter()
            .find(|asked| asked.id == id)
            .ok_or_else(|| CoreError::Invalid("the agent isn't waiting for that answer".into()))?;
        let said = match &answers {
            Some(answers) => Some(answer_text(question, answers)?),
            None => None,
        };
        run.questions.retain(|asked| asked.id != id);
        send(run, Command::AnswerQuestion { id, answers });
        let result = said.unwrap_or_else(|| "The question was declined.".to_owned());
        self.record_acp(thread, &acp::tool_result(&question_call(id), "completed", Some(&result))).await?;
        self.agent_changed(thread).await;
        Ok("ok")
    }

    /// Closes the questions a turn left open, unanswered.
    async fn questions_dropped(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let dropped = std::mem::take(&mut self.acp.get_mut(&thread).expect("caller checked").questions);
        for question in dropped {
            let line = acp::tool_result(&question_call(question.id), "completed", Some("The question went unanswered."));
            self.record_acp(thread, &line).await?;
        }
        Ok(())
    }

    pub(crate) fn preset(&self, thread: ThreadId) -> Preset {
        self.agent_runs.get(&thread).and_then(|run| Preset::parse(&run.preset)).unwrap_or_default()
    }

    pub(crate) async fn set_preset(&mut self, thread: ThreadId, preset: Preset) -> Result<&'static str, CoreError> {
        self.check_thread_change(thread, |rules| rules.check_preset(preset))?;
        self.keep_choice(thread, |record| record.preset = preset.as_str().to_owned()).await?;
        if let Some(run) = self.acp.get_mut(&thread) {
            follow_preset_in_settings(run, preset);
        }
        // An agent with modes of its own is put in the one that does what the preset says, so the two never disagree.
        if let Some(run) = self.acp.get_mut(&thread)
            && let Some(mode) = run.modes.as_ref().and_then(|modes| mode_for_preset(preset, modes))
            && run.modes.as_ref().is_some_and(|modes| modes.current != mode)
        {
            // Into the plan mode, it comes back once the plan is approved, as a Plan first turn does.
            let left = run.modes.as_ref().map(|modes| modes.current.clone());
            run.back_to = if preset == Preset::Plan { left } else { None };
            send(run, Command::SetMode(mode.clone()));
            if let Some(modes) = &mut run.modes {
                modes.current = mode.clone();
            }
            if preset != Preset::Plan {
                self.keep_choice(thread, |record| record.mode = Some(mode)).await?;
            }
        }
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
        let meaning = mode_meaning(&run.info.id, mode, run.plan_mode() == Some(mode));
        self.check_thread_change(thread, |rules| rules.check_mode(mode, meaning))?;
        let run = self.acp.get_mut(&thread).expect("checked above");
        run.back_to = None;
        send(run, Command::SetMode(mode.to_owned()));
        let planning = run.plan_mode() == Some(mode);
        let follows = preset_following(mode, planning, self.preset(thread));
        let run = self.acp.get_mut(&thread).expect("checked above");
        if let Some(modes) = &mut run.modes {
            modes.current = mode.to_owned();
        }
        self.keep_choice(thread, |record| {
            record.mode = Some(mode.to_owned());
            record.preset = follows.as_str().to_owned();
        })
        .await?;
        self.remember_seen(thread);
        self.announce_agent(thread);
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
        let run = self.acp.get_mut(&thread).expect("checked above");
        if let Some(setting) = run.settings.iter_mut().find(|setting| setting.id == id) {
            setting.current = value.clone();
        }
        send(run, Command::SetSetting { id, value });
        self.remember_seen(thread);
        self.announce_agent(thread);
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
        let mut gone = Vec::new();
        for (thread, run) in &mut self.agent_runs {
            if AgentState::parse(&run.state).is_some_and(AgentState::running) {
                run.state = AgentState::Stopped.as_str().to_owned();
                self.store.put_agent_run(*thread, run).await?;
                gone.push(*thread);
            }
        }
        // Its subagents and background commands went with it, whether the
        // daemon stopped it or the machine went down.
        for thread in gone {
            if let Err(error) = self.stop_left_tasks(thread, None).await {
                warn!(%error, %thread, "could not record that the agent's tasks stopped");
            }
        }
        Ok(())
    }

    /// Records that the work the thread's agent left running stopped, as its
    /// agent did: in the session `only`, else in every session it ran here.
    async fn stop_left_tasks(&mut self, thread: ThreadId, only: Option<&str>) -> Result<(), CoreError> {
        let Some(family) = self.threads.get(&thread).map(|record| record.family) else {
            return Ok(());
        };
        let mut sessions: Vec<(String, Vec<&str>)> = Vec::new();
        for delta in self.local_log(family).iter().filter(|delta| delta.thread == thread) {
            let Op::Conv(ConvOp::Native { session, file: SessionFile::Main, line }) = &delta.op else {
                continue;
            };
            if session.agent != acp::AGENT || only.is_some_and(|only| only != session.id) {
                continue;
            }
            match sessions.iter_mut().find(|(id, _)| *id == session.id) {
                Some((_, lines)) => lines.push(line),
                None => sessions.push((session.id.clone(), vec![line.as_str()])),
            }
        }
        let stops: Vec<(String, Vec<String>)> = sessions
            .into_iter()
            .map(|(id, lines)| (id, acp::left_running(lines)))
            .filter(|(_, stops)| !stops.is_empty())
            .collect();
        for (session, lines) in stops {
            for line in lines {
                self.record_line(&session, "main", line, None).await?;
            }
        }
        Ok(())
    }

    /// Ends every agent running here, with what they left running, as the
    /// daemon exits: nothing would be left to hear them end.
    pub(crate) async fn end_agents(&mut self) {
        let threads: Vec<ThreadId> = self.acp.iter().filter(|(_, run)| run.handle.is_some()).map(|(thread, _)| *thread).collect();
        for thread in threads {
            let run = &self.acp[&thread];
            if let Some(handle) = run.handle.clone() {
                let _ = tokio::task::spawn_blocking(move || handle.end_now()).await;
            }
            if let Some(session) = run.session.clone()
                && let Err(error) = self.stop_left_tasks(thread, Some(&session)).await
            {
                warn!(%error, %thread, "could not record that the agent's tasks stopped");
            }
        }
    }

    /// Ends a command the thread's agent left running in the background,
    /// started by the call `call`: ACP has no way to ask the agent to, so
    /// Silt ends its processes, and the agent hears that it ended.
    pub(crate) async fn stop_task(&mut self, selector: &str, call: &str) -> Result<&'static str, CoreError> {
        let thread = self.find_thread(selector)?;
        let run = self.running(thread)?;
        let not_running = || CoreError::Invalid("that command isn't running in the background".into());
        let (task, kept) = run.background.iter().find(|(_, kept)| kept.call == call).ok_or_else(not_running)?;
        let (task, command) = (task.clone(), kept.command.clone());
        if command.trim().is_empty() {
            return Err(CoreError::Invalid("the agent didn't say what the command runs, so Silt can't find it".into()));
        }
        // The same command left running twice is told apart by age.
        let nth = run.background.values().filter(|other| other.started < kept.started && other.command == command).count();
        let handle = run.handle.clone().ok_or_else(not_running)?;
        let ended = tokio::task::spawn_blocking(move || handle.stop_command(&command, nth))
            .await
            .map_err(|error| CoreError::Invalid(format!("could not look for the command: {error}")))?
            .map_err(|error| CoreError::Invalid(format!("could not read this machine's processes: {error}")))?;
        if ended == 0 {
            return Err(CoreError::Invalid("none of the agent's processes runs that command; it may have just ended".into()));
        }
        let run = self.acp.get_mut(&thread).expect("checked above");
        run.background.remove(&task);
        run.stopped_tasks.insert(call.to_owned());
        self.flush_acp(thread).await?;
        self.record_acp(thread, &acp::task(call, Some(&task), Some("background"), "stopped")).await?;
        self.agent_changed(thread).await;
        Ok("stopped")
    }

    pub(crate) fn agent_status(&self, thread: ThreadId) -> Option<AgentStatus> {
        let run = self.acp.get(&thread)?;
        Some(AgentStatus {
            agent: run.info.name.clone(),
            agent_id: run.info.id.clone(),
            state: run.shown(),
            streaming: run.text.clone(),
            thinking: run.thought.clone(),
            permissions: run
                .asking
                .iter()
                .map(|asked| PermissionAsked {
                    id: asked.id,
                    title: asked.title.clone(),
                    options: asked.options.clone(),
                    plan: plan_put(run, &asked.tool_call_id).map(str::to_owned),
                    tool_call: Some(asked.tool_call_id.to_string()),
                })
                .collect(),
            questions: run.questions.iter().map(question_asked).collect(),
            sign_in: run.sign_in.as_ref().map(sign_in::SignIn::asked),
            context: run.context,
            error: run.error.clone(),
            modes: run.modes.as_ref().map(Into::into),
            settings: run.settings.iter().map(Into::into).collect(),
            takes_images: run.accepts.image,
            takes_files: run.accepts.embedded,
            commands: run.commands.iter().map(Into::into).collect(),
            stoppable: run.background.values().map(|task| task.call.clone()).collect(),
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
        if let AcpEvent::Login { token, event } = event {
            return self.login_event(thread, token, event).await;
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
            AcpEvent::Login { .. } => unreachable!("handled by acp_event"),
            AcpEvent::Agent(silt_acp::Event::Ready { session, loaded, modes, settings, accepts, reports_work, .. }) => {
                run.reports_work = reports_work;
                let preset = self.preset(thread);
                // A thread whose mode was never chosen starts in the one its preset says; Plan's is
                // entered by its first prompt, which knows to come back from it.
                let kept = self.agent_runs.get(&thread).and_then(|record| record.mode.clone()).or_else(|| {
                    modes.as_ref().filter(|_| preset != Preset::Plan).and_then(|modes| mode_for_preset(preset, modes))
                });
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.modes = modes;
                run.settings = settings;
                run.accepts = accepts;
                // What New thread chose, as far as this agent offers it now.
                for (id, value) in std::mem::take(&mut run.chosen_settings) {
                    let Some(setting) = run.settings.iter_mut().find(|setting| setting.id == id) else { continue };
                    if setting.current == value || !setting.choices.iter().any(|choice| choice.id == value) {
                        continue;
                    }
                    setting.current = value.clone();
                    send(run, Command::SetSetting { id, value });
                }
                let follows = kept.as_deref().and_then(preset_for_mode).unwrap_or(preset);
                follow_preset_in_settings(run, follows);
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
                // Signed in, by itself when asked to.
                if run.sign_in.as_ref().is_some_and(sign_in::SignIn::by_agent) {
                    run.sign_in = None;
                }
                let first = run.first.take();
                self.remember_seen(thread);
                self.acp_ready(thread, &session, loaded).await?;
                if self.resend(thread) {
                    return Ok(());
                }
                if let Some(first) = first {
                    self.send_first(thread, first).await?;
                }
                self.next_prompt(thread).await?;
            }
            AcpEvent::Agent(silt_acp::Event::ModeChanged(mode)) => {
                let planning = run.plan_mode() == Some(mode.as_str());
                if let Some(modes) = &mut run.modes {
                    modes.current = mode.clone();
                }
                // The preset follows the agent's mode, as when it leaves its plan mode by the
                // person's answer to its own plan (ADR 0044); only Silt's own switch into plan
                // mode for one Plan first turn, which it undoes, leaves the preset be.
                let temporary = planning && run.back_to.is_some();
                if !temporary {
                    run.back_to = None;
                    let follows = preset_following(&mode, planning, self.preset(thread));
                    follow_preset_in_settings(self.acp.get_mut(&thread).expect("caller checked"), follows);
                    if follows != self.preset(thread) {
                        self.keep_choice(thread, |record| {
                            record.preset = follows.as_str().to_owned();
                            record.mode = Some(mode);
                        })
                        .await?;
                    }
                    self.remember_seen(thread);
                }
            }
            AcpEvent::Agent(silt_acp::Event::Settings(settings)) => {
                run.settings = settings;
                self.remember_seen(thread);
            }
            AcpEvent::Agent(silt_acp::Event::Commands(commands)) => {
                run.commands = commands;
                self.remember_seen(thread);
            }
            AcpEvent::Agent(silt_acp::Event::Refused(why)) => run.error = Some(why),
            AcpEvent::Agent(silt_acp::Event::Working) => {
                // Working on a prompt of Silt's, it is already; else it took
                // something up by itself, and stops at its own turn's end.
                if matches!(run.state, AgentState::Idle | AgentState::AwaitingInput) {
                    self.agent_woke(thread).await?;
                    self.acp.get_mut(&thread).expect("caller checked").state = AgentState::Working;
                }
            }
            AcpEvent::Agent(silt_acp::Event::Update(update)) => self.acp_update(thread, update).await?,
            AcpEvent::Agent(silt_acp::Event::Permission(permission)) => self.permission_asked(thread, permission),
            AcpEvent::Agent(silt_acp::Event::Question(question)) => self.question_asked(thread, question).await?,
            AcpEvent::Agent(silt_acp::Event::SignIn(methods)) => self.sign_in_needed(thread, methods).await?,
            AcpEvent::Agent(silt_acp::Event::Context { used, size }) => run.context = Some(ContextUse { used, size }),
            AcpEvent::Agent(silt_acp::Event::TurnUsage { input, output }) => {
                // What the turn cost comes after what it said.
                self.flush_acp(thread).await?;
                let id = uuid::Uuid::now_v7().simple().to_string();
                self.record_acp(thread, &acp::usage(&id, input, output)).await?;
            }
            AcpEvent::Agent(silt_acp::Event::TurnEnded { reason }) => {
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::stop(reason.as_str())).await?;
                self.guide_written(thread, &reason).await?;
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.own_turn = false;
                // An agent that puts its plan to the person itself asks when it is
                // one; its turn may end to wait on its subagents, or to ask.
                let planned = std::mem::take(&mut run.planning) && reason == silt_acp::StopReason::EndTurn && !run.presents_plans;
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
                run.tool_said.clear();
                run.calls_running.clear();
                let signing_in = run.sign_in.is_some();
                self.questions_dropped(thread).await?;
                // The queue waits for the sign-in, as the agent would refuse it too.
                if !signing_in {
                    self.next_prompt(thread).await?;
                }
            }
            AcpEvent::Agent(silt_acp::Event::Failed(error)) => run.error = Some(crate::protocol::error_chain(&*error)),
            AcpEvent::Agent(silt_acp::Event::Exited) => {
                let run = self.acp.get_mut(&thread).expect("caller checked");
                run.background.clear();
                run.subagents.clear();
                run.state = AgentState::Stopped;
                run.handle = None;
                self.close_agent_session(thread).await?;
            }
        }
        Ok(())
    }

    /// Ends what the thread's agent left open, as its process is gone.
    async fn close_agent_session(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        self.flush_acp(thread).await?;
        self.own_turn_ended(thread).await?;
        // What it left running went with it.
        if let Some(session) = self.acp[&thread].session.clone() {
            self.stop_left_tasks(thread, Some(&session)).await?;
        }
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.asking.clear();
        let session = run.session.clone();
        self.questions_dropped(thread).await?;
        if let Some(id) = session {
            self.close_open_tool(&id).await?;
            if let Some(session) = self.session_mut(&id) {
                session.live = false;
                session.in_turn = false;
            }
        }
        Ok(())
    }

    /// The agent's session is up: a loaded one continues its recording, a
    /// new one starts its own, primed with the thread's conversation.
    async fn acp_ready(&mut self, thread: ThreadId, native: &str, loaded: bool) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.state = AgentState::Idle;
        // The registry's name, as the picker shows it; the agent's own is often its package's.
        let (checkout_id, checkout, agent_id, name) =
            (run.checkout_id, run.checkout.clone(), run.info.id.clone(), run.info.name.clone());
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
        // A chat continued here keeps its own primer when the thread has none.
        run.primer = primer.or(run.primer.take());
        self.record_acp(thread, &acp::session(&agent_id, &name, native)).await
    }

    async fn acp_update(&mut self, thread: ThreadId, update: Update) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.said_at = Some(Instant::now());
        let says = matches!(update, Update::AgentText(_) | Update::Thought(_) | Update::ToolCall { .. });
        if says && !run.reports_work && !run.own_turn && run.state == AgentState::Idle {
            // Working, so a prompt sent meanwhile waits for it rather than
            // being folded into what it does.
            run.own_turn = true;
            run.state = AgentState::Working;
            // An agent that can't say why it went on, as Copilot, goes first
            // for the output of the shell whose end set it off.
            let read = match &update {
                Update::ToolCall { input: Some(input), .. } => input.get("shellId").and_then(serde_json::Value::as_str),
                _ => None,
            };
            if let Some(task) = read.and_then(|shell| run.background.get(shell)) {
                let label = run.labels.get(&task.call).cloned();
                run.woke_by.push(ended(Some("background"), label.as_deref(), "ended"));
            }
            self.agent_woke(thread).await?;
        }
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
                let run = self.acp.get_mut(&thread).expect("caller checked");
                if run.thought.is_empty() {
                    run.thought_ms = now_ms();
                }
                run.thought.push_str(&text);
            }
            Update::UserText(_) => {}
            Update::ToolCall { id, title, kind, input } => {
                run.calls_running.insert(id.clone());
                run.labels.insert(id.clone(), label_of(&title, &kind, input.as_ref()));
                run.tool_kinds.insert(id.clone(), kind.clone());
                run.tool_said.insert(id.clone(), (title.clone(), input.clone()));
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::tool_call(&id, &title, &kind, input.as_ref())).await?;
            }
            Update::ToolUpdate { id, status, output, title, input, task } => {
                if let Some((said_title, said_input)) = run.tool_said.get(&id).cloned() {
                    let title = title.unwrap_or_else(|| said_title.clone());
                    let input = input.or_else(|| said_input.clone());
                    if (&title, &input) != (&said_title, &said_input) {
                        let kind = run.tool_kinds.get(&id).cloned().unwrap_or_default();
                        run.labels.insert(id.clone(), label_of(&title, &kind, input.as_ref()));
                        run.tool_said.insert(id.clone(), (title.clone(), input.clone()));
                        self.flush_acp(thread).await?;
                        self.record_acp(thread, &acp::tool_update(&id, &title, &kind, input.as_ref())).await?;
                    }
                }
                let run = self.acp.get_mut(&thread).expect("caller checked");
                if let Some(output) = &output {
                    let before = run.tool_output.insert(id.clone(), output.clone()).unwrap_or_default();
                    self.announce_tool_output(thread, &id, &before, output);
                }
                if let Some(status @ ("completed" | "failed")) = status.as_deref() {
                    let run = self.acp.get_mut(&thread).expect("caller checked");
                    run.tool_output.remove(&id);
                    run.calls_running.remove(&id);
                    self.flush_acp(thread).await?;
                    self.record_acp(thread, &acp::tool_result(&id, status, output.as_deref())).await?;
                }
                if let Some(news) = task {
                    let run = self.acp.get_mut(&thread).expect("caller checked");
                    if run.stopped_tasks.contains(&id) {
                        if news.status != "running" {
                            run.hush = Some(run.labels.get(&id).cloned().unwrap_or_default());
                        }
                        return Ok(());
                    }
                    if let (Some("background"), Some(task)) = (news.kind.as_deref(), &news.id) {
                        if news.status == "running" {
                            let command = run
                                .tool_said
                                .get(&id)
                                .and_then(|(_, input)| input.as_ref()?.get("command")?.as_str().map(str::to_owned))
                                .unwrap_or_default();
                            run.background.entry(task.clone()).or_insert_with(|| Background {
                                call: id.clone(),
                                started: Instant::now(),
                                command,
                            });
                        } else {
                            run.background.remove(task);
                        }
                    }
                    if news.kind.as_deref() == Some("subagent") {
                        if news.status == "running" {
                            run.subagents.insert(id.clone());
                        } else {
                            run.subagents.remove(&id);
                        }
                    }
                    // Between turns, what ended is what the agent will likely go on about.
                    if news.status != "running" && run.state == AgentState::Idle {
                        let label = run.labels.get(&id).cloned();
                        run.woke_by.push(ended(news.kind.as_deref(), label.as_deref(), &news.status));
                    }
                    self.flush_acp(thread).await?;
                    let record = acp::task(&id, news.id.as_deref(), news.kind.as_deref(), &news.status);
                    self.record_acp(thread, &record).await?;
                }
            }
            Update::Plan(steps) => {
                self.flush_acp(thread).await?;
                self.record_acp(thread, &acp::plan(&steps)).await?;
            }
            Update::Within { parent, update } => self.acp_within(thread, parent, *update).await?,
        }
        Ok(())
    }

    /// The agent went on by itself, without a prompt: what it said before
    /// is the turn before's, and a turn of its own opens (ADR 0050).
    async fn agent_woke(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        self.flush_acp(thread).await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let mut cause = (!run.woke_by.is_empty()).then(|| std::mem::take(&mut run.woke_by).join("; "));
        // What ended since is what it goes on about; a command the person
        // stopped is, when nothing else ended.
        let hushed = run.hush.take().filter(|_| cause.is_none());
        if let Some(label) = &hushed {
            cause = Some(ended(Some("background"), Some(label).filter(|label| !label.is_empty()).map(String::as_str), "stopped"));
        }
        // Once stopped, nothing the agent left running starts it again
        // before the person does: ACP can't stop a background command, but
        // what the agent would do about it is stopped. An agent on v1, as
        // Copilot, ignores a cancel with no prompt to cancel, so it is
        // stopped as Stop agent does; the next prompt starts it again.
        let stopped = if run.stopped {
            Some(acp::Stopped::Agent)
        } else {
            // Stopping an agent on v1 would stop it for good, not the turn.
            hushed.filter(|_| run.reports_work).map(|_| acp::Stopped::Command)
        };
        if stopped.is_some() {
            send(run, if run.reports_work { Command::Cancel } else { Command::Stop });
        }
        self.record_acp(thread, &acp::wake(cause.as_deref(), stopped)).await
    }

    /// Ends the turn an agent that doesn't say when it stops opened by
    /// itself, before what comes after it.
    async fn own_turn_ended(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        if !std::mem::take(&mut run.own_turn) {
            return Ok(());
        }
        self.flush_acp(thread).await?;
        self.record_acp(thread, &acp::stop(silt_acp::StopReason::EndTurn.as_str())).await
    }

    /// Ends the turns agents that won't say when they stop opened by
    /// themselves, once they have gone quiet with no tool call running and
    /// nothing asked, and sends what was queued meanwhile.
    pub(crate) async fn end_quiet_own_turns(&mut self) {
        let quiet: Vec<ThreadId> = self
            .acp
            .iter()
            .filter(|(_, run)| {
                run.own_turn
                    && run.calls_running.is_empty()
                    && run.asking.is_empty()
                    && run.questions.is_empty()
                    && run.said_at.is_some_and(|said| said.elapsed() >= OWN_TURN_QUIET)
            })
            .map(|(thread, _)| *thread)
            .collect();
        for thread in quiet {
            if let Err(error) = self.own_turn_ended(thread).await {
                warn!(%error, "could not record the end of the agent's turn");
            }
            if let Some(run) = self.acp.get_mut(&thread).filter(|run| run.state == AgentState::Working) {
                run.state = AgentState::Idle;
            }
            if let Err(error) = self.next_prompt(thread).await {
                warn!(%error, "could not send the next prompt");
            }
            self.agent_changed(thread).await;
        }
    }

    /// Records what a subagent did, inside the call `parent` that started
    /// it: its messages, tool calls and their results, and the work it left
    /// running. Its thoughts and plan stay its own.
    async fn acp_within(&mut self, thread: ThreadId, parent: String, update: Update) -> Result<(), CoreError> {
        match update {
            Update::AgentText(text) => {
                let run = self.acp.get_mut(&thread).expect("caller checked");
                if let Some((call, said)) = &mut run.within
                    && *call == parent
                {
                    said.push_str(&text);
                    return Ok(());
                }
                self.flush_within(thread).await?;
                self.acp.get_mut(&thread).expect("caller checked").within = Some((parent, text));
            }
            Update::ToolCall { id, title, kind, input } => {
                self.flush_within(thread).await?;
                self.record_acp(thread, &acp::within(&acp::tool_call(&id, &title, &kind, input.as_ref()), &parent)).await?;
            }
            Update::ToolUpdate { id, status, output, title, input, task } => {
                self.flush_within(thread).await?;
                if let Some(title) = title {
                    let record = acp::tool_update(&id, &title, "", input.as_ref());
                    self.record_acp(thread, &acp::within(&record, &parent)).await?;
                }
                if let Some(status @ ("completed" | "failed")) = status.as_deref() {
                    let record = acp::tool_result(&id, status, output.as_deref());
                    self.record_acp(thread, &acp::within(&record, &parent)).await?;
                }
                if let Some(news) = task {
                    let record = acp::task(&id, news.id.as_deref(), news.kind.as_deref(), &news.status);
                    self.record_acp(thread, &acp::within(&record, &parent)).await?;
                }
            }
            Update::Thought(_) | Update::UserText(_) | Update::Plan(_) | Update::Within { .. } => {}
        }
        Ok(())
    }

    /// Records a subagent's message streamed so far.
    async fn flush_within(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        match run.within.take() {
            Some((call, said)) if !said.trim().is_empty() => {
                self.record_acp(thread, &acp::within(&acp::message(&said), &call)).await
            }
            _ => Ok(()),
        }
    }

    /// Records the message or thought streamed so far.
    async fn flush_acp(&mut self, thread: ThreadId) -> Result<(), CoreError> {
        self.flush_within(thread).await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let thought = std::mem::take(&mut run.thought);
        let thought_ms = run.thought_ms;
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
            self.record_acp(thread, &acp::thought(&thought, thought_ms)).await?;
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
            let by = match self.own_user(request.target) {
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
        let me = self.own_user(request.fork);
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
    let safe: String = base.chars().map(|c| if c.is_control() || c == ':' { '_' } else { c }).collect();
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

/// Where each agent's modes and settings are kept, as it last listed them.
const SEEN: &str = "agents-seen.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Seen {
    modes: Option<crate::protocol::AgentModes>,
    settings: Vec<crate::protocol::AgentSetting>,
    #[serde(default)]
    takes: Option<Takes>,
    /// When they were seen; an agent's offer older than `ASK_AGAIN_MS` is asked for again.
    #[serde(default)]
    seen_ms: i64,
    /// It put a plan to the person itself, as Claude's ExitPlanMode does.
    #[serde(default)]
    presents_plans: bool,
    #[serde(default)]
    commands: Vec<crate::protocol::AgentCommand>,
}

/// What an agent takes attached to a prompt, as kept beside its modes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Takes {
    images: bool,
    files: bool,
}

impl From<Accepts> for Takes {
    fn from(accepts: Accepts) -> Self {
        Self { images: accepts.image, files: accepts.embedded }
    }
}

impl From<Takes> for Accepts {
    fn from(takes: Takes) -> Self {
        Self { image: takes.images, embedded: takes.files }
    }
}

/// What an agent offers, as a session opened only to ask it said.
#[derive(Debug)]
pub(crate) struct Offered {
    modes: Option<silt_acp::Modes>,
    settings: Vec<silt_acp::Setting>,
    accepts: Accepts,
}

/// How long what an agent offers is taken as current: models and effort levels change with its releases.
const ASK_AGAIN_MS: i64 = 24 * 60 * 60 * 1000;

/// How long an agent asked what it offers may take to open its session.
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(90);

/// Where in a checkout a chat's files may be copied: a relative path of
/// plain names, as it may come from another person's browser.
fn copy_target(path: &str) -> Result<PathBuf, CoreError> {
    let path = Path::new(path.trim());
    let plain = path.components().all(|component| matches!(component, std::path::Component::Normal(_)));
    if path.as_os_str().is_empty() || !plain {
        return Err(CoreError::Invalid("copy the chat's files to a folder inside the checkout, such as `notes/chat`".into()));
    }
    Ok(path.to_owned())
}

/// Copies the files of a chat's folder, but not its git, into `to`. Links
/// are skipped, as they could point anywhere.
fn copy_chat_files(from: &Path, to: &Path) -> Result<(), CoreError> {
    if to.exists() && std::fs::read_dir(to)?.next().is_some() {
        return Err(CoreError::Invalid(format!("{} already has files; pick another folder for the chat's", to.display())));
    }
    fn copy(from: &Path, to: &Path, top: bool) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let target = to.join(entry.file_name());
            if top && entry.file_name() == ".git" {
                continue;
            }
            if kind.is_dir() {
                copy(&entry.path(), &target, false)?;
            } else if kind.is_file() {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }
    Ok(copy(from, to, true)?)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

/// A call as a person names it: the command it runs, or what its subagent
/// was asked to do.
fn label_of(title: &str, kind: &str, input: Option<&serde_json::Value>) -> String {
    let subject = silt_agents::tool::describe(title, Some(kind), input.unwrap_or(&serde_json::Value::Null)).subject;
    let label = subject.unwrap_or_else(|| title.to_owned());
    let line = label.lines().next().unwrap_or_default();
    if line.chars().count() > 80 { format!("{}…", line.chars().take(79).collect::<String>()) } else { line.to_owned() }
}

/// What ended, as the notice of the turn it set off says it.
fn ended(kind: Option<&str>, label: Option<&str>, status: &str) -> String {
    let how = match status {
        "completed" => "finished",
        "failed" => "failed",
        "ended" => "ended",
        _ => "was stopped",
    };
    match (kind, label) {
        (Some("subagent"), Some(label)) => format!("The subagent “{label}” {how}"),
        (Some("subagent"), None) => format!("A subagent {how}"),
        (_, Some(label)) => format!("`{label}` {how} in the background"),
        (_, None) => format!("A command in the background {how}"),
    }
}

/// Asks `info`'s agent what it offers: opens a session in a folder of the daemon's own, which nothing is
/// prompted in, takes the modes and settings it lists, and stops it.
fn probe_agent(events: mpsc::UnboundedSender<Event>, info: AgentInfo, data_dir: PathBuf) {
    tokio::spawn(async move {
        let offered = ask_agent(&info, &data_dir).await;
        let _ = events.send(Event::AgentProbed { agent: info.id, offered });
    });
}

async fn ask_agent(info: &AgentInfo, data_dir: &Path) -> Option<Offered> {
    let registry::AgentCommand { program, args, env } = registry::command(info, data_dir).await.ok()?;
    let cwd = data_dir.join("asking-agents");
    std::fs::create_dir_all(&cwd).ok()?;
    let start = Start {
        program,
        args,
        env: registry::agent_environment(&info.id, data_dir).into_iter().chain(env).collect(),
        cwd,
        mcp: None,
        load: None,
        sign_in_first: None,
    };
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let handle = silt_acp::start(start, sender);
    let offered = tokio::time::timeout(PROBE_WAIT, async {
        while let Some(event) = receiver.recv().await {
            match event {
                silt_acp::Event::Ready { modes, settings, accepts, .. } => return Some(Offered { modes, settings, accepts }),
                silt_acp::Event::Exited => return None,
                _ => {}
            }
        }
        None
    })
    .await
    .ok()
    .flatten();
    handle.send(Command::Stop);
    offered
}

/// Each agent's modes and settings as kept, by its registry id; none where
/// the file is missing or unreadable, which only means none were seen.
/// The plan the call `tool_call` puts to the person, as Claude's
/// ExitPlanMode does in its input.
fn plan_put<'run>(run: &'run AcpRun, tool_call: &str) -> Option<&'run str> {
    run.tool_said.get(tool_call).and_then(|(_, input)| input.as_ref()?.get("plan")?.as_str())
}

fn read_seen(data_dir: &Path) -> HashMap<String, Seen> {
    std::fs::read(data_dir.join(SEEN)).ok().and_then(|json| serde_json::from_slice(&json).ok()).unwrap_or_default()
}

/// A mode's id as Silt knows it: Copilot's are links to the protocol's
/// page on modes, ending in `#agent`, `#plan` and `#autopilot`.
/// What a mode means for the workspace's cap on what an agent does without
/// asking (ADR 0065): Copilot's say how it works, except Autopilot (ADR 0049).
pub(crate) fn mode_meaning(agent: &str, mode: &str, planning: bool) -> silt_workspace::ModeMeaning {
    use silt_workspace::ModeMeaning;
    if planning || mode_key(mode) == "plan" {
        return ModeMeaning::Preset(Preset::Plan);
    }
    match preset_for_mode(mode) {
        Some(preset) => ModeMeaning::Preset(preset),
        None if silt_workspace::agent_key(agent).as_str() == "copilot" => ModeMeaning::NotPermissions,
        None => ModeMeaning::Unknown,
    }
}

fn mode_key(mode: &str) -> &str {
    mode.rsplit_once('#').map_or(mode, |(_, key)| key)
}

/// The preset that lets an agent do what its mode `mode` does, by the ids
/// Claude's, Codex's and Copilot's adapters use.
fn preset_for_mode(mode: &str) -> Option<Preset> {
    match mode_key(mode) {
        "default" | "read-only" => Some(Preset::AskForEverything),
        "acceptEdits" => Some(Preset::AskBeforeCommands),
        "bypassPermissions" | "auto" | "dontAsk" | "full-access" | "autopilot" => Some(Preset::FullAccess),
        _ => None,
    }
}

/// The preset to go on with once the agent is in `mode`: Plan in its plan mode, the
/// mode's own where Silt knows it, and otherwise the one already chosen, unless that
/// was Plan, which the agent has left.
fn preset_following(mode: &str, planning: bool, chosen: Preset) -> Preset {
    if planning {
        return Preset::Plan;
    }
    preset_for_mode(mode).unwrap_or(if chosen == Preset::Plan { Preset::AskBeforeCommands } else { chosen })
}

/// The agent's mode that does what `preset` says, among those it offers.
/// Copilot's `agent` asks as its permissions setting says, which follows
/// the preset; Full access never moves it into `autopilot`, which also
/// runs on without the person.
fn mode_for_preset(preset: Preset, modes: &silt_acp::Modes) -> Option<String> {
    let wanted: &[&str] = match preset {
        Preset::Plan => return silt_acp::plan_mode(modes).map(|mode| mode.id.clone()),
        Preset::AskForEverything => &["default", "read-only", "agent"],
        Preset::AskBeforeCommands => &["acceptEdits", "agent"],
        Preset::FullAccess => &["bypassPermissions", "full-access", "dontAsk"],
    };
    wanted.iter().find_map(|id| modes.available.iter().find(|mode| mode_key(&mode.id) == *id)).map(|mode| mode.id.clone())
}

/// The value of an agent's own on/off permissions setting, such as
/// Copilot's Allow All, that does what `preset` says: on for Full access,
/// off otherwise, so the agent asks and the preset answers. None for a
/// setting of another kind, which the composer offers as it is.
fn permission_value(preset: Preset, setting: &silt_acp::Setting) -> Option<String> {
    if setting.category.as_deref() != Some("permissions") {
        return None;
    }
    let pick =
        |id: &str| setting.choices.iter().find(|choice| choice.id.eq_ignore_ascii_case(id)).map(|choice| choice.id.clone());
    let (on, off) = [("on", "off"), ("true", "false"), ("enabled", "disabled")]
        .into_iter()
        .find_map(|(on, off)| Some((pick(on)?, pick(off)?)))?;
    Some(if preset == Preset::FullAccess { on } else { off })
}

/// Puts the agent's own permissions settings where the preset says.
fn follow_preset_in_settings(run: &mut AcpRun, preset: Preset) {
    let changes: Vec<(String, String)> = run
        .settings
        .iter_mut()
        .filter_map(|setting| {
            let value = permission_value(preset, setting).filter(|value| *value != setting.current)?;
            setting.current = value.clone();
            Some((setting.id.clone(), value))
        })
        .collect();
    for (id, value) in changes {
        send(run, Command::SetSetting { id, value });
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
        let workspace_env = std::mem::take(&mut start.env);
        start.env = registry::agent_environment(&info.id, &data_dir).into_iter().chain(env).chain(workspace_env).collect();
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
    fn the_preset_follows_the_mode_the_agent_is_in() {
        assert_eq!(preset_following("plan", true, Preset::AskForEverything), Preset::Plan);
        assert_eq!(preset_following("acceptEdits", false, Preset::Plan), Preset::AskBeforeCommands);
        assert_eq!(preset_following("default", false, Preset::Plan), Preset::AskForEverything);
        assert_eq!(preset_following("bypassPermissions", false, Preset::AskForEverything), Preset::FullAccess);
        assert_eq!(
            preset_following("code", false, Preset::AskForEverything),
            Preset::AskForEverything,
            "a mode Silt doesn't know keeps the choice"
        );
        assert_eq!(
            preset_following("code", false, Preset::Plan),
            Preset::AskBeforeCommands,
            "but leaves Plan, which the agent left"
        );
    }

    #[test]
    fn copilot_s_modes_and_allow_all_follow_the_preset() {
        let modes = silt_acp::Modes {
            current: "https://agentclientprotocol.com/protocol/session-modes#autopilot".into(),
            available: ["agent", "plan", "autopilot"]
                .map(|key| silt_acp::Choice {
                    id: format!("https://agentclientprotocol.com/protocol/session-modes#{key}"),
                    name: key.into(),
                    description: None,
                })
                .into(),
        };
        let agent = "https://agentclientprotocol.com/protocol/session-modes#agent";
        assert_eq!(preset_following(&modes.current, false, Preset::AskBeforeCommands), Preset::FullAccess);
        assert_eq!(mode_for_preset(Preset::AskBeforeCommands, &modes).as_deref(), Some(agent), "out of autopilot");
        assert_eq!(mode_for_preset(Preset::FullAccess, &modes), None, "never into autopilot");
        assert!(mode_for_preset(Preset::Plan, &modes).is_some_and(|mode| mode.ends_with("#plan")));

        let choice = |id: &str| silt_acp::Choice { id: id.into(), name: id.into(), description: None };
        let allow_all = silt_acp::Setting {
            id: "allow_all".into(),
            name: "Allow All".into(),
            category: Some("permissions".into()),
            current: "on".into(),
            choices: vec![choice("on"), choice("off")],
        };
        assert_eq!(permission_value(Preset::FullAccess, &allow_all).as_deref(), Some("on"));
        assert_eq!(permission_value(Preset::AskBeforeCommands, &allow_all).as_deref(), Some("off"));
        let model = silt_acp::Setting { category: Some("model".into()), ..allow_all };
        assert_eq!(permission_value(Preset::FullAccess, &model), None);
    }

    #[test]
    fn a_question_is_answered_in_its_choices_words() {
        let question = Question {
            id: 1,
            message: "What should it count?".into(),
            fields: vec![silt_acp::QuestionField {
                key: "count".into(),
                title: "Progress".into(),
                description: None,
                options: vec![
                    silt_acp::QuestionChoice { value: "acked".into(), label: "Bytes acknowledged".into(), description: None },
                    silt_acp::QuestionChoice {
                        value: "sent".into(),
                        label: "Bytes sent".into(),
                        description: Some("Smooth".into()),
                    },
                ],
                multi: false,
                required: true,
            }],
        };
        let said = answer_text(&question, &[("count".into(), vec!["acked".into()])]).unwrap();
        let entry = silt_agents::Entry::ToolResult { tool_use_id: "q".into(), text: said, is_error: false };
        let shown = silt_agents::tool::shown(entry).unwrap();
        assert_eq!(shown.outcome.unwrap().answers[0].answer, "Bytes acknowledged");
        assert!(answer_text(&question, &[("count".into(), vec!["nope".into()])]).is_err());
        assert!(answer_text(&question, &[("other".into(), vec!["acked".into()])]).is_err());
        assert_eq!(question_input(&question)["questions"][0]["options"][1]["description"], "Smooth");
        let entry =
            silt_agents::Entry::ToolResult { tool_use_id: "q".into(), text: own_answer(&question, " Packets "), is_error: false };
        assert_eq!(silt_agents::tool::shown(entry).unwrap().outcome.unwrap().answers[0].answer, "Packets");
    }

    /// Claude's adapter follows each question with an optional box for an
    /// answer of one's own: it reads back as part of that question's answer.
    #[test]
    fn a_box_of_ones_own_answers_the_question_before_it() {
        let choice = |label: &str| silt_acp::QuestionChoice { value: label.into(), label: label.into(), description: None };
        let field = |key: &str, description: &str, options: Vec<silt_acp::QuestionChoice>| silt_acp::QuestionField {
            key: key.into(),
            title: key.into(),
            description: Some(description.into()),
            options,
            multi: false,
            required: false,
        };
        let question = Question {
            id: 1,
            message: "Please answer the following questions.".into(),
            fields: vec![
                field("question_0", "What should it count?", vec![choice("Bytes"), choice("Chunks")]),
                field("question_0_custom", "Type your own answer (optional).", vec![]),
                field("question_1", "Where should it show?", vec![choice("Header"), choice("Footer")]),
                field("question_1_custom", "Type your own answer (optional).", vec![]),
            ],
        };
        let asked = question_input(&question);
        let headers: Vec<&str> =
            asked["questions"].as_array().unwrap().iter().map(|each| each["header"].as_str().unwrap()).collect();
        assert_eq!(headers, ["question_0", "question_1"]);

        let said = answer_text(
            &question,
            &[
                ("question_1".into(), vec!["Footer".into()]),
                ("question_0".into(), vec!["Chunks".into()]),
                ("question_0_custom".into(), vec!["every 64 KiB".into()]),
            ],
        )
        .unwrap();
        let entry = silt_agents::Entry::ToolResult { tool_use_id: "q".into(), text: said, is_error: false };
        let answers = silt_agents::tool::shown(entry).unwrap().outcome.unwrap().answers;
        let answers: Vec<(&str, &str)> = answers.iter().map(|each| (each.question.as_str(), each.answer.as_str())).collect();
        assert_eq!(answers, [("What should it count?", "Chunks, every 64 KiB"), ("Where should it show?", "Footer")]);
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
        assert_eq!(attachment_name(hash, "C:notes.txt"), "prompt/0123456789ab/C_notes.txt");
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

    #[tokio::test]
    async fn files_for_the_first_prompt_are_refused_before_the_agent_starts() {
        let data = tempfile::tempdir().expect("temp dir");
        std::fs::write(data.path().join("agents.json"), r#"[{"id": "plain", "name": "Plain agent", "command": "plain-agent"}]"#)
            .expect("writes");
        let store = Store::open(data.path()).await.expect("opens");
        let (events, _inbox) = mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        let shot = PromptAttachment { name: "shot.png".into(), mime: "image/png".into(), data: BASE64.encode([0x89, 0x50]) };
        let start = |prompt: Option<&str>| StartOptions {
            prompt: prompt.map(str::to_owned),
            attachments: vec![shot.clone()],
            ..StartOptions::default()
        };
        let refused = |result: Result<String, CoreError>| result.expect_err("refused").to_string();

        let alone = refused(core.acp_start(None, Some(data.path()), "plain", start(None)).await);
        assert!(alone.contains("attached files go with the first prompt"), "{alone}");

        let takes_no_images = Seen { takes: Some(Takes { images: false, files: false }), ..Seen::default() };
        core.keep_seen("plain".into(), takes_no_images);
        let image = refused(core.acp_start(None, Some(data.path()), "plain", start(Some("look"))).await);
        assert!(image.contains("Plain agent doesn't take files like shot.png"), "{image}");
        assert!(core.acp.is_empty(), "nothing was started");
    }
}
