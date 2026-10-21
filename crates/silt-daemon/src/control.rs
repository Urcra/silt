//! This daemon's control channel to the server (ADR 0026): one poll always
//! open for the commands people send its threads from elsewhere, and
//! reports of the agents it runs, when they change and as a heartbeat.

use std::time::{Duration, Instant};

use silt_proto::{AgentReport, AgentsReport, AskedPermission, CommandAnswer, ControlCommand};
use tracing::warn;

use crate::core::{Core, Event};

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
    polling: bool,
    poll_after: Option<Instant>,
    reporting: bool,
    report_after: Option<Instant>,
    /// What the server last took, and when.
    reported: Option<(Instant, Vec<AgentReport>)>,
}

pub(crate) enum ControlEvent {
    Polled(Result<Vec<ControlCommand>, String>),
    Reported(Result<Vec<AgentReport>, String>),
}

fn waited(after: Option<Instant>) -> bool {
    after.is_none_or(|after| Instant::now() >= after)
}

impl Core {
    /// Keeps a poll open, and reports the agents here when due.
    pub(crate) async fn drive_control(&mut self) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        if !self.control.polling && waited(self.control.poll_after) {
            self.control.polling = true;
            let (events, remote) = (self.events.clone(), remote.clone());
            tokio::spawn(async move {
                let polled = remote.control(POLL_WAIT).await.map(|reply| reply.commands).map_err(|error| error.to_string());
                let _ = events.send(Event::Control(ControlEvent::Polled(polled)));
            });
        }
        if self.control.reporting || !waited(self.control.report_after) {
            return;
        }
        let agents = self.agent_reports().await;
        let due = self.control.reported.as_ref().is_none_or(|(at, sent)| *sent != agents || at.elapsed() >= HEARTBEAT);
        if !due {
            return;
        }
        self.control.reporting = true;
        let events = self.events.clone();
        tokio::spawn(async move {
            let report = AgentsReport { agents };
            let reported = remote.report_agents(&report).await.map(|()| report.agents).map_err(|error| error.to_string());
            let _ = events.send(Event::Control(ControlEvent::Reported(reported)));
        });
    }

    async fn agent_reports(&self) -> Vec<AgentReport> {
        let mut threads: Vec<_> = self.acp.keys().copied().collect();
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
                queued,
            });
        }
        agents
    }

    pub(crate) async fn control_event(&mut self, event: ControlEvent) {
        match event {
            ControlEvent::Polled(Ok(commands)) => {
                self.control.polling = false;
                for command in commands {
                    let id = command.id;
                    let answer = self.run_command(command).await;
                    let Some(remote) = self.remote.clone() else {
                        break;
                    };
                    tokio::spawn(async move {
                        if let Err(error) = remote.answer_command(id, &answer).await {
                            warn!(%error, id, "could not answer a command");
                        }
                    });
                }
                self.drive_control().await;
            }
            ControlEvent::Polled(Err(error)) => {
                warn!(%error, "could not poll the server for commands");
                self.control.polling = false;
                self.control.poll_after = Some(Instant::now() + RETRY);
            }
            ControlEvent::Reported(Ok(agents)) => {
                self.control.reporting = false;
                self.control.reported = Some((Instant::now(), agents));
            }
            ControlEvent::Reported(Err(error)) => {
                warn!(%error, "could not report this device's agents");
                self.control.reporting = false;
                self.control.report_after = Some(Instant::now() + RETRY);
            }
        }
    }

    async fn run_command(&mut self, command: ControlCommand) -> CommandAnswer {
        warn!(id = command.id, sender = command.sender, "refusing a command from elsewhere");
        CommandAnswer { ok: false, message: "this device doesn't take commands from elsewhere yet".into() }
    }
}
