//! What a tool call does and how it ended, the same for every agent: each
//! names its tools and their inputs its own way, and a thread shows them
//! alike (ADR 0044).

use serde_json::Value;
use silt_proto::view::{
    AnswerView, ChoiceView, HistoryEntry, ImageView, PlanStepView, QuestionView, TaskView, ToolOutcome, ToolView, UsageView,
};

/// What a tool call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Runs a command.
    Execute,
    /// Reads a file or lists a directory.
    Read,
    /// Searches files or their contents.
    Search,
    /// Changes or writes a file.
    Edit,
    /// Asks the person something.
    Question,
    /// Fetches a page or searches the web.
    Fetch,
    /// Hands work to a subagent.
    Subagent,
    Other,
}

impl ToolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Execute => "execute",
            Self::Read => "read",
            Self::Search => "search",
            Self::Edit => "edit",
            Self::Question => "question",
            Self::Fetch => "fetch",
            Self::Subagent => "subagent",
            Self::Other => "other",
        }
    }
}

/// A tool call as a thread shows it: its kind, what it acts on (the
/// command, file or pattern) and where or why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub kind: ToolKind,
    pub subject: Option<String>,
    pub detail: Option<String>,
}

/// Reads a call from its tool's name, the kind its agent gave it if any
/// (ACP agents give one) and its input.
pub fn describe(name: &str, kind: Option<&str>, input: &Value) -> ToolUse {
    let kind = kind.and_then(acp_kind).unwrap_or_else(|| kind_of(name));
    // Copilot over ACP names its task tool by what the subagent does.
    let kind = if kind == ToolKind::Other && ["agent_type", "subagent_type"].iter().any(|key| input.get(*key).is_some()) {
        ToolKind::Subagent
    } else {
        kind
    };
    let text = |keys: &[&str]| keys.iter().find_map(|key| input.get(*key).and_then(string_of));
    let (subject, detail) = match kind {
        ToolKind::Execute => (text(&["command", "cmd", "CommandLine"]), text(&["description", "toolSummary"])),
        ToolKind::Read | ToolKind::Edit => {
            (text(&["file_path", "path", "filePath", "absolute_path", "notebook_path", "dir_path", "Path"]), None)
        }
        ToolKind::Search => (text(&["pattern", "query", "glob", "regex"]), text(&["path", "dir_path", "include"])),
        ToolKind::Question => (question(input), None),
        ToolKind::Fetch => (text(&["url", "query"]), None),
        ToolKind::Subagent => (
            text(&["description", "title", "prompt", "task", "message", "query", "Role", "Prompt"]),
            text(&["subagent_type", "agent_type", "agent", "agent_name", "name", "TypeName"]),
        ),
        ToolKind::Other => (None, None),
    };
    ToolUse { kind, subject, detail }
}

/// The kinds ACP names (`ToolKind` in its schema).
fn acp_kind(kind: &str) -> Option<ToolKind> {
    Some(match kind {
        "execute" => ToolKind::Execute,
        "read" => ToolKind::Read,
        "search" => ToolKind::Search,
        "edit" | "delete" | "move" => ToolKind::Edit,
        "fetch" => ToolKind::Fetch,
        _ => return None,
    })
}

/// The kind of a tool by its name, as Claude Code, Codex, Gemini,
/// Antigravity, Copilot and opencode name theirs.
fn kind_of(name: &str) -> ToolKind {
    match name.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "local_shell" | "exec_command" | "run_shell_command" | "terminal" | "run_in_terminal"
        | "run_command" => ToolKind::Execute,
        "read" | "read_file" | "read_many_files" | "view" | "notebookread" | "ls" | "list" | "list_directory" => ToolKind::Read,
        "grep" | "glob" | "search" | "find" | "search_file_content" | "codebase_search" => ToolKind::Search,
        "edit" | "multiedit" | "write" | "write_file" | "replace" | "apply_patch" | "notebookedit" | "str_replace_editor"
        | "create" | "patch" | "edit_file" => ToolKind::Edit,
        "askuserquestion" | "request_user_input" | "ask_question" => ToolKind::Question,
        "webfetch" | "websearch" | "web_fetch" | "google_web_search" | "fetch" | "search_web" => ToolKind::Fetch,
        "task" | "agent" | "invoke_agent" | "spawn_agent" | "subagent" | "invoke_subagent" => ToolKind::Subagent,
        _ => ToolKind::Other,
    }
}

/// A string as given, or a command given as its words (Codex's `shell`).
fn string_of(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Array(words) if !words.is_empty() => {
            let words: Option<Vec<&str>> = words.iter().map(Value::as_str).collect();
            words.map(|words| shell_words(&words))
        }
        _ => None,
    }
}

/// `["bash", "-lc", "cargo test"]` reads as what it runs, `cargo test`.
fn shell_words(words: &[&str]) -> String {
    match words {
        [shell, flag, script] if shell.ends_with("sh") && flag.starts_with('-') && flag.contains('c') => (*script).to_owned(),
        _ => words.join(" "),
    }
}

/// The first question an `AskUserQuestion` call asks.
fn question(input: &Value) -> Option<String> {
    input.get("questions")?.as_array()?.first()?.get("question")?.as_str().map(str::to_owned)
}

/// How a tool call ended, from its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolEnd {
    pub failed: bool,
    pub exit_code: Option<i32>,
}

/// Reads a result: whether the agent marked it an error, and the exit code
/// a command's result names (Claude Code starts a failed command's with
/// `Exit code N`).
pub fn ended(text: &str, is_error: bool) -> ToolEnd {
    let exit_code =
        text.lines().take(3).find_map(|line| line.trim().strip_prefix("Exit code ").and_then(|code| code.trim().parse().ok()));
    ToolEnd { failed: is_error || exit_code.is_some_and(|code| code != 0), exit_code }
}

/// The largest picture a history page carries, base64; a larger one shows
/// as named, not drawn.
const IMAGE_LIMIT: usize = 4 * 1024 * 1024;

/// An entry as a thread's history shows it, but for where it stands: its
/// kind and text, and what a tool call does, how a result ended, the
/// picture or the cost it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub kind: String,
    pub text: String,
    pub tool: Option<ToolView>,
    pub outcome: Option<ToolOutcome>,
    pub image: Option<ImageView>,
    pub usage: Option<UsageView>,
    pub plan: Option<Vec<PlanStepView>>,
    /// The subagent transcript it is from; none for the session's own.
    pub side: Option<String>,
    pub task: Option<TaskView>,
}

impl Shown {
    pub fn plain(kind: &str, text: String) -> Self {
        Self {
            kind: kind.to_owned(),
            text,
            tool: None,
            outcome: None,
            image: None,
            usage: None,
            plan: None,
            side: None,
            task: None,
        }
    }

    /// The history entry at `index` of `thread`.
    pub fn into_entry(self, index: usize, thread: String, origin: &str, time_ms: i64, rewound: bool) -> HistoryEntry {
        HistoryEntry {
            index,
            thread,
            origin: origin.to_owned(),
            kind: self.kind,
            text: self.text,
            time_ms,
            rewound,
            tool: self.tool,
            outcome: self.outcome,
            image: self.image,
            usage: self.usage,
            plan: self.plan,
            side: self.side,
            task: self.task,
        }
    }
}

/// How a history shows `entry`; none for what it leaves out (hooks,
/// settings, snapshots).
pub fn shown(entry: crate::Entry) -> Option<Shown> {
    use crate::Entry;
    Some(match entry {
        Entry::Other { .. } => return None,
        Entry::ToolCall { id, name, input, kind } => {
            let tool = describe(&name, kind.as_deref(), &input);
            let view = ToolView {
                id,
                name: name.clone(),
                kind: tool.kind.as_str().to_owned(),
                subject: tool.subject,
                detail: tool.detail,
                questions: if tool.kind == ToolKind::Question { questions(&input) } else { Vec::new() },
            };
            Shown { tool: Some(view), plan: plan_of(&name, &input), ..Shown::plain("tool_call", format!("{name} {input}")) }
        }
        Entry::ToolResult { tool_use_id, text, is_error } => {
            let end = ended(&text, is_error);
            let outcome =
                ToolOutcome { call: tool_use_id, failed: end.failed, exit_code: end.exit_code, answers: answers(&text) };
            Shown { outcome: Some(outcome), ..Shown::plain("tool_result", text) }
        }
        Entry::Image { media_type, data } => {
            let data = if data.len() > IMAGE_LIMIT { String::new() } else { data };
            Shown { image: Some(ImageView { media_type, data }), ..Shown::plain("image", String::new()) }
        }
        Entry::Usage { message, input, output } => {
            Shown { usage: Some(UsageView { message, input, output }), ..Shown::plain("usage", String::new()) }
        }
        Entry::Plan { steps } => {
            let text = steps.iter().map(|(step, _)| step.as_str()).collect::<Vec<_>>().join("\n");
            let plan = steps.into_iter().map(|(text, status)| PlanStepView { text, status }).collect();
            Shown { plan: Some(plan), ..Shown::plain("plan", text) }
        }
        Entry::Compaction { summary } => {
            Shown::plain("compaction", summary.unwrap_or_else(|| "(context compacted; everything above is still here)".into()))
        }
        Entry::Woke { text } => Shown::plain("wake", text),
        Entry::Task(task) => {
            let view = TaskView {
                id: task.id,
                call: task.call,
                kind: task.kind.map(|kind| kind.as_str().to_owned()),
                status: task.status.as_str().to_owned(),
                side: task.side,
                summary: task.summary,
            };
            let text = view.summary.clone().unwrap_or_else(|| format!("{} {}", view.id, view.status));
            Shown { task: Some(view), ..Shown::plain("task", text) }
        }
        other => {
            let kind = other.kind().to_owned();
            Shown::plain(&kind, other.into_searchable_text().unwrap_or_default())
        }
    })
}

/// The plan a planning tool writes: Claude Code's `TodoWrite`, Codex's
/// `update_plan` and Gemini's `write_todos`.
fn plan_of(name: &str, input: &Value) -> Option<Vec<PlanStepView>> {
    let steps = match name {
        "TodoWrite" | "write_todos" | "todowrite" => input.get("todos"),
        "update_plan" => input.get("plan"),
        _ => None,
    }?;
    let text =
        |step: &Value, keys: &[&str]| keys.iter().find_map(|key| step.get(*key).and_then(Value::as_str)).map(str::to_owned);
    Some(
        steps
            .as_array()?
            .iter()
            .filter_map(|step| {
                Some(PlanStepView {
                    text: text(step, &["content", "step", "description"])?,
                    status: text(step, &["status"]).unwrap_or_else(|| "pending".to_owned()),
                })
            })
            .collect(),
    )
}

/// The questions an `AskUserQuestion` (or Codex `request_user_input`) call asks.
fn questions(input: &Value) -> Vec<QuestionView> {
    let Some(asked) = input.get("questions").and_then(Value::as_array) else {
        return Vec::new();
    };
    let text =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).filter(|text| !text.is_empty()).map(str::to_owned);
    asked
        .iter()
        .filter_map(|question| {
            Some(QuestionView {
                header: text(question, "header"),
                question: text(question, "question")?,
                multi_select: question
                    .get("multiSelect")
                    .or_else(|| question.get("multi_select"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                options: question
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(ChoiceView { label: text(option, "label")?, description: text(option, "description") })
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The answers a question tool's result reports, as Claude Code words it:
/// `User has answered your questions: "Q"="A", "R"="B". ...`.
fn answers(text: &str) -> Vec<AnswerView> {
    let Some(rest) = text.strip_prefix("User has answered your questions: ") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut rest = rest;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(split) = after.find("\"=\"") else {
            break;
        };
        let question = &after[..split];
        let tail = &after[split + 3..];
        let Some(close) = tail.find('"') else {
            break;
        };
        found.push(AnswerView { question: question.to_owned(), answer: tail[..close].to_owned() });
        rest = &tail[close + 1..];
    }
    found
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn each_agents_tools_read_alike() {
        let bash = describe("Bash", None, &json!({"command": "cargo test upload", "description": "Run the upload tests"}));
        assert_eq!(
            bash,
            ToolUse {
                kind: ToolKind::Execute,
                subject: Some("cargo test upload".into()),
                detail: Some("Run the upload tests".into())
            }
        );
        let codex = describe("shell", None, &json!({"command": ["bash", "-lc", "cargo test"]}));
        assert_eq!(codex.subject.as_deref(), Some("cargo test"));
        let grep = describe("Grep", None, &json!({"pattern": "progress", "path": "src"}));
        assert_eq!(
            (grep.kind, grep.subject.as_deref(), grep.detail.as_deref()),
            (ToolKind::Search, Some("progress"), Some("src"))
        );
        let gemini = describe("read_file", None, &json!({"absolute_path": "/w/src/upload.rs"}));
        assert_eq!((gemini.kind, gemini.subject.as_deref()), (ToolKind::Read, Some("/w/src/upload.rs")));
        let acp = describe("Edit src/main.rs", Some("edit"), &json!({"path": "src/main.rs"}));
        assert_eq!((acp.kind, acp.subject.as_deref()), (ToolKind::Edit, Some("src/main.rs")));
        let asked = describe("AskUserQuestion", None, &json!({"questions": [{"question": "Which one?", "options": []}]}));
        assert_eq!((asked.kind, asked.subject.as_deref()), (ToolKind::Question, Some("Which one?")));
        assert_eq!(describe("TodoWrite", None, &json!({})).kind, ToolKind::Other);
        let copilot = describe(
            "Count README lines",
            Some("other"),
            &json!({"name": "readme-lines", "agent_type": "explore", "description": "Count README lines", "prompt": "How many?"}),
        );
        assert_eq!(
            (copilot.kind, copilot.subject.as_deref(), copilot.detail.as_deref()),
            (ToolKind::Subagent, Some("Count README lines"), Some("explore"))
        );
    }

    #[test]
    fn a_question_reads_with_its_choices_and_its_answer() {
        let input = json!({"questions": [{"header": "Progress", "question": "What should it count?", "multiSelect": false,
            "options": [{"label": "Bytes acknowledged", "description": "Accurate"}, {"label": "Chunks"}]}]});
        let asked = questions(&input);
        assert_eq!(asked[0].header.as_deref(), Some("Progress"));
        assert_eq!(asked[0].options[1], ChoiceView { label: "Chunks".into(), description: None });
        let said =
            answers("User has answered your questions: \"What should it count?\"=\"Bytes acknowledged\". You can now continue.");
        assert_eq!(said, vec![AnswerView { question: "What should it count?".into(), answer: "Bytes acknowledged".into() }]);
    }

    #[test]
    fn each_agents_plan_reads_as_steps() {
        let claude = json!({"todos": [{"content": "Find it", "status": "completed", "activeForm": "Finding it"}, {"content": "Fix it", "status": "in_progress"}]});
        let codex = json!({"plan": [{"step": "Fix it", "status": "pending"}]});
        assert_eq!(
            plan_of("TodoWrite", &claude).unwrap()[1],
            PlanStepView { text: "Fix it".into(), status: "in_progress".into() }
        );
        assert_eq!(plan_of("update_plan", &codex).unwrap()[0].status, "pending");
        assert_eq!(plan_of("Read", &claude), None);
        let read = crate::acp::read_line(&crate::acp::plan(&[("Fix it".into(), "completed".into())]));
        let shown = shown(read.entries.into_iter().next().unwrap()).unwrap();
        assert_eq!((shown.kind.as_str(), shown.plan.unwrap()[0].status.as_str()), ("plan", "completed"));
        let earlier = crate::acp::read_line(r#"{"t":"plan","entries":["Fix it"]}"#);
        assert_eq!(earlier.entries, [crate::Entry::Plan { steps: vec![("Fix it".into(), "pending".into())] }]);
    }

    #[test]
    fn a_result_says_whether_it_failed_and_with_what_code() {
        assert_eq!(ended("Exit code 101\nrunning 2 tests", true), ToolEnd { failed: true, exit_code: Some(101) });
        assert_eq!(ended("ok", false), ToolEnd { failed: false, exit_code: None });
        assert_eq!(ended("permission denied", true), ToolEnd { failed: true, exit_code: None });
    }
}
