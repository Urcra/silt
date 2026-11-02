//! Signing a thread's agent in when its own sign-in expired (ADR 0056): the
//! card's state, the sign-in program, and starting the agent again once it
//! signed in.

use silt_acp::registry::{self, AgentCommand};
use silt_acp::{SignInHow, SignInMethod};
use silt_agents::sign_in::{Login, LoginEvent, LoginScreen};
use silt_proto::{AgentState, SignInAsked, SignInChoice, SignInShown};
use silt_types::ThreadId;

use super::{AcpEvent, AcpRun, send, spawn_agent};
use crate::core::{Core, CoreError, Event};

/// How much of a sign-in's output is kept, from its end.
const KEPT_OUTPUT: usize = 256 * 1024;

/// The agent needs signing in: the ways it lists, and the sign-in under way.
#[derive(Debug, Default)]
pub(super) struct SignIn {
    methods: Vec<SignInMethod>,
    running: Option<Running>,
    failed: Option<String>,
}

#[derive(Debug)]
struct Running {
    method: String,
    /// Which sign-in reports, so a cancelled one's late events are ignored.
    token: u64,
    /// The program, for a terminal method; none while the agent signs in by
    /// itself.
    login: Option<Login>,
    output: Vec<u8>,
    code_sent: bool,
    /// What it printed before the last code was typed into it.
    typed_at: Option<usize>,
    cancelled: bool,
}

impl SignIn {
    pub(super) fn asked(&self) -> SignInAsked {
        SignInAsked {
            methods: self
                .methods
                .iter()
                .map(|method| SignInChoice {
                    id: method.id.clone(),
                    name: method.name.clone(),
                    description: method.description.clone(),
                    by_agent: method.how == SignInHow::Agent,
                })
                .collect(),
            running: self.running.as_ref().map(|running| {
                let screen = LoginScreen::read(&running.output);
                SignInShown {
                    method: running.method.clone(),
                    link: screen.link,
                    code: screen.code,
                    asks_code: screen.asks_code && !running.code_sent,
                    code_sent: running.code_sent,
                    tail: screen.tail,
                }
            }),
            failed: self.failed.clone(),
        }
    }

    /// The agent signing in by itself, when it is.
    pub(super) fn by_agent(&self) -> bool {
        self.running.as_ref().is_some_and(|running| running.login.is_none())
    }
}

/// The program a terminal method runs: the agent's own command with the
/// method's arguments, or the one it names.
#[derive(Debug, PartialEq, Eq)]
struct LoginCommand {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

fn program_for(agent_id: &str, how: &SignInHow, command: &AgentCommand) -> Option<LoginCommand> {
    match how {
        SignInHow::Terminal { args, env } => Some(LoginCommand {
            program: command.program.clone(),
            args: command.args.iter().chain(args).cloned().collect(),
            env: env.clone(),
        }),
        SignInHow::Program { program, args, env } => {
            let mut args = args.clone();
            // Its browser flow calls back to 127.0.0.1, which only a browser on this machine reaches.
            if agent_id.contains("copilot") && args == ["login"] {
                args.push("--device-code".to_owned());
            }
            Some(LoginCommand { program: program.clone(), args, env: env.clone() })
        }
        SignInHow::Agent => None,
    }
}

/// The methods a person can follow from a card. Claude's adapter offers its
/// whole terminal UI (`--cli` alone) once a sign-in was made, where the
/// person would type `/login`; its `auth login`, which it offers before
/// any, signs in the same way and asks only for the code, so that's offered
/// instead.
fn followable(agent_id: &str, methods: Vec<SignInMethod>) -> Vec<SignInMethod> {
    let whole_ui = |method: &SignInMethod| matches!(&method.how, SignInHow::Terminal { args, .. } if args == &["--cli"]);
    if !agent_id.contains("claude") || !methods.iter().any(whole_ui) {
        return methods;
    }
    let mut followable = Vec::new();
    for method in methods {
        let SignInHow::Terminal { env, .. } = &method.how else {
            followable.push(method);
            continue;
        };
        if !whole_ui(&method) {
            followable.push(method);
            continue;
        }
        let login = |id: &str, name: &str, description: &str, flag: &str| SignInMethod {
            id: id.to_owned(),
            name: name.to_owned(),
            description: Some(description.to_owned()),
            how: SignInHow::Terminal { args: ["--cli", "auth", "login", flag].map(str::to_owned).to_vec(), env: env.clone() },
        };
        followable.push(login("claude-ai-login", "Claude Subscription", "Use Claude subscription", "--claudeai"));
        followable.push(login("console-login", "Anthropic Console", "Use Anthropic Console (API usage billing)", "--console"));
    }
    let mut seen = std::collections::HashSet::new();
    followable.retain(|method| seen.insert(method.id.clone()));
    followable
}

impl Core {
    /// The agent said it needs signing in; a prompt it refused goes again
    /// once it is.
    pub(super) async fn sign_in_needed(&mut self, thread: ThreadId, methods: Vec<SignInMethod>) -> Result<(), CoreError> {
        self.flush_acp(thread).await?;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let failed = match &run.sign_in {
            Some(sign_in) if sign_in.by_agent() => Some(format!("{} didn't sign in", run.info.name)),
            Some(sign_in) => sign_in.failed.clone(),
            None => None,
        };
        if run.state == AgentState::Working {
            run.resend = run.last_prompt.take();
        }
        let methods = followable(&run.info.id, methods);
        run.sign_in = Some(SignIn { methods, running: None, failed });
        run.error = None;
        Ok(())
    }

    fn signing_in(&mut self, selector: &str) -> Result<(ThreadId, &mut AcpRun), CoreError> {
        let thread = self.find_thread(selector)?;
        let run = self
            .acp
            .get_mut(&thread)
            .filter(|run| run.sign_in.is_some())
            .ok_or_else(|| CoreError::Invalid("the agent isn't waiting to be signed in".into()))?;
        Ok((thread, run))
    }

    /// Signs the thread's agent in by `method`, one of those it listed.
    pub(crate) async fn acp_sign_in(&mut self, selector: &str, method: &str) -> Result<&'static str, CoreError> {
        let (thread, run) = self.signing_in(selector)?;
        let sign_in = run.sign_in.as_ref().expect("checked above");
        if sign_in.running.is_some() {
            return Err(CoreError::Invalid("a sign-in is under way; cancel it first".into()));
        }
        let chosen = sign_in
            .methods
            .iter()
            .find(|offered| offered.id == method)
            .cloned()
            .ok_or_else(|| CoreError::Invalid(format!("the agent lists no sign-in called {method}")))?;
        let (info, cwd) = (run.info.clone(), run.checkout.clone());
        self.next_token += 1;
        let token = self.next_token;
        if chosen.how == SignInHow::Agent {
            let run = self.acp.get_mut(&thread).expect("checked above");
            let sign_in = run.sign_in.as_mut().expect("checked above");
            sign_in.failed = None;
            sign_in.running = Some(Running {
                method: chosen.id.clone(),
                token,
                login: None,
                output: Vec::new(),
                code_sent: false,
                typed_at: None,
                cancelled: false,
            });
            self.restart_agent(thread, Some(chosen.id)).await;
            return Ok("signing in");
        }
        let command = registry::command(&info, &self.data_dir).await.map_err(|error| CoreError::Invalid(error.to_string()))?;
        let LoginCommand { program, args, env } = program_for(&info.id, &chosen.how, &command).expect("a terminal method");
        let env: Vec<(String, String)> =
            registry::agent_environment(&info.id, &self.data_dir).into_iter().chain(command.env).chain(env).collect();
        let (events, run_token) = (self.events.clone(), self.acp[&thread].run);
        let report = move |event| {
            let _ = events.send(Event::Acp { thread, run: run_token, event: AcpEvent::Login { token, event } });
        };
        let login = Login::start(&program, &args, &env, &cwd, report)
            .map_err(|error| CoreError::Invalid(format!("could not run {program}: {error}")))?;
        let run = self.acp.get_mut(&thread).expect("checked above");
        let sign_in = run.sign_in.as_mut().expect("checked above");
        sign_in.failed = None;
        sign_in.running = Some(Running {
            method: chosen.id,
            token,
            login: Some(login),
            output: Vec::new(),
            code_sent: false,
            typed_at: None,
            cancelled: false,
        });
        self.agent_changed(thread).await;
        Ok("signing in")
    }

    /// Types a code from the sign-in's page into the sign-in.
    pub(crate) async fn acp_sign_in_code(&mut self, selector: &str, code: &str) -> Result<&'static str, CoreError> {
        if code.trim().is_empty() || code.contains(['\n', '\r']) {
            return Err(CoreError::Invalid("paste the code on one line".into()));
        }
        let (thread, run) = self.signing_in(selector)?;
        let running = run.sign_in.as_mut().and_then(|sign_in| sign_in.running.as_mut());
        let Some(running) = running.filter(|running| running.login.is_some()) else {
            return Err(CoreError::Invalid("no sign-in is waiting for a code".into()));
        };
        let login = running.login.as_mut().expect("checked above");
        login.type_line(code).map_err(|error| CoreError::Invalid(format!("could not pass the code on: {error}")))?;
        running.code_sent = true;
        running.typed_at = Some(running.output.len());
        self.agent_changed(thread).await;
        Ok("sent")
    }

    pub(crate) async fn acp_sign_in_cancel(&mut self, selector: &str) -> Result<&'static str, CoreError> {
        let (thread, run) = self.signing_in(selector)?;
        let sign_in = run.sign_in.as_mut().expect("checked above");
        match sign_in.running.as_mut() {
            Some(running) if running.login.is_some() => {
                running.cancelled = true;
                running.login.as_mut().expect("checked above").stop();
            }
            // The agent signing in by itself stops with it, and waits as it did.
            Some(_) => {
                sign_in.running = None;
                if let Some(handle) = run.handle.take() {
                    handle.send(silt_acp::Command::Stop);
                }
            }
            None => return Err(CoreError::Invalid("no sign-in is under way".into())),
        }
        self.agent_changed(thread).await;
        Ok("cancelled")
    }

    pub(super) async fn login_event(&mut self, thread: ThreadId, token: u64, event: LoginEvent) {
        let Some(run) = self.acp.get_mut(&thread) else { return };
        let name = run.info.name.clone();
        let Some(sign_in) = run.sign_in.as_mut() else { return };
        let Some(running) = sign_in.running.as_mut().filter(|running| running.token == token) else { return };
        match event {
            LoginEvent::Output(chunk) => {
                running.output.extend(chunk);
                let over = running.output.len().saturating_sub(KEPT_OUTPUT);
                running.output.drain(..over);
                running.typed_at = running.typed_at.map(|at| at.saturating_sub(over));
                let screen = LoginScreen::read(&running.output);
                // A code pasted back was taken once the program says more.
                running.code_sent &= screen.asks_code;
                // A program that says the code failed, and waits, is done.
                let since_typed = running.typed_at.map(|at| LoginScreen::read(&running.output[at..]));
                if since_typed.as_ref().and_then(LoginScreen::failure).is_some() && screen.failure().is_some() {
                    running.typed_at = None;
                    if let Some(login) = running.login.as_mut() {
                        login.stop();
                    }
                }
                self.announce_agent(thread);
            }
            LoginEvent::Exited { success: true } => {
                run.sign_in = None;
                self.restart_agent(thread, None).await;
            }
            LoginEvent::Exited { success: false } => {
                let cancelled = running.cancelled;
                let last = LoginScreen::read(&running.output).tail.lines().last().map(str::to_owned);
                sign_in.running = None;
                sign_in.failed = (!cancelled).then(|| match last {
                    Some(last) => format!("the sign-in ended without signing {name} in: {last}"),
                    None => format!("the sign-in ended without signing {name} in"),
                });
                self.agent_changed(thread).await;
            }
        }
    }

    /// Starts the thread's agent again, loading its session, now that it
    /// signed in, or asking it to sign in by `method` first.
    async fn restart_agent(&mut self, thread: ThreadId, method: Option<String>) {
        if let Err(error) = self.close_agent_session(thread).await {
            tracing::warn!(%error, "could not close the agent's session");
        }
        let run = self.acp.get_mut(&thread).expect("caller checked");
        if let Some(handle) = run.handle.take() {
            handle.send(silt_acp::Command::Stop);
        }
        let agent = run.info.id.clone();
        let checkout = run.checkout.clone();
        let load = self.earlier_session(thread, &agent);
        let mut start = self.agent_start(thread, &checkout, load.as_ref().map(|(_, native)| native.clone()));
        start.sign_in_first = method;
        self.next_token += 1;
        let token = self.next_token;
        let run = self.acp.get_mut(&thread).expect("caller checked");
        run.run = token;
        run.load = load;
        run.state = AgentState::Starting;
        run.queued.clear();
        spawn_agent(self.events.clone(), thread, token, run.info.clone(), self.data_dir.clone(), start);
        self.agent_changed(thread).await;
    }

    /// Sends a prompt the agent refused for want of a sign-in, as it was:
    /// the log has it already.
    pub(super) fn resend(&mut self, thread: ThreadId) -> bool {
        let run = self.acp.get_mut(&thread).expect("caller checked");
        let Some((text, attachments)) = run.resend.take() else { return false };
        run.state = AgentState::Working;
        run.last_prompt = Some((text.clone(), attachments.clone()));
        send(run, silt_acp::Command::Prompt { text, attachments });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command() -> AgentCommand {
        AgentCommand { program: "npx".into(), args: vec!["-y".into(), "adapter@1".into()], env: vec![("A".into(), "1".into())] }
    }

    #[test]
    fn a_terminal_method_runs_the_agent_s_own_command_with_its_arguments() {
        let how = SignInHow::Terminal { args: vec!["--cli".into(), "auth".into(), "login".into()], env: vec![] };
        let login = program_for("claude-acp", &how, &command()).expect("a program");
        assert_eq!(login.program, "npx");
        assert_eq!(login.args, ["-y", "adapter@1", "--cli", "auth", "login"]);
    }

    #[test]
    fn claude_s_whole_terminal_ui_is_offered_as_its_sign_in_alone() {
        // As Claude's adapter 0.99.1 lists it once an expired sign-in is there.
        let ui = SignInMethod {
            id: "claude-login".into(),
            name: "Log in with Claude".into(),
            description: Some("Run `claude /login` in the terminal".into()),
            how: SignInHow::Terminal { args: vec!["--cli".into()], env: vec![] },
        };
        let offered = followable("claude-acp", vec![ui.clone()]);
        let ids: Vec<&str> = offered.iter().map(|method| method.id.as_str()).collect();
        assert_eq!(ids, ["claude-ai-login", "console-login"]);
        assert_eq!(
            offered[0].how,
            SignInHow::Terminal { args: ["--cli", "auth", "login", "--claudeai"].map(str::to_owned).to_vec(), env: vec![] }
        );
        assert_eq!(followable("other", vec![ui.clone()]), [ui], "another agent's methods are offered as it lists them");
    }

    #[test]
    fn copilot_s_login_uses_the_device_flow_which_works_from_anywhere() {
        let how = SignInHow::Program { program: "/opt/copilot".into(), args: vec!["login".into()], env: vec![] };
        let login = program_for("github-copilot-cli", &how, &command()).expect("a program");
        assert_eq!(
            (login.program.as_str(), login.args.as_slice()),
            ("/opt/copilot", ["login".to_owned(), "--device-code".to_owned()].as_slice())
        );
        let login = program_for("other", &how, &command()).expect("a program");
        assert_eq!(login.args, ["login"], "another agent's program runs as it lists it");
        assert_eq!(program_for("other", &SignInHow::Agent, &command()), None);
    }
}
