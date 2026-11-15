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
    ArtifactGuide, ArtifactPut, ArtifactRead, DesignSource, FileTouch, HistoryHit, HistoryPage, InFlight, NewArtifact, Request,
    Response, SearchResult, TaskView,
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
        ask_daemon(&Request::Scoped { token, asking: Box::new(Request::PutArtifact { artifact: Box::new(artifact) }) })
    }

    /// `path` as the agent meant it: relative to where it runs.
    fn path_arg(&self, arguments: &Value) -> Option<String> {
        let path = arguments.get("path").and_then(Value::as_str)?;
        Some(self.cwd.join(path).to_string_lossy().into_owned())
    }

    /// The repository's and the person's skills, and Silt's own for a run
    /// the web UI shows (ADR 0071).
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
                let mut files = skill_files(&skill.dir);
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
                    edits: Vec::new(),
                    note: None,
                    description: None,
                    capabilities: None,
                    files: Default::default(),
                    patch: None,
                };
                self.show(artifact)?;
                return Ok("Shown to the person in the conversation.".into());
            }
            "silt_artifact" => {
                let id = arguments.get("id").and_then(Value::as_str).map(str::to_owned);
                let text = |name: &str| arguments.get(name).and_then(Value::as_str).map(str::to_owned);
                let (title, kind) = match (&id, text("title"), text("kind")) {
                    (None, None, _) => return Err("missing argument title".into()),
                    (None, _, None) => return Err("missing argument kind".into()),
                    (_, title, kind) => (title.unwrap_or_default(), kind.unwrap_or_default()),
                };
                let artifact = NewArtifact {
                    title,
                    kind,
                    content: text("content"),
                    path: self.path_arg(arguments),
                    id,
                    caption: None,
                    inline: false,
                    design_system: text("design_system"),
                    edits: edits_arg(arguments)?,
                    note: text("note"),
                    description: text("description"),
                    capabilities: match arguments.get("capabilities") {
                        None | Some(Value::Null) => None,
                        Some(Value::Object(declared)) => Some(declared.clone()),
                        Some(_) => return Err("capabilities is an object of names and configs".into()),
                    },
                    files: self.files_arg(arguments)?,
                    patch: arguments.get("patch").filter(|patch| !patch.is_null()).cloned(),
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
            "silt_artifact_data" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs reaches its artifacts' data")?;
                let artifact = text_arg(arguments, "id")?;
                let mut asked = arguments.clone();
                if let Some(object) = asked.as_object_mut() {
                    object.remove("id");
                }
                let request: silt_proto::page::PageRequest =
                    serde_json::from_value(asked).map_err(|error| format!("that isn't a request of page data: {error}"))?;
                let answer: Value = ask_daemon(&Request::Scoped {
                    token,
                    asking: Box::new(Request::PageData { thread: String::new(), artifact, asked: request }),
                })?;
                return Ok(serde_json::to_string_pretty(&answer).unwrap_or_default());
            }
            "silt_artifact_room" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs speaks in a room")?;
                let artifact = text_arg(arguments, "id")?;
                let emit = arguments
                    .get("topic")
                    .and_then(Value::as_str)
                    .map(|topic| (topic.to_owned(), arguments.get("data").cloned().unwrap_or(Value::Null)));
                let heard: Value = ask_daemon(&Request::Scoped {
                    token,
                    asking: Box::new(Request::RoomAgent { thread: String::new(), artifact, emit }),
                })?;
                return Ok(serde_json::to_string_pretty(&heard).unwrap_or_default());
            }
            "silt_artifact_doc" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs edits its docs")?;
                let artifact = text_arg(arguments, "id")?;
                let asked = match arguments.get("edits") {
                    None | Some(Value::Null) => silt_proto::edited::DocRequest::Read,
                    Some(edits) => silt_proto::edited::DocRequest::Edit {
                        edits: serde_json::from_value(edits.clone())
                            .map_err(|error| format!("edits are {{old, new}}: {error}"))?,
                    },
                };
                let answer: Value = ask_daemon(&Request::Scoped {
                    token,
                    asking: Box::new(Request::DocData { thread: String::new(), artifact, asked }),
                })?;
                return Ok(answer["text"].as_str().unwrap_or_default().to_owned());
            }
            "silt_artifact_sheet" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs edits its sheets")?;
                let artifact = text_arg(arguments, "id")?;
                let ask = |asked| {
                    ask_daemon::<Value>(&Request::Scoped {
                        token: token.clone(),
                        asking: Box::new(Request::SheetData { thread: String::new(), artifact: artifact.clone(), asked }),
                    })
                };
                if let Some(set) = arguments.get("set").and_then(Value::as_object) {
                    let cells = set
                        .iter()
                        .map(|(address, raw)| (address.clone(), raw.as_str().map_or_else(|| raw.to_string(), str::to_owned)))
                        .collect();
                    ask(silt_proto::edited::SheetRequest::Set { sheet: 0, cells })?;
                }
                let range = arguments.get("read").and_then(Value::as_str).map(str::to_owned);
                let read = ask(silt_proto::edited::SheetRequest::Read { sheet: 0, range: range.clone() })?;
                return Ok(match range {
                    Some(_) => serde_json::to_string(&read["values"]).unwrap_or_default(),
                    None => sheet_text(&read),
                });
            }
            "silt_artifact_read" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs reads its artifacts back")?;
                let id = text_arg(arguments, "id")?;
                let version = arguments.get("version").and_then(Value::as_u64).and_then(|version| u32::try_from(version).ok());
                let read: ArtifactRead =
                    ask_daemon(&Request::Scoped { token, asking: Box::new(Request::ArtifactRead { id, version }) })?;
                return read_back(&self.thread, &read);
            }
            "silt_artifact_check" => {
                let token = self.token.clone().ok_or("only an agent the web UI runs checks its artifacts")?;
                let id = text_arg(arguments, "id")?;
                let version = arguments.get("version").and_then(Value::as_u64).and_then(|version| u32::try_from(version).ok());
                let read: ArtifactRead =
                    ask_daemon(&Request::Scoped { token, asking: Box::new(Request::ArtifactRead { id, version }) })?;
                let dir = env::temp_dir()
                    .join("silt-artifacts")
                    .join(short(&self.thread))
                    .join(format!("check-{}-v{}", read.id, read.version));
                return crate::artifact_check::check(&read, &self.checkout, &dir);
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

/// The longest source `silt_artifact_read` returns whole, beside its file.
const READ_INLINE_BYTES: usize = 64 << 10;

/// Writes a version read back to a file of its own outside the checkout,
/// and says where, with its source when it's short.
fn read_back(thread: &str, read: &ArtifactRead) -> Result<String, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&read.data).map_err(|error| error.to_string())?;
    let extension = match read.kind.as_str() {
        "html" => "html",
        "svg" => "svg",
        "markdown" => "md",
        "mermaid" => "mmd",
        _ => read.media_type.rsplit('/').next().filter(|ext| ext.bytes().all(|b| b.is_ascii_alphanumeric())).unwrap_or("bin"),
    };
    // The id is checked by the daemon; the thread's short form names the folder.
    let dir = env::temp_dir().join("silt-artifacts").join(short(thread));
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("{}-v{}.{extension}", read.id, read.version));
    std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
    let mut out = format!(
        "Artifact {} \"{}\" ({}), version {} of {}, is at {}.",
        read.id,
        read.title,
        read.kind,
        read.version,
        read.latest,
        path.display()
    );
    if let Some(system) = &read.design_system {
        let _ = write!(out, " It was styled with the {system} design system.");
    }
    let _ = write!(out, " Edit that file and pass its path to silt_artifact with id {} for a new version.", read.id);
    match std::str::from_utf8(&bytes) {
        Ok(text) if text.len() <= READ_INLINE_BYTES => {
            let _ = write!(out, "\n\n{text}");
        }
        _ => {}
    }
    Ok(out)
}

/// What `silt_artifact_guide` answers: the design guidance for `kind`, the
/// design system that applies and the others there are (ADR 0071).
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
        kind if silt_proto::artifact_types::is_type(kind) => {
            if let Some(text) = body(&format!("artifact-{kind}")) {
                out.push_str(&text);
            }
            if let Some((schema, example)) = silt_proto::artifact_types::guide(kind) {
                let pretty = |value: &Value| serde_json::to_string_pretty(value).unwrap_or_default();
                let _ = write!(
                    out,
                    "\n# The content's schema\n\nGive `content` as JSON that fits this schema; a version that doesn't is refused \
with why. Revise one with `patch`, an RFC 6902 JSON Patch, rather than resending it.\n\n```json\n{}\n```\n\n# An example\n\n```json\n{}\n```\n",
                    pretty(&schema),
                    pretty(&example)
                );
            }
        }
        "doc" => out.push_str(
            "A doc is Markdown that people edit with you, in place. Give its first text as `content`; after that, change \
it with silt_artifact_doc's edits, which merge with what people type at the same moment, never with a new version. \
Read it first, since people may have changed it. Lead with the point, use headings a reader can scan, and keep raw \
HTML out: it shows as text.\n",
        ),
        "sheet" => out.push_str(
            "A sheet is a grid with formulas that people edit with you. Give its first cells as `content`, \
{\"rows\": [[\"Task\", \"Hours\"], [\"Design\", 3]]}, header row first; after that, write cells with \
silt_artifact_sheet. Use formulas (=SUM(B2:B9)) rather than computed numbers, so the sheet stays right as people \
change it, and keep one table per sheet with one header row.\n",
        ),
        "whiteboard" => out.push_str(
            "A whiteboard is notes, arrows, sections and strokes people arrange together, live. Give its first items as \
`content`: {\"items\": [{\"id\": \"goal\", \"type\": \"block\", \"x\": 0, \"y\": 0, \"w\": 160, \"h\": 100, \"text\": \"Ship M30\", \
\"color\": \"yellow\"}, {\"id\": \"a1\", \"type\": \"arrow\", \"from\": \"goal\", \"to\": \"risk\", \"label\": \"blocked by\"}]}. Types: block \
(a note; colour yellow, blue, green, pink or gray), arrow (from and to item ids, an optional label), section (a labelled \
frame, x, y, w, h, text), stroke (points). After that, add or move items with silt_artifact_data at items/<id>, \
and say something to the people there with silt_artifact_room. Lay items on a grid, 40 px apart, one idea per note.\n",
        ),
        _ => out.push_str("An image shows as it is; give it a short title.\n"),
    }
    if !matches!(kind, "html" | "slides" | "canvas") {
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

/// The files of a skill's folder other than `SKILL.md`, by their paths in
/// it, one folder deep, as a skill's `references/` holds them.
fn skill_files(dir: &std::path::Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            for inner in std::fs::read_dir(entry.path()).into_iter().flatten().flatten() {
                if inner.file_type().is_ok_and(|kind| kind.is_file()) {
                    files.push(format!("{name}/{}", inner.file_name().to_string_lossy()));
                }
            }
        } else if name != "SKILL.md" {
            files.push(name);
        }
    }
    files
}

impl DaemonTools {
    /// `files`, as `{path: {content} | {base64} | {path} | null}`, a
    /// relative `path` read from the working directory as `path` is.
    fn files_arg(
        &self,
        arguments: &Value,
    ) -> Result<std::collections::BTreeMap<String, Option<silt_daemon::protocol::NewFile>>, String> {
        let Some(files) = arguments.get("files") else {
            return Ok(Default::default());
        };
        let files = files.as_object().ok_or("files is an object of paths and files")?;
        files
            .iter()
            .map(|(path, file)| {
                if file.is_null() {
                    return Ok((path.clone(), None));
                }
                let text = |name: &str| file.get(name).and_then(Value::as_str).map(str::to_owned);
                let from = self.path_arg(file);
                Ok((
                    path.clone(),
                    Some(silt_daemon::protocol::NewFile { content: text("content"), base64: text("base64"), path: from }),
                ))
            })
            .collect()
    }
}

/// A sheet as the agent reads it: each non-empty cell by address, what it
/// holds and, for a formula, its value.
fn sheet_text(view: &Value) -> String {
    let mut out = String::new();
    let rows = view["raw"].as_array().cloned().unwrap_or_default();
    for (row, cells) in rows.iter().enumerate() {
        for (column, raw) in cells.as_array().into_iter().flatten().enumerate() {
            let raw = raw.as_str().unwrap_or_default();
            if raw.is_empty() {
                continue;
            }
            let value = view["values"][row][column].as_str().unwrap_or_default();
            let address = silt_sheet::address(row, column);
            if raw.starts_with('=') {
                let _ = writeln!(out, "{address}: {raw} → {value}");
            } else {
                let _ = writeln!(out, "{address}: {raw}");
            }
        }
    }
    if out.is_empty() { "The sheet is empty.".into() } else { out }
}

/// `edits`, checked here as the MCP server checks only numbers: a list of
/// `{old, new}` strings.
fn edits_arg(arguments: &Value) -> Result<Vec<silt_proto::artifact::Edit>, String> {
    let Some(edits) = arguments.get("edits") else {
        return Ok(Vec::new());
    };
    let edits = edits.as_array().ok_or("edits is a list of {old, new}")?;
    edits
        .iter()
        .enumerate()
        .map(|(at, edit)| {
            let field = |name: &str| edit.get(name).and_then(Value::as_str).map(str::to_owned);
            match (field("old"), field("new")) {
                (Some(old), Some(new)) => Ok(silt_proto::artifact::Edit { old, new, file: field("file") }),
                _ => Err(format!("edit {} needs old and new text", at + 1)),
            }
        })
        .collect()
}

/// How long a page's tool may take, past the daemon's own wait.
const PAGE_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(70);

/// `silt sample-tools`: the tools a page gave its question, served to the
/// agent answering it; each call is run by the page (ADR 0072).
struct PageTools {
    ask: String,
    listed: Value,
}

impl silt_mcp::Backend for PageTools {
    fn call(&mut self, tool: &str, arguments: &Value) -> Result<String, String> {
        let request = Request::SampleToolCall { ask: self.ask.clone(), name: tool.to_owned(), input: arguments.clone() };
        match client::send(&paths::socket_path(), &request, Some(PAGE_TOOL_TIMEOUT)) {
            Ok(Response::Ok { data: Value::String(text) }) => Ok(text),
            Ok(Response::Ok { data }) => Ok(data.to_string()),
            Ok(Response::Error { message }) => Err(message),
            Err(error) => Err(format!("the Silt daemon is not answering: {error}")),
        }
    }

    fn listed(&self) -> Option<Value> {
        Some(self.listed.clone())
    }
}

pub fn serve_page_tools(file: &std::path::Path) -> Result<(), Exn<CliError>> {
    let text = std::fs::read_to_string(file).or_raise(|| CliError(format!("cannot read {}", file.display())))?;
    let offered: Value = serde_json::from_str(&text).or_raise(|| CliError("the page's tools aren't JSON".into()))?;
    let ask = offered["ask"].as_str().unwrap_or_default().to_owned();
    let listed = offered["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|tool| {
            serde_json::json!({
                "name": tool["name"],
                "description": tool["description"],
                "inputSchema": if tool["input_schema"].is_object() { tool["input_schema"].clone() } else { serde_json::json!({"type": "object"}) },
            })
        })
        .collect();
    let mut tools = PageTools { ask, listed: Value::Array(listed) };
    silt_mcp::serve(io::stdin().lock(), io::stdout().lock(), &mut tools).or_raise(|| CliError("the MCP connection failed".into()))
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
