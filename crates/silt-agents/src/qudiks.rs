//! Qudiks, a fork of Codex that also runs on GitHub Copilot, OpenAI API
//! credits and local endpoints (https://github.com/viterkim/qudiks). Its
//! sessions are Codex rollouts, so this adapter is Codex's with Qudiks'
//! command and home: `$QUDIKS_HOME`, else `$CODEX_HOME`, else `~/.qudiks`,
//! the order Qudiks reads them in.

use std::path::{Path, PathBuf};

use crate::conventions::NativeConventions;
use crate::{
    Agent, AgentError, CheckedSession, Entry, Found, Launch, LaunchPlan, OnDisk, Reader, SessionIds, codex, translated_text,
};

pub const AGENT: &str = "qudiks";

const DIR: &str = ".qudiks";

/// The variables that move its home, the first set winning.
const VARIABLES: [&str; 2] = ["QUDIKS_HOME", "CODEX_HOME"];

/// Its installer turns YOLO on in the config and in the `qudiks` launcher,
/// so a workspace that keeps it asking sets the sandbox back as well as the
/// approvals; Silt's settings come after the launcher's, and later ones win.
const ASKING: [&str; 2] = ["approval_policy=\"on-request\"", "sandbox_mode=\"workspace-write\""];

/// Its home as `value` reads the run's variables, else under `user_home`.
fn resolve(value: impl Fn(&str) -> Option<PathBuf>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    VARIABLES.into_iter().find_map(value).or_else(|| user_home.map(|home| home.join(DIR)))
}

fn process_value(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable).filter(|value| !value.is_empty()).map(PathBuf::from)
}

pub fn home() -> Option<PathBuf> {
    resolve(process_value, crate::home())
}

/// [`home`] for a session in `cwd`, whose workspace may set its own (ADR
/// 0065). A workspace's variable is set on top of this process's.
fn home_in(cwd: &Path) -> Option<PathBuf> {
    resolve(|variable| crate::workspace_home(variable, cwd).or_else(|| process_value(variable)), crate::home())
}

/// The homes to import from: this process's and each workspace's own,
/// leaving out those that are a Codex home too, whose sessions Codex's
/// adapter imports, as nothing in a rollout says which of the two wrote it.
fn import_homes() -> Vec<PathBuf> {
    let own = home().into_iter().chain(crate::workspace_homes("QUDIKS_HOME"));
    apart_from(own, &crate::agent_homes("CODEX_HOME", ".codex"))
}

/// Each of `homes` once, leaving out `codex`'s.
fn apart_from(homes: impl IntoIterator<Item = PathBuf>, codex: &[PathBuf]) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for home in homes {
        if !codex.contains(&home) && !kept.contains(&home) {
            kept.push(home);
        }
    }
    kept
}

/// The Qudiks adapter.
pub struct Qudiks;

impl Agent for Qudiks {
    fn homes(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        let value = |variable: &str| env(variable).filter(|value| !value.is_empty()).map(PathBuf::from);
        resolve(value, Some(home.to_owned())).into_iter().collect()
    }

    fn sign_in(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        self.homes(home, env).into_iter().map(|dir| dir.join("auth.json")).collect()
    }

    fn memory_stores(
        &self,
        home: &Path,
        env: &dyn Fn(&str) -> Option<String>,
        project: Option<&Path>,
    ) -> Vec<crate::memory::Store> {
        self.homes(home, env).iter().flat_map(|dir| codex::memory_stores_in(dir, project)).collect()
    }

    fn read_only_home(&self, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        self.homes(home, env).into_iter().map(|dir| dir.join("config.toml")).collect()
    }

    fn id(&self) -> &'static str {
        AGENT
    }

    fn command(&self) -> &'static str {
        "qudiks"
    }

    fn display_name(&self) -> &'static str {
        "Qudiks"
    }

    fn native(&self) -> NativeConventions {
        codex::Codex.native()
    }

    fn reader(&self) -> Box<dyn Reader> {
        codex::Codex.reader()
    }

    fn session_ids(&self) -> SessionIds {
        SessionIds::Discovered
    }

    fn transcript(&self, cwd: &Path, id: &str) -> Option<PathBuf> {
        codex::transcript_in(&home_in(cwd)?, id)
    }

    fn side_transcripts(&self, transcript: &Path) -> Vec<(String, PathBuf)> {
        codex::subagent_rollouts(transcript)
    }

    fn discover(&self, cwd: &Path, since_ms: i64, known: &[String]) -> Option<Found> {
        codex::discover_in(&home_in(cwd)?, cwd, since_ms, known)
    }

    fn on_disk(&self, _dirs: &[PathBuf]) -> Vec<OnDisk> {
        import_homes().iter().flat_map(|home| codex::on_disk_in(home)).collect()
    }

    fn restore(&self, root: &Path, cwd: &Path, session: &CheckedSession) -> Result<PathBuf, AgentError> {
        let home = home_in(cwd).ok_or(AgentError::NoHome)?;
        codex::restore_into(&home, root, cwd, session)
    }

    fn translate(&self, entries: &[Entry], cwd: &Path, id: &str) -> Option<Vec<String>> {
        Some(codex::rollout_lines(&translated_text(entries), cwd, id))
    }

    fn bypass_flag(&self, args: &[String]) -> Option<String> {
        codex::bypass_flag(args)
    }

    fn sandbox_support(&self) -> crate::SandboxSupport {
        crate::SandboxSupport::Sandbox
    }

    fn launch(&self, plan: &LaunchPlan) -> Result<Launch, AgentError> {
        Ok(codex::launch_as(self.command(), plan, &ASKING))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_home_is_qudiks_home_then_codex_home_then_its_own_folder() {
        let user = Some(PathBuf::from("/u"));
        let set = |vars: &'static [(&'static str, &'static str)]| {
            move |variable: &str| vars.iter().find(|(name, _)| *name == variable).map(|(_, value)| PathBuf::from(value))
        };
        assert_eq!(resolve(set(&[("CODEX_HOME", "/c"), ("QUDIKS_HOME", "/q")]), user.clone()), Some("/q".into()));
        assert_eq!(resolve(set(&[("CODEX_HOME", "/c")]), user.clone()), Some("/c".into()));
        assert_eq!(resolve(set(&[]), user), Some("/u/.qudiks".into()));
    }

    #[test]
    fn a_workspace_that_keeps_it_asking_sets_its_sandbox_back_too() {
        let plan = |no_bypass| LaunchPlan {
            start: crate::Start::Resume { id: "s1".into(), prompt: None },
            cwd: "/w".into(),
            mcp: crate::McpServer { command: "silt".into(), args: vec!["mcp".into()] },
            hook_command: String::new(),
            hook_program: "silt".into(),
            instructions: None,
            scratch: "/tmp".into(),
            skills: Vec::new(),
            args: vec!["--github".into()],
            locks: crate::Locks { no_bypass, ..crate::Locks::default() },
            env: Vec::new(),
            memory: None,
        };
        let args = |no_bypass| -> Vec<String> {
            let launch = Qudiks.launch(&plan(no_bypass)).expect("launches");
            launch.args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
        };
        let asking = args(true);
        assert_eq!(asking[0], "resume");
        assert!(asking.contains(&ASKING[0].to_owned()) && asking.contains(&ASKING[1].to_owned()), "{asking:?}");
        assert_eq!(&asking[asking.len() - 2..], ["--github", "s1"], "its own flags, then the session");
        assert!(!args(false).iter().any(|arg| arg.starts_with("approval_policy") || arg.starts_with("sandbox_mode")));
    }

    #[test]
    fn a_resumed_session_is_told_first_what_changed() {
        let plan = LaunchPlan {
            start: crate::Start::Resume { id: "s1".into(), prompt: Some("/src/lib is now in your sandbox".into()) },
            cwd: "/w".into(),
            mcp: crate::McpServer { command: "silt".into(), args: vec!["mcp".into()] },
            hook_command: String::new(),
            hook_program: "silt".into(),
            instructions: None,
            scratch: "/tmp".into(),
            skills: Vec::new(),
            args: vec!["--github".into()],
            locks: crate::Locks::default(),
            env: Vec::new(),
            memory: None,
        };
        let words =
            |launch: crate::Launch| -> Vec<String> { launch.args.iter().map(|arg| arg.to_string_lossy().into_owned()).collect() };
        let codex = words(Qudiks.launch(&plan).expect("launches"));
        assert_eq!(&codex[codex.len() - 3..], ["--github", "s1", "/src/lib is now in your sandbox"], "{codex:?}");
        let claude = words(crate::claude::ClaudeCode.launch(&plan).expect("launches"));
        assert_eq!(claude.last().map(String::as_str), Some("/src/lib is now in your sandbox"), "{claude:?}");
        assert!(claude.windows(2).any(|pair| pair == ["--resume", "s1"]), "{claude:?}");
    }

    #[test]
    fn a_home_codex_shares_is_left_to_codex_s_import() {
        let homes = ["/u/.qudiks", "/c", "/w/q", "/u/.qudiks"].map(PathBuf::from);
        assert_eq!(apart_from(homes, &[PathBuf::from("/c")]), [PathBuf::from("/u/.qudiks"), PathBuf::from("/w/q")]);
    }

    #[test]
    fn a_run_s_home_ignores_an_empty_variable() {
        let env = |variable: &str| (variable == "QUDIKS_HOME").then(String::new);
        assert_eq!(Qudiks.homes(Path::new("/u"), &env), vec![PathBuf::from("/u/.qudiks")]);
    }
}
