//! `silt mcp`: the history tools of ADR 0011, answered by the daemon.

use std::env;
use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;

use exn::{Exn, ResultExt};
use serde::de::DeserializeOwned;
use serde_json::Value;
use silt_agents::conventions::{self, Skill};
use silt_daemon::protocol::{
    ArtifactGuide, ArtifactPut, DesignSource, FileTouch, HistoryHit, HistoryPage, InFlight, NewArtifact, Request, Response,
    SearchResult, TaskView,
};
use silt_daemon::{client, paths};

use crate::CliError;
use crate::render::short;

const DEFAULT_LIMIT: usize = 40;
/// Longest text of one history entry an agent reads.
const ENTRY_CHARS: usize = 4000;
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

struct DaemonTools {
    thread: String,
    cwd: PathBuf,
    checkout: PathBuf,
    /// The run's token, which scopes every answer to its workspace (ADR 0065).
    token: Option<String>,
    /// A person watches the run in the web UI (ADR 0068).
    shows: bool,
}

pub fn serve(thread: &str, checkout: Option<PathBuf>, token: Option<String>, shows: bool) -> Result<(), Exn<CliError>> {
    let cwd = env::current_dir().or_raise(|| CliError("cannot read the current directory".into()))?;
    let token = token.or_else(|| env::var("SILT_RUN_TOKEN").ok()).filter(|token| !token.is_empty());
    let mut tools =
        DaemonTools { thread: thread.to_owned(), checkout: checkout.unwrap_or_else(|| cwd.clone()), cwd, token, shows };
    silt_mcp::serve(io::stdin().lock(), io::stdout().lock(), &mut tools).or_raise(|| CliError("the MCP connection failed".into()))
}

fn ask_daemon<T: DeserializeOwned>(request: &Request) -> Result<T, String> {
    match client::send(&paths::socket_path(), request, Some(REQUEST_TIMEOUT)) {
        Ok(Response::Ok { data }) => serde_json::from_value(data).map_err(|error| error.to_string()),
        Ok(Response::Error { message }) => Err(message),
        Err(error) => Err(format!("the Silt daemon is not answering: {error}")),
    }
}

/// A file's name without its extension, as a title for an image shown
/// without a caption.
fn file_title(path: &str) -> String {
    std::path::Path::new(path).file_stem().map_or_else(|| "Image".to_owned(), |stem| stem.to_string_lossy().into_owned())
}

fn text_arg(arguments: &Value, name: &str) -> Result<String, String> {
    arguments.get(name).and_then(Value::as_str).map(str::to_owned).ok_or_else(|| format!("missing argument {name}"))
}

fn number_arg(arguments: &Value, name: &str) -> Option<usize> {
    arguments.get(name).and_then(Value::as_u64).and_then(|value| usize::try_from(value).ok())
}

impl DaemonTools {
    fn ask<T: DeserializeOwned>(&self, request: Request) -> Result<T, String> {
        match &self.token {
            Some(token) => ask_daemon(&Request::Scoped { token: token.clone(), asking: Box::new(request) }),
            None => ask_daemon(&request),
        }
    }

    /// Asks the daemon to keep what `silt_show` or `silt_artifact` shows.
    fn show(&self, artifact: NewArtifact) -> Result<ArtifactPut, String> {
        let token = self.token.clone().ok_or("only an agent the web UI runs can show the person something")?;
        ask_daemon(&Request::Scoped { token, asking: Box::new(Request::PutArtifact { artifact }) })
    }

    /// `path` as the agent meant it: relative to where it runs.
    fn path_arg(&self, arguments: &Value) -> Option<String> {
        let path = arguments.get("path").and_then(Value::as_str)?;
        Some(self.cwd.join(path).to_string_lossy().into_owned())
    }

    /// The repository's and the person's skills, and Silt's own for a run
    /// the web UI shows (ADR 0070).
    fn skills(&self) -> Vec<Skill> {
        let home = silt_os::home_dir();
        let builtin = self.shows.then(|| silt_agents::builtin::builtin_skills_root(&paths::data_dir()).ok()).flatten();
        conventions::discover_skills_with(&self.checkout, home.as_deref(), builtin.as_deref())
    }

    fn skill(&self, name: &str) -> Result<Skill, String> {
        self.skills()
            .into_iter()
            .find(|skill| skill.name == name)
            .ok_or_else(|| format!("no skill named {name}; silt_skills lists them"))
    }
}

impl silt_mcp::Backend for DaemonTools {
    fn shows(&self) -> bool {
        self.shows
    }

    fn prompts(&mut self) -> Vec<silt_mcp::Prompt> {
        self.skills()
            .into_iter()
            .filter(|skill| skill.user_invocable)
            .map(|skill| silt_mcp::Prompt { name: skill.name, description: skill.description })
            .collect()
    }

    fn prompt(&mut self, name: &str) -> Result<String, String> {
        let skill = self.skill(name)?;
        let body = skill.body().map_err(|error| error.to_string())?;
        Ok(format!("Use the {name} skill (its files are in {}):\n\n{body}", skill.dir.display()))
    }

    fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String> {
        match tool {
            "silt_skills" => {
                let skills: Vec<Skill> = self.skills().into_iter().filter(|skill| skill.model_invocable).collect();
                if skills.is_empty() {
                    return Ok("No skills in .silt/skills, .agents/skills or ~/.agents/skills.".into());
                }
                let mut out = String::from("Skills (load one with silt_use_skill):\n");
                for skill in skills {
                    let _ = writeln!(out, "- {}: {}", skill.name, skill.description);
                }
                return Ok(out);
            }
            "silt_use_skill" => {
                let skill = self.skill(&text_arg(arguments, "name")?)?;
                let body = skill.body().map_err(|error| error.to_string())?;
                let mut files: Vec<String> = std::fs::read_dir(&skill.dir)
                    .map(|entries| {
                        entries
                            .flatten()
                            .map(|entry| entry.file_name().to_string_lossy().into_owned())
                            .filter(|name| name != "SKILL.md")
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                let mut out = format!("Skill {} from {}.", skill.name, skill.dir.display());
                if !files.is_empty() {
                    let _ = write!(out, " Its other files: {}.", files.join(", "));
                }
                let _ = write!(out, "\n\n{body}");
                return Ok(out);
            }
            "silt_show" => {
                let path = self.path_arg(arguments).ok_or("missing argument path")?;
                let caption = arguments.get("caption").and_then(Value::as_str).map(str::to_owned);
                let title = caption.clone().unwrap_or_else(|| file_title(&path));
                let artifact = NewArtifact {
                    title,
                    kind: "image".into(),
                    content: None,
                    path: Some(path),
                    id: None,
                    caption,
                    inline: true,
                    design_system: None,
                };
                self.show(artifact)?;
                return Ok("Shown to the person in the conversation.".into());
            }
            "silt_artifact" => {
                let artifact = NewArtifact {
                    title: text_arg(arguments, "title")?,
                    kind: text_arg(arguments, "kind")?,
                    content: arguments.get("content").and_then(Value::as_str).map(str::to_owned),
                    path: self.path_arg(arguments),
                    id: arguments.get("id").and_then(Value::as_str).map(str::to_owned),
                    caption: None,
                    inline: false,
                    design_system: arguments.get("design_system").and_then(Value::as_str).map(str::to_owned),
                };
                let kept = self.show(artifact)?;
                let mut out = format!(
                    "Artifact {} version {} is beside the conversation. For a new version, call silt_artifact again with id {}.",
                    kept.id, kept.version, kept.id
                );
                if let Some(system) = &kept.design_system {
                    let _ = write!(out, " Its frame has the {system} design system's tokens.");
                }
                if !kept.dropped.is_empty() {
                    let _ = write!(out, "\nThe design system's tokens left out: {}.", kept.dropped.join("; "));
                }
                return Ok(out);
            }
            "silt_artifact_guide" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs makes artifacts")?;
                let kind = text_arg(arguments, "kind")?;
                let asked = arguments.get("design_system").and_then(Value::as_str).map(str::to_owned);
                let guide: ArtifactGuide =
                    ask_daemon(&Request::Scoped { token, asking: Box::new(Request::ArtifactGuide { design_system: asked }) })?;
                return Ok(guide_text(&kind, &guide, &self.skills()));
            }
            _ => {}
        }
        let thread = arguments.get("thread").and_then(Value::as_str).map_or_else(|| self.thread.clone(), str::to_owned);
        match tool {
            "silt_history_read" | "silt_fork_read" => {
                let page: HistoryPage = self.ask(Request::HistoryRead {
                    thread,
                    offset: number_arg(arguments, "offset").unwrap_or(0),
                    limit: number_arg(arguments, "limit").unwrap_or(DEFAULT_LIMIT),
                })?;
                Ok(render_page(&page))
            }
            "silt_history_search" => {
                let hits: Vec<HistoryHit> = self.ask(Request::HistorySearch {
                    thread,
                    query: text_arg(arguments, "query")?,
                    limit: u32::try_from(number_arg(arguments, "limit").unwrap_or(10)).unwrap_or(10),
                })?;
                if hits.is_empty() {
                    return Ok("No matches in this thread's history.".into());
                }
                let mut out = String::from("Matches (read around one with silt_history_read, offset a few entries before it):\n");
                for hit in hits {
                    let _ = writeln!(out, "[{}] {}: {}", hit.index, hit.kind, hit.snippet);
                }
                Ok(out)
            }
            "silt_file_history" => {
                let touches: Vec<FileTouch> = self.ask(Request::FileHistory { thread, path: text_arg(arguments, "path")? })?;
                if touches.is_empty() {
                    return Ok("No recorded changes to this file in this thread.".into());
                }
                let mut out = String::new();
                for touch in touches {
                    let who = match (&touch.tool_use_id, touch.by_agent) {
                        (Some(tool), _) => format!("tool call {tool}"),
                        (None, true) => "the agent".to_owned(),
                        (None, false) => "edits outside tool calls".to_owned(),
                    };
                    let prompt = touch.prompt.as_deref().map(clip).unwrap_or("(before any prompt)".into());
                    let _ = writeln!(
                        out,
                        "thread {}: {who}, +{} -{} chars, in the turn for: {prompt}",
                        short(&touch.thread),
                        touch.inserted,
                        touch.deleted
                    );
                }
                Ok(out)
            }
            "silt_thread_search" => {
                let hits: Vec<SearchResult> = self.ask(Request::Search {
                    query: text_arg(arguments, "query")?,
                    limit: u32::try_from(number_arg(arguments, "limit").unwrap_or(20)).unwrap_or(20),
                    filters: Default::default(),
                })?;
                if hits.is_empty() {
                    return Ok("No matches.".into());
                }
                let mut out = String::from("Matches (read a thread with silt_fork_read):\n");
                for hit in hits {
                    let _ = writeln!(out, "thread {} {}: {}", hit.thread, hit.kind, hit.snippet);
                }
                Ok(out)
            }
            "silt_in_flight" => {
                let found: Vec<InFlight> =
                    self.ask(Request::InFlight { cwd: self.cwd.clone(), path: text_arg(arguments, "path")? })?;
                if found.is_empty() {
                    return Ok("No other thread is changing this path.".into());
                }
                let mut out = String::new();
                for item in found {
                    let owner = if item.owner.is_empty() { "you" } else { &item.owner };
                    let _ = writeln!(out, "thread {} ({owner}): {}", item.thread, item.files.join(", "));
                }
                Ok(out)
            }
            other => Err(format!("unknown tool {other}")),
        }
    }
}

/// What `silt_artifact_guide` answers: the design guidance for `kind`, the
/// design system that applies and the others there are (ADR 0070).
fn guide_text(kind: &str, guide: &ArtifactGuide, skills: &[Skill]) -> String {
    let body = |name: &str| skills.iter().find(|skill| skill.name == name).and_then(|skill| skill.body().ok());
    let mut out = String::new();
    match kind {
        "html" => match body("artifact-design") {
            Some(text) => out.push_str(&text),
            None => out.push_str("Write a whole HTML document, and design it for both light and dark themes and every width.\n"),
        },
        "svg" => out.push_str(
            "An SVG artifact shows as an image: nothing in it runs and it gets no design system's tokens, so give it its \
own colours, readable in light and dark (a `prefers-color-scheme` rule inside the SVG works). For a chart, load the \
artifact-dataviz skill with silt_use_skill; for a diagram, artifact-diagramming.\n",
        ),
        "mermaid" => out.push_str(
            "Silt renders Mermaid in its own theme. Keep node labels short, label edges, and use the codebase's real \
names. The artifact-diagramming skill says what a diagram should show.\n",
        ),
        "markdown" => out.push_str(
            "Silt renders Markdown in its own styles, with no raw HTML. Lead with the point, use headings a reader can \
scan, and tables for comparisons.\n",
        ),
        _ => out.push_str("An image shows as it is; give it a short title.\n"),
    }
    if kind != "html" {
        return out;
    }
    let source = |source: DesignSource| match source {
        DesignSource::Project => "the repository's",
        DesignSource::Personal => "the person's own",
        DesignSource::Builtin => "built into Silt",
    };
    out.push_str("\n# The design system\n\n");
    match &guide.system {
        Some(system) => {
            let _ = writeln!(
                out,
                "This artifact uses {} (`{}`, {}), {}. Silt gives its frame these tokens as CSS custom properties for both \
themes and sets `data-theme` from the viewer, so use them with a fallback, as in `var(--bg, #fff)`. Pass \
`design_system` to silt_artifact to use another, or `none`.\n",
                system.title,
                system.name,
                source(system.source),
                guide.why.as_deref().unwrap_or("asked for")
            );
            if let Some(readme) = &system.readme {
                let _ = writeln!(out, "## Its rules\n\n{}\n", readme.trim());
            }
            if !system.tokens.is_empty() {
                out.push_str("## Its tokens\n\n");
                for (family, names) in &system.tokens {
                    let _ = writeln!(out, "- {family}: {}", names.join(", "));
                }
                out.push('\n');
            }
        }
        None => out.push_str(
            "None applies: the project names no default in `.silt/design-systems/default` and its workspace none. Work \
from a small token set of your own, or pass `design_system` to silt_artifact with one of those below.\n\n",
        ),
    }
    let others: Vec<String> = guide
        .systems
        .iter()
        .filter(|system| guide.system.as_ref().is_none_or(|chosen| chosen.name != system.name))
        .map(|system| format!("`{}` ({}, {})", system.name, system.title, source(system.source)))
        .collect();
    if !others.is_empty() {
        let _ =
            writeln!(out, "Other design systems: {}. silt_artifact_guide with `design_system` reads one.\n", others.join(", "));
    }
    out.push_str(
        "For a chart, also load the artifact-dataviz skill with silt_use_skill; for a diagram, artifact-diagramming; to \
write a design system for this project, design-system.\n",
    );
    out
}

fn render_page(page: &HistoryPage) -> String {
    let mut out = format!("Thread {} has {} history entries.", page.thread, page.total);
    match (page.entries.first(), page.entries.last()) {
        (Some(first), Some(last)) => {
            let _ = writeln!(out, " Entries {} to {}:", first.index, last.index);
        }
        _ => out.push_str(" No entries in this range.\n"),
    }
    for entry in &page.entries {
        let origin = match entry.origin.as_str() {
            "own" => String::new(),
            other => format!(" ({other} from {})", short(&entry.thread)),
        };
        // An agent must not take a rewound turn as part of the conversation (ADR 0029).
        let rewound = if entry.rewound { " (rewound by a revert, no longer part of the conversation)" } else { "" };
        let side = entry.side.as_deref().map(|side| format!(" (in subagent {side})")).unwrap_or_default();
        let text = match &entry.task {
            Some(task) => task_line(task),
            None => clipped(entry.text.trim()),
        };
        let _ = writeln!(out, "[{}] {}{origin}{side}{rewound}: {text}", entry.index, entry.kind);
    }
    out
}

/// An entry's text as an agent reads it: one long entry mustn't crowd the
/// rest of its page out of the tool's bounded answer.
fn clipped(text: &str) -> String {
    match text.char_indices().nth(ENTRY_CHARS) {
        Some((end, _)) => format!("{} …", &text[..end]),
        None => text.to_owned(),
    }
}

/// News of a subagent or background command, as one line.
fn task_line(task: &TaskView) -> String {
    let what = task.kind.as_deref().unwrap_or("task");
    let call = task.call.as_deref().map(|call| format!(", started by tool call {call}")).unwrap_or_default();
    let side = task.side.as_deref().map(|side| format!(", its entries marked subagent {side}")).unwrap_or_default();
    let summary = task.summary.as_deref().map(|summary| format!(": {}", summary.trim())).unwrap_or_default();
    format!("{what} {} {}{call}{side}{summary}", task.id, task.status)
}

fn clip(text: &str) -> String {
    let text = text.trim().replace('\n', " ");
    if text.chars().count() > 160 { text.chars().take(160).collect::<String>() + "…" } else { text }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silt_daemon::protocol::DesignSystemView;

    fn system(name: &str, source: DesignSource, readme: Option<&str>) -> DesignSystemView {
        DesignSystemView {
            name: name.into(),
            title: name.to_uppercase(),
            source,
            readme: readme.map(str::to_owned),
            tokens: [("color".to_owned(), vec!["--bg".to_owned(), "--fg-1".to_owned()])].into(),
        }
    }

    #[test]
    fn the_guide_gives_the_design_skill_the_system_that_applies_and_the_others() {
        let data = tempfile::tempdir().expect("temp dir");
        let root = silt_agents::builtin::builtin_skills_root(data.path()).expect("skills");
        let checkout = tempfile::tempdir().expect("temp dir");
        let skills = conventions::discover_skills_with(checkout.path(), None, Some(&root));
        let guide = ArtifactGuide {
            system: Some(system("nous", DesignSource::Builtin, Some("Use `accent` sparingly."))),
            why: Some("the project's default".into()),
            systems: vec![system("nous", DesignSource::Builtin, None), system("acme", DesignSource::Project, None)],
        };
        let text = guide_text("html", &guide, &skills);
        assert!(text.starts_with("# Making an artifact"), "{text}");
        assert!(text.contains("This artifact uses NOUS (`nous`, built into Silt), the project's default."), "{text}");
        assert!(text.contains("Use `accent` sparingly.") && text.contains("- color: --bg, --fg-1"), "{text}");
        assert!(text.contains("Other design systems: `acme` (ACME, the repository's)."), "{text}");

        let none = ArtifactGuide { system: None, why: None, systems: guide.systems.clone() };
        assert!(guide_text("html", &none, &skills).contains("None applies"));
        let svg = guide_text("svg", &guide, &skills);
        assert!(svg.contains("artifact-dataviz") && !svg.contains("Other design systems"), "{svg}");
    }
}
