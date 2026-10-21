use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("HOME is not set")]
    NoHome,
    #[error("ACP sessions continue from the web UI; resume the thread with `--agent` to use an agent CLI")]
    AcpSession,
    #[error("this agent's sessions are files")]
    NoSnapshot,
    #[error("{} has no directory", .0.display())]
    NoDirectory(PathBuf),
    #[error("opencode {command} failed: {stderr}")]
    OpenCode { command: String, stderr: String },
    #[error("OpenCode's export is not JSON")]
    Export(#[source] serde_json::Error),
    /// A recorded session whose id, subagent names or side files would lead
    /// outside its agent's session store.
    #[error("the recorded session has an unsafe {0}")]
    Unsafe(&'static str),
}
