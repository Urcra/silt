//! Auto's check for an agent without an auto mode of its own (ADR 0129):
//! the judge's session, what its verdict does, refusals handed back to the
//! agent, and the person's answers to `silt_escalate`.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use silt_access::judge::{Call, Context, Verdict};
use silt_acp::registry::{self, AgentInfo};
use silt_acp::{Command, Permission, Start, Update};
use silt_types::ThreadId;
use silt_workspace::{Judge, Preset};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use crate::acp::{Aborting, AcpSandbox, send};
use crate::core::{Core, CoreError, Event};
use crate::protocol::{AutoCheck, EscalationAsked, Response};

/// How long a check may take before its call goes to the person.
pub(crate) const JUDGE_WAIT: Duration = Duration::from_secs(60);
/// How long `silt_escalate` waits on the person, below MCP clients' timeouts.
const ESCALATE_WAIT: Duration = Duration::from_secs(50);
/// How long a judge's processes get to end once it answered.
const JUDGE_ENDING: Duration = Duration::from_secs(5);
/// Refusals kept per run, for the agent to escalate.
const REFUSALS_KEPT: usize = 20;
const JUDGES_FILE: &str = "judges.json";
/// Who a note from Silt itself is by.
pub(crate) const SILT: &str = "Silt";

/// What Auto's check holds for one run of a thread's agent.
#[derive(Default)]
pub(crate) struct AutoState {
    /// Calls the judge is checking, by the permission request each answers.
    judging: HashMap<u64, Judging>,
    /// Why the check couldn't judge a request left to the person, by its id.
    pub(crate) unchecked: HashMap<u64, String>,
    /// Calls the judge allowed, by their hash: the same call goes through
    /// again while the thread stays on Auto.
    allowed: HashSet<String>,
    /// Calls the judge refused, newest last, for the agent to escalate.
    refused: VecDeque<Refused>,
    /// Calls the person allowed through an escalation, each once.
    granted: HashSet<String>,
    /// The person's last prompts, which the judge reads the call against.
    prompts: VecDeque<String>,
    /// Notes Silt steered into the agent's turn, so they read as Silt's.
    pub(crate) silt_steers: Vec<String>,
    /// Notes for the agent that wait for its turn to end.
    pub(crate) hand_back: Vec<String>,
    escalations: Vec<Escalation>,
}

struct Judging {
    permission: Permission,
    call: CallSaid,
    judge: String,
    started: Instant,
}

/// A call as the agent announced it.
#[derive(Debug, Clone)]
pub(crate) struct CallSaid {
    pub(crate) id: String,
    pub(crate) kind: Option<String>,
    pub(crate) title: String,
    pub(crate) input: Option<serde_json::Value>,
    pub(crate) cwd: String,
}

impl CallSaid {
    fn call(&self) -> Call<'_> {
        Call { kind: self.kind.as_deref(), title: &self.title, input: self.input.as_ref(), cwd: &self.cwd }
    }
}

struct Refused {
    hash: String,
    shown: String,
    reason: String,
}

struct Escalation {
    id: u64,
    hash: String,
    shown: String,
    refused: String,
    why: String,
    /// The agent's `silt_escalate` call, while it still waits.
    reply: Option<oneshot::Sender<Response>>,
}

impl AutoState {
    /// What the thread's preset no longer allows once it leaves Auto.
    pub(crate) fn left_auto(&mut self) {
        self.allowed.clear();
        self.granted.clear();
    }

    /// Keeps a prompt of the person's, for the judge to read calls against.
    pub(crate) fn prompted(&mut self, prompt: &str) {
        self.prompts.push_back(prompt.to_owned());
        while self.prompts.len() > silt_access::judge::PROMPTS {
            self.prompts.pop_front();
        }
    }

    /// Whether an escalation waits on the person.
    pub(crate) fn waits_on_person(&self) -> bool {
        !self.escalations.is_empty()
    }

    /// The calls being checked, by their tool call ids.
    pub(crate) fn checking(&self) -> Vec<String> {
        self.judging.values().map(|judging| judging.call.id.clone()).collect()
    }

    pub(crate) fn escalations(&self) -> Vec<EscalationAsked> {
        self.escalations
            .iter()
            .map(|asked| EscalationAsked {
                id: asked.id,
                call: asked.shown.clone(),
                refused: asked.refused.clone(),
                why: asked.why.clone(),
            })
            .collect()
    }

    /// What a turn's end leaves moot: checks of requests it took along.
    pub(crate) fn turn_ended(&mut self) {
        self.judging.clear();
        self.unchecked.clear();
    }
}

/// The judges threads chose, kept across restarts.
#[derive(Debug, Default)]
pub(crate) struct Judges(BTreeMap<String, Judge>);

impl Judges {
    pub(crate) fn load(data_dir: &Path) -> Self {
        let read = std::fs::read(data_dir.join(JUDGES_FILE)).ok().and_then(|json| serde_json::from_slice(&json).ok());
        Self(read.unwrap_or_default())
    }

    fn save(&self, data_dir: &Path) -> Result<(), CoreError> {
        let json = serde_json::to_vec_pretty(&self.0).map_err(|error| CoreError::Invalid(error.to_string()))?;
        crate::design::write_atomically(&data_dir.join(JUDGES_FILE), &json)?;
        Ok(())
    }

    fn of(&self, thread: ThreadId) -> Option<&Judge> {
        self.0.get(&thread.to_string())
    }
}

/// What a judge session answered.
#[derive(Debug)]
pub(crate) struct Judged {
    pub(crate) thread: ThreadId,
    pub(crate) run: u64,
    pub(crate) permission: u64,
    pub(crate) outcome: Result<Verdict, String>,
}

/// A judge to start: its agent, the model asked of it and the session's
/// place, sandboxed as the thread's agent is.
struct JudgeStart {
    info: AgentInfo,
    model: Option<String>,
    start: Start,
    sandbox: Option<AcpSandbox>,
    prompt: String,
}

impl Core {
    /// Takes a request Auto leaves to Silt's check: a call the person
    /// allowed once, or the judge allowed before, goes through; any other is
    /// put to the judge. Answers the request back when no judge can start.
    pub(crate) async fn auto_check(&mut self, thread: ThreadId, permission: Permission, said: CallSaid) -> Option<Permission> {
        let call = said.call();
        let hash = call.hash();
        let run = self.acp.get_mut(&thread)?;
        let early = if run.auto.granted.remove(&hash) {
            Some(("granted", "You allowed it when the agent asked.".to_owned()))
        } else if run.auto.allowed.contains(&hash) {
            Some(("before", "Auto allowed the same call earlier in this thread.".to_owned()))
        } else {
            None
        };
        if let Some((verdict, reason)) = early {
            answer(run, &permission, true);
            if let Err(error) = self.record_verdict(thread, &said.id, verdict, &reason, None, None).await {
                warn!(%thread, "recording a verdict failed: {error}");
            }
            return None;
        }
        let (judge, name) = match self.judge_for(thread).await {
            Ok(judge) => judge,
            Err(error) => {
                warn!(%thread, "no judge could start: {error}");
                let run = self.acp.get_mut(&thread)?;
                run.auto.unchecked.insert(permission.id, format!("Auto couldn't start its check: {error}"));
                return Some(permission);
            }
        };
        let run = self.acp.get_mut(&thread)?;
        let prompts: Vec<String> = run.auto.prompts.iter().cloned().collect();
        let cap = self.thread_cap(thread);
        let run = self.acp.get_mut(&thread)?;
        let sandboxed = run.sandbox.is_some().then_some("Silt's sandbox: the thread's checkout and toolchains, no credentials");
        let context = Context { cap: cap.map(Preset::label), sandbox: sandboxed, prompts: &prompts };
        let prompt = silt_access::judge::prompt(&call, &context);
        let (events, data_dir, number) = (self.events.clone(), self.data_dir.clone(), run.run_number());
        let start = JudgeStart { prompt, ..judge };
        run.auto
            .judging
            .insert(permission.id, Judging { permission: permission.clone(), call: said, judge: name, started: Instant::now() });
        spawn_judge(events, thread, number, permission.id, data_dir, start);
        None
    }

    /// The judge for `thread`'s next check, and its name: the one the rules,
    /// the thread or its workspace choose, else the thread's own agent where
    /// that one isn't allowed or isn't here.
    async fn judge_for(&mut self, thread: ThreadId) -> Result<(JudgeStart, String), CoreError> {
        let workspace = self.start_workspace(Some(&thread.to_string()), None, None)?;
        let chosen = self.judges.of(thread).cloned();
        let judge = self.rules(&workspace).judge(chosen.as_ref(), workspace.defaults.auto_judge.as_ref());
        let run = self.acp.get(&thread).ok_or_else(|| CoreError::Invalid("the agent isn't running".into()))?;
        let own = (run.info.clone(), run.model().map(|(id, _)| id));
        let (agent, model) = judge.agent_for(&own.0.id, own.1.as_deref());
        let (agent, model) = (agent.to_owned(), model.map(str::to_owned));
        let other = std::iter::once(registry::local_agent(&crate::acp::silt_exe()))
            .chain(registry::all(&self.data_dir))
            .find(|candidate| candidate.id == agent)
            .filter(|info| self.judge_allowed(&workspace, info));
        let (info, model) = match other {
            Some(info) => (info, model),
            None => {
                if agent != own.0.id {
                    warn!(%thread, %agent, "the judge chosen isn't here or allowed; the thread's own agent judges");
                }
                own
            }
        };
        let name = self.judge_name(&info, model.as_deref());
        let run = self.acp.get(&thread).expect("checked above");
        let mut start = Start {
            program: String::new(),
            args: Vec::new(),
            env: workspace.env.iter().map(|(name, value)| (name.clone(), value.clone())).collect(),
            cwd: run.checkout.clone(),
            mcp: None,
            load: None,
            sign_in_first: None,
        };
        let token = run.token.clone();
        let sandbox = run.sandbox.as_ref().map(|(reach, token, workspace)| AcpSandbox {
            reach: crate::folders::widened(reach.clone(), &run.folders),
            token: token.clone(),
            scratch: judge_scratch(&self.data_dir, thread),
            workspace: workspace.clone(),
            thread: thread.to_string(),
            relayed: Vec::new(),
        });
        if info.id == registry::LOCAL_AGENT {
            start.env.extend(self.local_env(&token).await?);
        }
        Ok((JudgeStart { info, model, start, sandbox, prompt: String::new() }, name))
    }

    /// Whether the workspace lets `info`'s agent run, and so judge.
    fn judge_allowed(&self, workspace: &silt_workspace::Workspace, info: &AgentInfo) -> bool {
        let custom = registry::custom(&self.data_dir).iter().any(|own| own.id == info.id);
        let choice = if custom {
            silt_workspace::AgentChoice::Custom { name: info.name.clone() }
        } else {
            silt_workspace::AgentChoice::Known { key: silt_workspace::agent_key(&info.id), name: info.name.clone() }
        };
        let support = if self.effective_policy(workspace).silt_sandbox {
            silt_workspace::SandboxSupport::Isolate
        } else {
            silt_workspace::SandboxSupport::None
        };
        self.rules(workspace).check_agent(&choice, support).is_ok()
    }

    /// The judge as the person knows it: its agent, on its model.
    fn judge_name(&self, info: &AgentInfo, model: Option<&str>) -> String {
        match model {
            Some(model) => {
                let shown = crate::acp::seen_model_name(&self.data_dir, &info.id, model).unwrap_or_else(|| model.to_owned());
                format!("{} on {shown}", info.name)
            }
            None => info.name.clone(),
        }
    }

    /// The most the thread's workspace lets an agent do without asking.
    fn thread_cap(&self, thread: ThreadId) -> Option<Preset> {
        let family = self.threads.get(&thread)?.family;
        self.effective_policy(&self.family_workspace(family)).max_preset
    }

    /// Acts on a judge's verdict: the call goes through, or is refused and
    /// the agent told why; with none, it's the person's to answer.
    pub(crate) async fn judged(&mut self, judged: Judged) {
        let Judged { thread, run, permission, outcome } = judged;
        let Some(current) = self.acp.get_mut(&thread).filter(|current| current.run_number() == run) else {
            return;
        };
        let Some(judging) = current.auto.judging.remove(&permission) else {
            return;
        };
        let ms = i64::try_from(judging.started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let shown = judging.call.call().shown();
        let hash = judging.call.call().hash();
        let (verdict, reason) = match outcome {
            Ok(Verdict::Allow { reason }) => {
                answer(current, &judging.permission, true);
                current.auto.allowed.insert(hash);
                ("allow", reason)
            }
            Ok(Verdict::Refuse { reason }) => {
                answer(current, &judging.permission, false);
                current.auto.refused.push_back(Refused { hash, shown: shown.clone(), reason: reason.clone() });
                while current.auto.refused.len() > REFUSALS_KEPT {
                    current.auto.refused.pop_front();
                }
                let note = silt_access::judge::refused_note(&shown, &reason);
                self.tell_agent(thread, note).await;
                ("refuse", reason)
            }
            Err(why) => {
                let reason = format!("Auto couldn't check this: {why}. It's yours to answer.");
                current.auto.unchecked.insert(permission, reason.clone());
                current.asking.push(judging.permission);
                ("unchecked", reason)
            }
        };
        if let Err(error) = self.record_verdict(thread, &judging.call.id, verdict, &reason, Some(&judging.judge), Some(ms)).await
        {
            warn!(%thread, "recording a verdict failed: {error}");
        }
        self.agent_changed(thread).await;
    }

    async fn record_verdict(
        &mut self,
        thread: ThreadId,
        call: &str,
        verdict: &str,
        reason: &str,
        judge: Option<&str>,
        ms: Option<i64>,
    ) -> Result<(), CoreError> {
        self.record_acp(thread, &silt_agents::acp::auto(call, verdict, reason, judge, ms)).await
    }

    /// Tells the agent `note` in its turn where it takes steers, else once
    /// its turn ends, as a message from Silt.
    pub(crate) async fn tell_agent(&mut self, thread: ThreadId, note: String) {
        let Some(run) = self.acp.get_mut(&thread) else {
            return;
        };
        if run.steers && run.working() {
            run.auto.silt_steers.push(note.clone());
            send(run, Command::Steer(note));
            return;
        }
        run.auto.hand_back.push(note);
        if !run.working()
            && let Err(error) = self.hand_back(thread).await
        {
            warn!(%thread, "handing a note to the agent failed: {error}");
        }
    }

    /// Sends the notes waiting for the agent's turn to end, as one message
    /// from Silt; answers whether there were any.
    pub(crate) async fn hand_back(&mut self, thread: ThreadId) -> Result<bool, CoreError> {
        let Some(run) = self.acp.get_mut(&thread).filter(|run| !run.auto.hand_back.is_empty()) else {
            return Ok(false);
        };
        let notes = std::mem::take(&mut run.auto.hand_back).join("\n\n");
        self.send_prompt(thread, SILT, &notes, crate::acp::Given::default()).await?;
        Ok(true)
    }

    /// `thread`'s agent asks the person to allow a call Auto refused, named
    /// as the refusal named it. `reply` gets their answer, or word that it
    /// comes later once the wait runs out.
    pub(crate) async fn escalate(&mut self, thread: ThreadId, call: &str, why: &str, reply: oneshot::Sender<Response>) {
        let Some(run) = self.acp.get_mut(&thread) else {
            let _ = reply.send(Response::error("only an agent Silt runs asks the person this way"));
            return;
        };
        let wanted = call.trim().trim_matches('`');
        let Some(refused) = run.auto.refused.iter().rev().find(|refused| refused.shown == wanted) else {
            let named: Vec<String> = run.auto.refused.iter().map(|refused| format!("`{}`", refused.shown)).collect();
            let why = if named.is_empty() {
                "Auto hasn't refused any call of yours in this run".to_owned()
            } else {
                format!("name a call Auto refused exactly as it was refused: {}", named.join(", "))
            };
            let _ = reply.send(Response::error(why));
            return;
        };
        let why = why.trim();
        if why.is_empty() {
            let _ = reply.send(Response::error("say why the call is needed"));
            return;
        }
        self.next_token += 1;
        let id = self.next_token;
        let escalation = Escalation {
            id,
            hash: refused.hash.clone(),
            shown: refused.shown.clone(),
            refused: refused.reason.clone(),
            why: why.to_owned(),
            reply: Some(reply),
        };
        run.auto.escalations.push(escalation);
        let events = self.events.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ESCALATE_WAIT).await;
            let _ = events.send(Event::EscalationWaited { thread, id });
        });
        self.agent_changed(thread).await;
    }

    /// The wait on the person ran out: the agent hears their answer later.
    pub(crate) fn escalation_waited(&mut self, thread: ThreadId, id: u64) {
        let Some(asked) = self.acp.get_mut(&thread).and_then(|run| run.auto.escalations.iter_mut().find(|asked| asked.id == id))
        else {
            return;
        };
        if let Some(reply) = asked.reply.take() {
            let waiting = "The person hasn't answered yet. Their answer comes to you as a message from Silt once they do; \
                           meanwhile go on with what doesn't need the call, or end your turn.";
            let _ = reply.send(Response::ok(waiting));
        }
    }

    /// The person's answer to an escalation: allowed, the call goes through
    /// once, without the check; refused, the agent hears why.
    pub(crate) async fn escalation_answer(
        &mut self,
        thread: ThreadId,
        id: u64,
        allow: bool,
        words: Option<&str>,
    ) -> Result<&'static str, CoreError> {
        let run = self.acp.get_mut(&thread).ok_or_else(|| CoreError::Invalid("the agent isn't running".into()))?;
        let at = run
            .auto
            .escalations
            .iter()
            .position(|asked| asked.id == id)
            .ok_or_else(|| CoreError::Invalid("the agent isn't waiting for that answer".into()))?;
        let mut asked = run.auto.escalations.remove(at);
        let words = words.map(str::trim).filter(|words| !words.is_empty());
        let said = words.map(|words| format!(" They said: {words}")).unwrap_or_default();
        let text = if allow {
            run.auto.granted.insert(asked.hash.clone());
            format!(
                "The person allowed `{}` once.{said} Run it again now; Silt lets it through without Auto's check.",
                asked.shown
            )
        } else {
            format!(
                "The person refused `{}`.{said} Don't run it; do the work another way, or stop and say why you can't.",
                asked.shown
            )
        };
        match asked.reply.take() {
            Some(reply) => {
                let _ = reply.send(Response::ok(text));
            }
            None => self.tell_agent(thread, text).await,
        }
        self.agent_changed(thread).await;
        Ok("ok")
    }

    /// Who checks the thread's commands on Auto, as its permissions menu
    /// shows it; none for an agent with an auto mode of its own.
    pub(crate) fn auto_check_view(&self, thread: ThreadId) -> Option<AutoCheck> {
        let run = self.acp.get(&thread)?;
        if run.has_own_auto() {
            return None;
        }
        let own = (run.info.clone(), run.model());
        let workspace = self.start_workspace(Some(&thread.to_string()), None, None).ok()?;
        let chosen = self.judges.of(thread).cloned();
        let rules = self.rules(&workspace);
        let fixed_by = rules.fixed_judge().map(|(_, source)| source.to_string());
        let judge = rules.judge(chosen.as_ref(), workspace.defaults.auto_judge.as_ref());
        let name = match &judge {
            Judge::Thread => {
                let model = own.1.map(|(_, name)| name);
                model.map_or_else(|| own.0.name.clone(), |model| format!("{} on {model}", own.0.name))
            }
            Judge::Agent { agent, model } => {
                let info = registry::all(&self.data_dir).into_iter().find(|info| info.id == *agent);
                match info {
                    Some(info) => self.judge_name(&info, model.as_deref()),
                    None => agent.clone(),
                }
            }
        };
        Some(AutoCheck { judge, chosen, fixed_by, name })
    }

    /// Who checks the thread's commands on Auto: one the rules allow, or the
    /// workspace's default when none.
    pub(crate) async fn set_judge(&mut self, thread: ThreadId, judge: Option<Judge>) -> Result<&'static str, CoreError> {
        let workspace = self.start_workspace(Some(&thread.to_string()), None, None)?;
        let rules = self.rules(&workspace);
        if let Some((_, source)) = rules.fixed_judge() {
            return Err(CoreError::Invalid(format!("{source} set who checks in {}", workspace.name)));
        }
        if let Some(Judge::Agent { agent, .. }) = &judge {
            let info = std::iter::once(registry::local_agent(&crate::acp::silt_exe()))
                .chain(registry::all(&self.data_dir))
                .find(|info| info.id == *agent)
                .ok_or_else(|| CoreError::Invalid(format!("no agent called {agent}")))?;
            if !self.judge_allowed(&workspace, &info) {
                return Err(CoreError::Invalid(format!("{} isn't allowed in {}", info.name, workspace.name)));
            }
        }
        match judge {
            Some(judge) => self.judges.0.insert(thread.to_string(), judge),
            None => self.judges.0.remove(&thread.to_string()),
        };
        self.judges.save(&self.data_dir)?;
        self.agent_changed(thread).await;
        Ok("ok")
    }
}

/// Answers a permission request with the agent's "allow once" (else
/// "allow always"), or its "reject once" (else "reject always", else none,
/// which refuses it).
fn answer(run: &mut crate::acp::AcpRun, permission: &Permission, allow: bool) {
    let wanted: &[&str] = if allow { &["allow_once", "allow_always"] } else { &["reject_once", "reject_always"] };
    let option = wanted
        .iter()
        .find_map(|kind| permission.option_kinds.iter().find(|(_, offered)| offered == kind).map(|(id, _)| id.clone()));
    send(run, Command::Answer { id: permission.id, option });
}

fn judge_scratch(data_dir: &Path, thread: ThreadId) -> PathBuf {
    let n = uuid::Uuid::now_v7().simple().to_string();
    data_dir.join("judges").join(thread.to_string()).join(n)
}

fn spawn_judge(
    events: mpsc::UnboundedSender<Event>,
    thread: ThreadId,
    run: u64,
    permission: u64,
    data_dir: PathBuf,
    start: JudgeStart,
) {
    tokio::spawn(async move {
        let scratch = start.sandbox.as_ref().map(|sandbox| sandbox.scratch.clone());
        let outcome = judge(thread, &data_dir, start).await;
        if let Some(scratch) = scratch {
            let _ = std::fs::remove_dir_all(scratch);
        }
        let _ = events.send(Event::Judged(Box::new(Judged { thread, run, permission, outcome })));
    });
}

/// Starts the judge, asks it, and stops it: its verdict, or why there's
/// none.
async fn judge(thread: ThreadId, data_dir: &Path, judge: JudgeStart) -> Result<Verdict, String> {
    let JudgeStart { info, model, mut start, sandbox, prompt } = judge;
    let registry::AgentCommand { program, args, env } =
        registry::command(&info, data_dir).await.map_err(|error| format!("{} couldn't start: {error}", info.name))?;
    start.program = program;
    start.args = args;
    let given: Vec<(String, String)> = env.into_iter().chain(std::mem::take(&mut start.env)).collect();
    let inherited = registry::agent_environment(&info.id, data_dir);
    start.env = inherited.iter().chain(&given).cloned().collect();
    let mut outside = Vec::new();
    if let Some(sandbox) = &sandbox {
        std::fs::create_dir_all(&sandbox.scratch).map_err(|error| error.to_string())?;
        start.env = inherited;
        outside = crate::acp::sandboxed(&mut start, given, &info, thread, data_dir, sandbox)
            .map_err(|error| format!("{} couldn't start in Silt's sandbox: {error}", info.name))?;
    }
    let _ended = Aborting(outside);
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let handle = silt_acp::start(start.for_agent(&info.id), sender);
    let asked = async {
        let mut said = String::new();
        while let Some(event) = receiver.recv().await {
            match event {
                silt_acp::Event::Ready { modes, settings, .. } => {
                    let model_setting = settings.iter().find(|setting| crate::acp::is_model(&setting.category, &setting.id));
                    if let (Some(model), Some(setting)) = (&model, model_setting)
                        && setting.current != *model
                    {
                        handle.send(Command::SetSetting { id: setting.id.clone(), value: model.clone() });
                    }
                    if let Some(modes) = &modes
                        && let Some(mode) = crate::acp::mode_for_preset(Preset::AskForEverything, modes)
                        && mode != modes.current
                    {
                        handle.send(Command::SetMode(mode));
                    }
                    handle.send(Command::Prompt { text: prompt.clone(), attachments: Vec::new() });
                }
                silt_acp::Event::Update(Update::AgentText(text)) => said.push_str(&text),
                silt_acp::Event::Permission(permission) => {
                    // It may look, to see what a call would do, and nothing else.
                    let looking = matches!(permission.tool_kind.as_deref(), Some("read" | "search"));
                    let kinds: &[&str] =
                        if looking { &["allow_once", "allow_always"] } else { &["reject_once", "reject_always"] };
                    let option = kinds.iter().find_map(|kind| {
                        permission.option_kinds.iter().find(|(_, offered)| offered == kind).map(|(id, _)| id.clone())
                    });
                    handle.send(Command::Answer { id: permission.id, option });
                }
                silt_acp::Event::Question(question) => {
                    handle.send(Command::AnswerQuestion { id: question.id, answers: None });
                }
                silt_acp::Event::SignIn(_) => return Err(format!("{} needs signing in on this machine", info.name)),
                silt_acp::Event::TurnEnded { .. } => {
                    return silt_access::judge::verdict(&said).map_err(|_| format!("{}'s answer wasn't a verdict", info.name));
                }
                silt_acp::Event::Failed(error) => return Err(crate::protocol::error_chain(&*error)),
                silt_acp::Event::Exited => break,
                _ => {}
            }
        }
        Err(format!("{} stopped before it answered", info.name))
    };
    let outcome = tokio::time::timeout(JUDGE_WAIT, asked)
        .await
        .unwrap_or_else(|_| Err(format!("{}'s check took more than {} s", info.name, JUDGE_WAIT.as_secs())));
    handle.send(Command::Stop);
    let exited = async {
        while let Some(event) = receiver.recv().await {
            if matches!(event, silt_acp::Event::Exited) {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(JUDGE_ENDING, exited).await;
    outcome
}
