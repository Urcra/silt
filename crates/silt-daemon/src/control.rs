//! This daemon's control channel to the server (ADR 0026): one poll always
//! open for the commands people send its threads from elsewhere, and
//! reports of the agents it runs, when they change and as a heartbeat.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::time::{Duration, Instant};

use silt_proto::{
    AgentReport, AgentState, AgentsReport, AskedPermission, CommandAnswer, ControlCommand, ErrorReply, RelayRequest,
    RemoteCommand, RunningElsewhere, Startable,
};
use silt_types::ThreadId;
use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::core::{Core, CoreError, Event};
use crate::protocol::{AgentStatus, BusEvent, PermissionAsked, Preset, Request, Response, StartOptions};
use crate::remote::RemoteError;

/// How long one poll waits on the server for a command.
const POLL_WAIT: Duration = Duration::from_secs(25);
/// Reports are sent this often even when nothing changed, so the server
/// knows the device is still there.
const HEARTBEAT: Duration = Duration::from_secs(30);
/// How long to wait after the server failed a poll or report, such as one
/// that doesn't know the control channel yet.
const RETRY: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct Control {
    /// The channel to each server, by URL (ADR 0032).
    channels: HashMap<String, Channel>,
    /// Agents running on the person's other online devices, by thread, as
    /// the last listing of the server named with each said.
    elsewhere: HashMap<ThreadId, (String, RunningElsewhere)>,
}

#[derive(Default)]
struct Channel {
    polling: bool,
    poll_after: Option<Instant>,
    reporting: bool,
    report_after: Option<Instant>,
    /// What the server last took, and when.
    reported: Option<(Instant, Vec<AgentReport>)>,
}

/// What a server answered on its channel, by its URL.
pub(crate) enum ControlEvent {
    Polled(String, Result<Vec<ControlCommand>, String>),
    Reported(String, Result<Vec<AgentReport>, String>),
}

/// What this machine's web UI asks of an agent running on another of the
/// person's devices.
pub(crate) enum Elsewhere {
    PassOn(ThreadId, RemoteCommand),
    Refuse(&'static str),
}

fn waited(after: Option<Instant>) -> bool {
    after.is_none_or(|after| Instant::now() >= after)
}

impl Core {
    /// Keeps a poll open to each server, and reports the agents here when due.
    pub(crate) async fn drive_control(&mut self) {
        for remote in self.remotes.clone() {
            self.drive_channel(remote).await;
        }
    }

    async fn drive_channel(&mut self, remote: crate::remote::Remote) {
        let server = remote.config.url.clone();
        let channel = self.control.channels.entry(server.clone()).or_default();
        if !channel.polling && waited(channel.poll_after) {
            channel.polling = true;
            let (events, remote, server) = (self.events.clone(), remote.clone(), server.clone());
            tokio::spawn(async move {
                let polled = remote.control(POLL_WAIT).await.map(|reply| reply.commands).map_err(|error| error.to_string());
                let _ = events.send(Event::Control(ControlEvent::Polled(server, polled)));
            });
        }
        if channel.reporting || !waited(channel.report_after) {
            return;
        }
        let agents = self.agent_reports(&server).await;
        let channel = self.control.channels.entry(server.clone()).or_default();
        let due = channel.reported.as_ref().is_none_or(|(at, sent)| *sent != agents || at.elapsed() >= HEARTBEAT);
        if !due {
            return;
        }
        channel.reporting = true;
        let events = self.events.clone();
        let startable = silt_acp::registry::installed_here(&self.data_dir)
            .into_iter()
            .map(|agent| Startable { id: agent.id, name: agent.name })
            .collect();
        tokio::spawn(async move {
            let report = AgentsReport { agents, startable };
            let reported = remote.report_agents(&report).await.map(|()| report.agents).map_err(|error| error.to_string());
            let _ = events.send(Event::Control(ControlEvent::Reported(server, reported)));
        });
    }

    /// The agents here in threads that sync with `server`, which hears of
    /// no others.
    async fn agent_reports(&self, server: &str) -> Vec<AgentReport> {
        let mut threads: Vec<_> = self.acp.keys().copied().filter(|thread| self.homed_at(*thread, server)).collect();
        threads.sort();
        let mut agents = Vec::new();
        for thread in threads {
            let Some(status) = self.agent_status(thread) else {
                continue;
            };
            let queued = match self.store.queue(thread).await {
                Ok(queue) => u32::try_from(queue.len()).unwrap_or(u32::MAX),
                Err(error) => {
                    warn!(%error, "could not read the queue to report it");
                    0
                }
            };
            agents.push(AgentReport {
                thread,
                agent: status.agent,
                state: status.state,
                permissions: status
                    .permissions
                    .into_iter()
                    .map(|asked| AskedPermission { id: asked.id, title: asked.title, options: asked.options })
                    .collect(),
                questions: status.questions,
                sign_in: status.sign_in,
                background: self.background_here(thread),
                queued,
                preset: serde_json::to_value(self.preset(thread))
                    .ok()
                    .and_then(|preset| preset.as_str().map(str::to_owned))
                    .unwrap_or_default(),
            });
        }
        agents
    }

    pub(crate) async fn control_event(&mut self, event: ControlEvent) {
        match event {
            ControlEvent::Polled(server, Ok(commands)) => {
                self.control.channels.entry(server.clone()).or_default().polling = false;
                for command in commands {
                    self.run_command(&server, command).await;
                }
                self.drive_control().await;
            }
            ControlEvent::Polled(server, Err(error)) => {
                warn!(%error, server, "could not poll the server for commands");
                let channel = self.control.channels.entry(server).or_default();
                channel.polling = false;
                channel.poll_after = Some(Instant::now() + RETRY);
            }
            ControlEvent::Reported(server, Ok(agents)) => {
                let channel = self.control.channels.entry(server).or_default();
                channel.reporting = false;
                channel.reported = Some((Instant::now(), agents));
            }
            ControlEvent::Reported(server, Err(error)) => {
                warn!(%error, server, "could not report this device's agents");
                let channel = self.control.channels.entry(server).or_default();
                channel.reporting = false;
                channel.report_after = Some(Instant::now() + RETRY);
            }
        }
    }

    /// Runs a command as the same request from this machine's web UI would
    /// run, and answers it once that request is answered.
    async fn run_command(&mut self, server: &str, command: ControlCommand) {
        let Some(remote) = self.remotes.iter().find(|remote| remote.config.url == server).cloned() else {
            return;
        };
        let id = command.id;
        let request = self.checked(server, command);
        let (reply, response) = oneshot::channel();
        match request {
            Ok(request) => self.request(request, reply).await,
            Err(refusal) => {
                let _ = reply.send(Response::error(refusal));
            }
        }
        tokio::spawn(async move {
            let answer = match response.await {
                Ok(Response::Ok { data }) => CommandAnswer { ok: true, message: String::new(), data },
                Ok(Response::Error { message }) => CommandAnswer::refused(message),
                Err(_) => CommandAnswer::refused("the daemon stopped"),
            };
            if let Err(error) = remote.answer_command(id, &answer).await {
                warn!(%error, id, "could not answer a command");
            }
        });
    }

    /// The request a command from `server` makes here, once its sender may
    /// make it (ADR 0026): only this machine's own person there runs things
    /// on it, from a device allowed to, and only in threads that sync there
    /// (ADR 0032).
    fn checked(&self, server: &str, command: ControlCommand) -> Result<Request, String> {
        let me = self.remotes.iter().find(|remote| remote.config.url == server).map(|remote| remote.config.user.clone());
        let me = me.unwrap_or_default();
        if me.is_empty() {
            return Err("this machine doesn't know whose it is; sign it in again".into());
        }
        if command.sender != me {
            warn!(sender = command.sender, "refusing a command from someone else");
            return Err(format!("only {me} runs things on this machine"));
        }
        if self.threads.contains_key(&command.thread) && !self.homed_at(command.thread, server) {
            warn!(thread = %command.thread, server, "refusing a command for a thread of another server");
            return Err("that thread doesn't sync with this server".into());
        }
        command.capabilities.allow(&command.command).map_err(|needs| format!("that device may not {needs}"))?;
        info!(id = command.id, thread = %command.thread, from = ?command.from, "running a command from elsewhere");
        let thread = command.thread.to_string();
        let preset = |preset: String| {
            serde_json::from_value::<Preset>(serde_json::Value::String(preset)).map_err(|_| "unknown preset".to_owned())
        };
        Ok(match command.command {
            RemoteCommand::Prompt { text, plan } => Request::AcpPrompt { thread, text, plan, attachments: Vec::new() },
            RemoteCommand::Queue { text } => Request::QueueAdd { thread, text },
            RemoteCommand::Steer { text } => Request::AcpSteer { thread, text },
            RemoteCommand::Answer { id, option } => Request::AcpAnswer { thread, id, option },
            RemoteCommand::Reply { id, answers } => Request::AcpReply { thread, id, answers },
            RemoteCommand::Cancel => Request::AcpCancel { thread },
            RemoteCommand::Stop => Request::AcpStop { thread },
            RemoteCommand::Preset { preset: chosen } => Request::AgentPreset { thread, preset: preset(chosen)? },
            RemoteCommand::Mode { mode } => Request::AcpMode { thread, mode },
            RemoteCommand::Setting { id, value } => Request::AcpSetting { thread, id, value },
            RemoteCommand::MrShow { mr, hide_git } => Request::MrShow { mr, hide_git },
            RemoteCommand::MrSeen { mr, path, seen } => Request::MrSeen { mr, path, seen },
            RemoteCommand::FileAt { path, at } => Request::FileAt { thread, query: silt_proto::view::FileQuery { path, at } },
            RemoteCommand::SignIn { method } => Request::AcpSignIn { thread, method },
            RemoteCommand::SignInCode { code } => Request::AcpSignInCode { thread, code },
            RemoteCommand::SignInCancel => Request::AcpSignInCancel { thread },
            RemoteCommand::PageFiles { artifact, asked } => Request::PageFiles { thread, artifact, asked },
            RemoteCommand::PageMcp { artifact, asked } => Request::PageMcp {
                thread,
                artifact,
                asked: serde_json::from_value(asked).map_err(|error| format!("that isn't a connector call: {error}"))?,
            },
            RemoteCommand::Sample { question } => {
                Request::Sample { thread, artifact: None, ask: None, input: serde_json::Value::String(question) }
            }
            RemoteCommand::Start { agent, preset: chosen, mode } => Request::AcpStart {
                thread: Some(thread),
                clone: None,
                agent,
                options: StartOptions { mode, preset: chosen.map(preset).transpose()?, ..StartOptions::default() },
            },
        })
    }
}

impl Core {
    /// Takes in what the listing says runs on the person's other devices,
    /// and tells browsers of each thread whose agent's place changed.
    pub(crate) fn take_running_elsewhere(&mut self, server: &str, running: Vec<RunningElsewhere>) {
        let mut now: HashMap<ThreadId, (String, RunningElsewhere)> = self
            .control
            .elsewhere
            .iter()
            .filter(|(_, (from, _))| from != server)
            .map(|(thread, running)| (*thread, running.clone()))
            .collect();
        now.extend(running.into_iter().map(|running| (running.agent.thread, (server.to_owned(), running))));
        let changed: BTreeSet<ThreadId> = now
            .keys()
            .chain(self.control.elsewhere.keys())
            .copied()
            .filter(|thread| now.get(thread) != self.control.elsewhere.get(thread))
            .collect();
        self.control.elsewhere = now;
        for thread in &changed {
            self.announce_agent(*thread);
        }
        if !changed.is_empty() {
            self.announce(BusEvent::Threads);
        }
    }

    fn running_here(&self, thread: ThreadId) -> bool {
        self.acp.contains_key(&thread) && self.agent_state(thread).is_some_and(AgentState::running)
    }

    /// The agent another of the person's devices runs in the thread, while
    /// none runs here.
    fn elsewhere_of(&self, thread: ThreadId) -> Option<&RunningElsewhere> {
        if self.running_here(thread) { None } else { self.control.elsewhere.get(&thread).map(|(_, running)| running) }
    }

    /// How the thread's agent stands for this machine's web UI: the one
    /// here, or one running on another of the person's devices (ADR 0026).
    pub(crate) fn shown_status(&self, thread: ThreadId) -> Option<AgentStatus> {
        let Some(running) = self.elsewhere_of(thread) else {
            return self.agent_status(thread);
        };
        Some(AgentStatus {
            agent: format!("{} on {}", running.agent.agent, running.device_name),
            agent_id: String::new(),
            state: running.agent.state,
            streaming: String::new(),
            thinking: String::new(),
            permissions: running
                .agent
                .permissions
                .iter()
                .map(|asked| PermissionAsked {
                    id: asked.id,
                    title: asked.title.clone(),
                    options: asked.options.clone(),
                    plan: None,
                    tool_call: None,
                })
                .collect(),
            questions: running.agent.questions.clone(),
            sign_in: running.agent.sign_in.clone(),
            context: None,
            error: None,
            modes: None,
            settings: Vec::new(),
            takes_images: false,
            takes_files: false,
            commands: Vec::new(),
            stoppable: Vec::new(),
        })
    }

    pub(crate) fn shown_state(&self, thread: ThreadId) -> Option<AgentState> {
        self.elsewhere_of(thread).map(|running| running.agent.state).or_else(|| self.agent_state(thread))
    }

    /// How many commands and subagents the thread's agent left running,
    /// here or on the device running it.
    pub(crate) fn shown_background(&self, thread: ThreadId) -> u32 {
        match self.elsewhere_of(thread) {
            Some(running) => running.agent.background,
            None => self.background_here(thread),
        }
    }

    pub(crate) fn shown_preset(&self, thread: ThreadId) -> Preset {
        self.elsewhere_of(thread).and_then(|running| Preset::parse(&running.agent.preset)).unwrap_or_else(|| self.preset(thread))
    }

    /// The command `request` makes of an agent on another device, when the
    /// thread's agent runs there and not here.
    pub(crate) fn elsewhere(&self, request: &Request) -> Option<Elsewhere> {
        let (selector, command) = match request {
            Request::AcpPrompt { thread, text, plan, attachments } => {
                if !attachments.is_empty() {
                    let thread = self.find_thread(thread).ok()?;
                    return self
                        .elsewhere_of(thread)
                        .map(|_| Elsewhere::Refuse("attach files from the machine running the agent"));
                }
                (thread, RemoteCommand::Prompt { text: text.clone(), plan: *plan })
            }
            Request::AcpSteer { thread, text } => (thread, RemoteCommand::Steer { text: text.clone() }),
            Request::AcpStopTask { thread, .. } => {
                let thread = self.find_thread(thread).ok()?;
                return self.elsewhere_of(thread).map(|_| Elsewhere::Refuse("stop it from the machine running the agent"));
            }
            Request::AcpCancel { thread } => (thread, RemoteCommand::Cancel),
            Request::AcpAnswer { thread, id, option } => (thread, RemoteCommand::Answer { id: *id, option: option.clone() }),
            Request::AcpReply { thread, id, answers } => (thread, RemoteCommand::Reply { id: *id, answers: answers.clone() }),
            Request::AcpStop { thread } => (thread, RemoteCommand::Stop),
            Request::AcpSignIn { thread, method } => (thread, RemoteCommand::SignIn { method: method.clone() }),
            Request::AcpSignInCode { thread, code } => (thread, RemoteCommand::SignInCode { code: code.clone() }),
            Request::AcpSignInCancel { thread } => (thread, RemoteCommand::SignInCancel),
            Request::AcpMode { thread, mode } => (thread, RemoteCommand::Mode { mode: mode.clone() }),
            Request::AcpSetting { thread, id, value } => {
                (thread, RemoteCommand::Setting { id: id.clone(), value: value.clone() })
            }
            Request::AgentPreset { thread, preset } => (thread, RemoteCommand::Preset { preset: preset.as_str().to_owned() }),
            Request::QueueAdd { thread, text } => (thread, RemoteCommand::Queue { text: text.clone() }),
            _ => return None,
        };
        let thread = self.find_thread(selector).ok()?;
        self.elsewhere_of(thread).map(|_| Elsewhere::PassOn(thread, command))
    }

    /// Passes `command` on through the server to the device running the
    /// thread's agent, answering with its answer.
    pub(crate) fn pass_on(
        &self,
        thread: ThreadId,
        command: RemoteCommand,
    ) -> Result<impl Future<Output = Result<serde_json::Value, CoreError>> + Send + 'static, CoreError> {
        let remote = self.server_for(thread)?;
        Ok(async move {
            remote.relay(thread, &RelayRequest { command, when_back: false }).await.map_err(|error| match error {
                RemoteError::Refused(ErrorReply::Invalid { message } | ErrorReply::Forbidden { message }) => {
                    CoreError::Invalid(message)
                }
                source => CoreError::Server { doing: "pass the command on", source },
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use silt_proto::Capabilities;
    use silt_store::Store;
    use silt_types::{ActorId, DeviceId, ThreadId};
    use tokio::sync::mpsc;

    use super::*;
    use crate::remote::{Remote, RemoteConfig};

    fn command(sender: &str, capabilities: Capabilities, command: RemoteCommand) -> ControlCommand {
        ControlCommand { id: 1, thread: ThreadId(7), sender: sender.into(), from: None, capabilities, sent_ms: 0, command }
    }

    #[tokio::test]
    async fn only_this_machines_person_runs_things_on_it_from_an_allowed_device() {
        let data = tempfile::tempdir().expect("temp dir");
        let store = Store::open(data.path()).await.expect("opens");
        let (events, _inbox) = mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        let prompt = || RemoteCommand::Prompt { text: "hi".into(), plan: false };
        let work = "http://127.0.0.1:9";
        assert!(core.checked(work, command("alice", Capabilities::ALL, prompt())).is_err(), "not signed in");

        let sign_in = |url: &str, user: &str| {
            Remote::new(RemoteConfig {
                url: url.into(),
                token: String::new(),
                device: DeviceId(2),
                name: "workstation".into(),
                user: user.into(),
            })
        };
        core.remotes = vec![sign_in(work, "alice"), sign_in("http://127.0.0.1:10", "ada")];
        assert_eq!(
            core.checked(work, command("bob", Capabilities::ALL, prompt())).err().as_deref(),
            Some("only alice runs things on this machine")
        );
        assert_eq!(
            core.checked("http://127.0.0.1:10", command("alice", Capabilities::ALL, prompt())).err().as_deref(),
            Some("only ada runs things on this machine"),
            "each server knows its own person"
        );
        let answering = Capabilities { answer_permissions: true, review: true, ..Capabilities::default() };
        assert_eq!(
            core.checked(work, command("alice", answering, prompt())).err().as_deref(),
            Some("that device may not send prompts")
        );
        let answer = RemoteCommand::Answer { id: 3, option: Some("allow".into()) };
        assert!(matches!(
            core.checked(work, command("alice", answering, answer)),
            Ok(Request::AcpAnswer { id: 3, option: Some(_), .. })
        ));
        assert!(matches!(
            core.checked(work, command("alice", Capabilities::ALL, prompt())),
            Ok(Request::AcpPrompt { text, plan: false, .. }) if text == "hi"
        ));
        let unknown = RemoteCommand::Preset { preset: "anything goes".into() };
        assert_eq!(core.checked(work, command("alice", Capabilities::ALL, unknown)).err().as_deref(), Some("unknown preset"));
    }

    #[tokio::test]
    async fn a_server_runs_nothing_in_a_thread_of_another() {
        let data = tempfile::tempdir().expect("temp dir");
        let store = Store::open(data.path()).await.expect("opens");
        let (events, _inbox) = mpsc::unbounded_channel();
        let mut core = Core::load(store, ActorId(1), data.path(), events).await.expect("loads");
        let sign_in = |url: &str| {
            Remote::new(RemoteConfig {
                url: url.into(),
                token: String::new(),
                device: DeviceId(2),
                name: "workstation".into(),
                user: "alice".into(),
            })
        };
        let (work, oss) = ("http://127.0.0.1:9", "http://127.0.0.1:10");
        core.remotes = vec![sign_in(work), sign_in(oss)];
        let family = silt_types::FamilyId(3);
        core.threads.insert(
            ThreadId(7),
            silt_store::ThreadRecord {
                id: ThreadId(7),
                family,
                repo_root: Default::default(),
                repo: "example.com/app".into(),
                created_ms: 0,
                owner: "alice".into(),
            },
        );
        core.set_home(family, crate::homes::Home::Server(oss.into())).await.expect("saves");
        let prompt = RemoteCommand::Prompt { text: "hi".into(), plan: false };
        assert_eq!(
            core.checked(work, command("alice", Capabilities::ALL, prompt.clone())).err().as_deref(),
            Some("that thread doesn't sync with this server")
        );
        assert!(core.checked(oss, command("alice", Capabilities::ALL, prompt.clone())).is_ok());
        core.set_home(family, crate::homes::Home::Local).await.expect("saves");
        assert!(core.checked(oss, command("alice", Capabilities::ALL, prompt)).is_err(), "a local thread takes no commands");
    }
}
