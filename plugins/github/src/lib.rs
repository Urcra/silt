//! GitHub, shipped with Silt (ADR 0103): an agent's clones, pushes and calls
//! to GitHub's API, each one an action the person's rules allow, ask or
//! never, per repo. Silt keeps the thread's mirrors and pushes from them;
//! the agent never holds a token.

use std::collections::BTreeMap;

use silt_plugin_api::http::{Request, Response, send};
use silt_plugin_api::{
    Block, Call, Card, Context, Effect, FileChange, Item, Output, Plugin, Tone, Tool, Value, git, json, serde_json,
};

struct GitHub;

const API: &str = "https://api.github.com";
const GIT: &str = "https://github.com";
/// Where the agent sees the thread's mirrors and its work area.
const MIRRORS: &str = "/silt/github";
const WORK: &str = "/silt/work";

type Fields = BTreeMap<String, String>;

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty())
}

fn need<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    text(input, key).ok_or_else(|| format!("give `{key}`"))
}

fn number(input: &Value, key: &str) -> Option<u64> {
    input.get(key).and_then(Value::as_u64)
}

/// `owner/name`, as GitHub spells a repo.
fn repo(input: &Value) -> Result<String, String> {
    let repo = need(input, "repo")?.trim_end_matches(".git");
    let repo = repo.strip_prefix("https://github.com/").unwrap_or(repo);
    let parts: Vec<&str> = repo.split('/').collect();
    let named = parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
                && *part != "."
                && *part != ".."
                && part.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        });
    if named { Ok(repo.to_owned()) } else { Err(format!("{repo} isn't a repo, such as `acme/api`")) }
}

/// A branch name git would take, without anything that reads as an option
/// or climbs.
fn branch_name(name: &str) -> Result<String, String> {
    let name = name.strip_prefix("refs/heads/").unwrap_or(name);
    let fine = !name.is_empty()
        && !name.starts_with('-')
        && !name.contains("..")
        && !name.ends_with('/')
        && !name.ends_with(".lock")
        && name.chars().all(|c| c.is_ascii_graphic() && !"~^:?*[\\".contains(c));
    if fine { Ok(name.to_owned()) } else { Err(format!("{name} isn't a branch name")) }
}

use silt_forge::encode;

fn url(repo: &str, rest: &str) -> String {
    let api = silt_forge::Api { kind: silt_forge::ForgeKind::GitHub, base: API.to_owned(), path: repo.to_owned() };
    format!("{}{rest}", api.repo())
}

fn remote(repo: &str) -> String {
    format!("{GIT}/{repo}.git")
}

fn headers() -> Vec<(String, String)> {
    vec![
        ("accept".into(), "application/vnd.github+json".into()),
        ("x-github-api-version".into(), "2022-11-28".into()),
        ("user-agent".into(), "silt-github-plugin".into()),
    ]
}

fn get_with(url: &str, accept: Option<&str>) -> Result<Response, String> {
    let mut headers = headers();
    if let Some(accept) = accept {
        headers[0].1 = accept.to_owned();
    }
    send(&Request { method: "GET".into(), url: url.to_owned(), headers, body: None })
}

/// A read's answer, or what GitHub said was wrong.
fn get(url: &str) -> Result<Value, String> {
    let answer = get_with(url, None)?;
    answered(&answer)
}

fn answered(answer: &Response) -> Result<Value, String> {
    let body: Value = serde_json::from_slice(&answer.body).unwrap_or(Value::Null);
    if (200..300).contains(&answer.status) {
        return Ok(body);
    }
    let message = body.get("message").and_then(Value::as_str).unwrap_or("no message");
    Err(format!("GitHub answered {}: {message}", answer.status))
}

/// A change: its method, where it goes, and its body exactly as it's sent.
struct Change {
    method: &'static str,
    url: String,
    body: Option<String>,
}

impl Change {
    fn effect(&self) -> Effect {
        Effect::Http {
            method: self.method.into(),
            url: self.url.clone(),
            body_sha256: self.body.as_ref().map(|body| silt_plugin_api::body_sha256(body.as_bytes())),
        }
    }

    fn send(&self) -> Result<Value, String> {
        let mut headers = headers();
        if self.body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let request = Request {
            method: self.method.into(),
            url: self.url.clone(),
            headers,
            body: self.body.as_ref().map(|body| body.as_bytes().to_vec()),
        };
        let answer = send(&request)?;
        if answer.body.is_empty() && (200..300).contains(&answer.status) {
            return Ok(Value::Null);
        }
        answered(&answer)
    }
}

fn body(value: Value) -> Option<String> {
    // serde_json's maps keep their keys sorted, so the same input makes the
    // same bytes when it's prepared and when it runs.
    Some(value.to_string())
}

fn fields(pairs: &[(&str, &str)]) -> Fields {
    pairs.iter().map(|(key, value)| ((*key).to_owned(), (*value).to_owned())).collect()
}

fn pairs(pairs: &[(&str, String)]) -> Block {
    Block::Fields { pairs: pairs.iter().map(|(name, value)| ((*name).to_owned(), value.clone())).collect() }
}

fn tool(name: &str, action: &str, description: &str, properties: Value, required: &[&str]) -> Tool {
    let mut properties = properties;
    properties["repo"] = json!({"type": "string", "description": "The repo, such as acme/api"});
    let mut required: Vec<&str> = required.to_vec();
    required.insert(0, "repo");
    Tool {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type": "object", "properties": properties, "required": required}),
        action: action.into(),
    }
}

/// Where a path the agent names is for Silt: its checkout's, relative, or
/// its work area's, under `work/`.
fn thread_path(path: &str) -> Result<String, String> {
    let path = path.trim_end_matches('/');
    if let Some(rest) = path.strip_prefix(WORK) {
        return Ok(format!("work{rest}"));
    }
    if path.starts_with('/') {
        return Err(format!("{path} isn't in your checkout or under {WORK}; push from one of those"));
    }
    Ok(if path.is_empty() || path == "." { ".".to_owned() } else { path.trim_start_matches("./").to_owned() })
}

/// What a push is: where the branch is now, where it goes, how it's
/// classified, and what it brings.
struct Push {
    repo: String,
    branch: String,
    old: Option<String>,
    new: String,
    action: &'static str,
    commits: Vec<git::Commit>,
    files: Vec<git::FileChange>,
    default_branch: String,
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(7)]
}

fn prepare_push(input: &Value) -> Result<Push, String> {
    let repo = repo(input)?;
    let local = branch_name(need(input, "branch")?)?;
    let branch = branch_name(text(input, "remote_branch").unwrap_or(&local))?;
    let path = thread_path(text(input, "path").unwrap_or("."))?;
    let reference = format!("refs/heads/{branch}");
    let about = get(&url(&repo, ""))?;
    let default_branch = about.get("default_branch").and_then(Value::as_str).unwrap_or("main").to_owned();
    let old = git::remote_head(&remote(&repo), &reference)?;
    let mut refspecs = vec![format!("+refs/heads/{default_branch}:refs/heads/{default_branch}")];
    if old.is_some() && branch != default_branch {
        refspecs.push(format!("+{reference}:{reference}"));
    }
    git::fetch(&repo, &remote(&repo), &refspecs)?;
    let new = git::import_commit(&repo, &path, &format!("refs/heads/{local}"))?;
    let force = match &old {
        Some(old) if *old != new => !git::log(&repo, &new, old)?.0.is_empty(),
        _ => false,
    };
    let protected = match get(&url(&repo, &format!("/branches/{}", encode(&branch)))) {
        Ok(found) => found.get("protected").and_then(Value::as_bool).unwrap_or(false),
        Err(_) => false,
    };
    let action = if force {
        "push_force"
    } else if branch == default_branch || protected {
        "push_default"
    } else {
        "push"
    };
    let base = old.clone().unwrap_or_else(|| format!("refs/heads/{default_branch}"));
    let (commits, files) =
        if old.as_deref() == Some(new.as_str()) { (Vec::new(), Vec::new()) } else { git::log(&repo, &base, &new)? };
    Ok(Push { repo, branch, old, new, action, commits, files, default_branch })
}

fn push_card(push: &Push) -> Card {
    let moves = match &push.old {
        Some(old) => format!("{} → {}", short(old), short(&push.new)),
        None => format!("new, at {}", short(&push.new)),
    };
    let mut blocks = vec![pairs(&[("Repo", push.repo.clone()), ("Branch", push.branch.clone()), ("Moves", moves)])];
    if !push.commits.is_empty() {
        blocks.push(Block::List {
            label: format!("{} commit{}", push.commits.len(), if push.commits.len() == 1 { "" } else { "s" }),
            items: push
                .commits
                .iter()
                .map(|commit| Item { key: Some(short(&commit.id).to_owned()), text: commit.subject.clone() })
                .collect(),
        });
    }
    if !push.files.is_empty() {
        blocks.push(Block::Diff {
            label: "Files".into(),
            files: push
                .files
                .iter()
                .map(|file| FileChange { path: file.path.clone(), added: file.added, removed: file.removed })
                .collect(),
        });
    }
    let note = match push.action {
        "push_force" => Some((Tone::Danger, "It doesn't follow from where the branch is: what's there now is lost.".to_owned())),
        "push_default" if push.branch == push.default_branch => {
            Some((Tone::Warn, format!("{} is the repo's default branch.", push.branch)))
        }
        "push_default" => Some((Tone::Warn, format!("{} is protected.", push.branch))),
        _ => None,
    };
    if let Some((tone, text)) = note {
        blocks.push(Block::Note { tone, text });
    }
    blocks.push(Block::Note {
        tone: Tone::Neutral,
        text: "Exactly this commit is pushed, and only if the branch is still where it was.".into(),
    });
    Card { title: format!("Push {} to {}", push.branch, push.repo), blocks }
}

/// The REST changes, each built the same way when it's prepared and when
/// it runs, with its card.
fn change(tool: &str, input: &Value, repo: &str) -> Result<(&'static str, Change, Card, Fields), String> {
    let base = fields(&[("repo", repo)]);
    Ok(match tool {
        "branch_delete" => {
            let branch = branch_name(need(input, "branch")?)?;
            let change = Change { method: "DELETE", url: url(repo, &format!("/git/refs/heads/{}", encode(&branch))), body: None };
            let card = Card {
                title: format!("Delete the branch {branch} of {repo}"),
                blocks: vec![pairs(&[("Repo", repo.to_owned()), ("Branch", branch.clone())])],
            };
            ("branch_delete", change, card, fields(&[("repo", repo), ("branch", &branch)]))
        }
        "issue_write" => {
            let mut value = json!({});
            for key in ["title", "body", "state"] {
                if let Some(given) = text(input, key) {
                    value[key] = json!(given);
                }
            }
            let (method, url, title) = match number(input, "number") {
                Some(issue) => ("PATCH", url(repo, &format!("/issues/{issue}")), format!("Edit issue #{issue} of {repo}")),
                None => {
                    need(input, "title")?;
                    ("POST", url(repo, "/issues"), format!("Open an issue in {repo}"))
                }
            };
            let mut blocks = vec![pairs(&[("Repo", repo.to_owned())])];
            if let Some(title) = text(input, "title") {
                blocks.push(pairs(&[("Title", title.to_owned())]));
            }
            if let Some(state) = text(input, "state") {
                blocks.push(pairs(&[("State", state.to_owned())]));
            }
            if let Some(said) = text(input, "body") {
                blocks.push(Block::Code { label: "Text".into(), text: said.to_owned() });
            }
            ("issue_write", Change { method, url, body: body(value) }, Card { title, blocks }, base)
        }
        "comment" => {
            let issue = number(input, "number").ok_or("give `number`, the issue's or pull request's")?;
            let said = need(input, "body")?;
            let change = Change {
                method: "POST",
                url: url(repo, &format!("/issues/{issue}/comments")),
                body: body(json!({"body": said})),
            };
            let card = Card {
                title: format!("Comment on #{issue} of {repo}"),
                blocks: vec![Block::Code { label: "Comment".into(), text: said.to_owned() }],
            };
            ("comment", change, card, base)
        }
        "pr_create" => {
            let head = branch_name(need(input, "head")?)?;
            let into = branch_name(need(input, "base")?)?;
            let title = need(input, "title")?;
            let said = text(input, "body").unwrap_or("");
            let draft = input.get("draft").and_then(Value::as_bool).unwrap_or(false);
            let change = Change {
                method: "POST",
                url: url(repo, "/pulls"),
                body: body(json!({"base": into, "head": head, "title": title, "body": said, "draft": draft})),
            };
            let mut blocks =
                vec![pairs(&[("Repo", repo.to_owned()), ("Into", format!("{into} ← {head}")), ("Title", title.to_owned())])];
            if !said.is_empty() {
                blocks.push(Block::Code { label: "Text".into(), text: said.to_owned() });
            }
            if draft {
                blocks.push(Block::Note { tone: Tone::Neutral, text: "Opened as a draft.".into() });
            }
            let card = Card { title: format!("Open a pull request in {repo}"), blocks };
            ("pr_create", change, card, fields(&[("repo", repo), ("branch", &head)]))
        }
        "pr_update" => {
            let pull = number(input, "number").ok_or("give `number`")?;
            let mut value = json!({});
            let mut shown = vec![("Repo", repo.to_owned())];
            for (key, label) in [("title", "Title"), ("state", "State"), ("base", "Into")] {
                if let Some(given) = text(input, key) {
                    value[key] = json!(given);
                    shown.push((label, given.to_owned()));
                }
            }
            let mut blocks = vec![pairs(&shown)];
            if let Some(said) = text(input, "body") {
                value["body"] = json!(said);
                blocks.push(Block::Code { label: "Text".into(), text: said.to_owned() });
            }
            let change = Change { method: "PATCH", url: url(repo, &format!("/pulls/{pull}")), body: body(value) };
            ("pr_update", change, Card { title: format!("Edit pull request #{pull} of {repo}"), blocks }, base)
        }
        "review" => {
            let pull = number(input, "number").ok_or("give `number`")?;
            let event = match need(input, "event")? {
                "approve" | "APPROVE" => "APPROVE",
                "request_changes" | "REQUEST_CHANGES" => "REQUEST_CHANGES",
                "comment" | "COMMENT" => "COMMENT",
                other => return Err(format!("{other}: approve, request_changes or comment")),
            };
            let said = text(input, "body").unwrap_or("");
            let change = Change {
                method: "POST",
                url: url(repo, &format!("/pulls/{pull}/reviews")),
                body: body(json!({"event": event, "body": said})),
            };
            let verb = match event {
                "APPROVE" => "Approve",
                "REQUEST_CHANGES" => "Ask for changes to",
                _ => "Review",
            };
            let mut blocks = vec![pairs(&[("Repo", repo.to_owned()), ("Pull request", format!("#{pull}"))])];
            if !said.is_empty() {
                blocks.push(Block::Code { label: "Review".into(), text: said.to_owned() });
            }
            ("review", change, Card { title: format!("{verb} pull request #{pull} of {repo}"), blocks }, base)
        }
        "merge" => {
            let pull = number(input, "number").ok_or("give `number`")?;
            let method = match text(input, "method").unwrap_or("merge") {
                method @ ("merge" | "squash" | "rebase") => method,
                other => return Err(format!("{other}: merge, squash or rebase")),
            };
            // Bound to the head it was approved at: a later push makes GitHub refuse it.
            let sha = match text(input, "sha") {
                Some(sha) => sha.to_owned(),
                None => get(&url(repo, &format!("/pulls/{pull}")))?
                    .pointer("/head/sha")
                    .and_then(Value::as_str)
                    .ok_or("GitHub didn't say where the pull request's head is")?
                    .to_owned(),
            };
            let change = Change {
                method: "PUT",
                url: url(repo, &format!("/pulls/{pull}/merge")),
                body: body(json!({"merge_method": method, "sha": sha})),
            };
            let card = Card {
                title: format!("Merge pull request #{pull} of {repo}"),
                blocks: vec![
                    pairs(&[("Repo", repo.to_owned()), ("At", short(&sha).to_owned()), ("By", method.to_owned())]),
                    Block::Note { tone: Tone::Neutral, text: "Only if its head is still at this commit.".into() },
                ],
            };
            ("merge", change, card, base)
        }
        "workflow_run" => {
            let workflow = need(input, "workflow")?;
            let on = branch_name(need(input, "ref")?)?;
            let inputs = input.get("inputs").cloned().unwrap_or_else(|| json!({}));
            let change = Change {
                method: "POST",
                url: url(repo, &format!("/actions/workflows/{}/dispatches", encode(workflow))),
                body: body(json!({"ref": on, "inputs": inputs})),
            };
            let mut blocks = vec![pairs(&[("Repo", repo.to_owned()), ("Workflow", workflow.to_owned()), ("On", on.clone())])];
            if inputs.as_object().is_some_and(|inputs| !inputs.is_empty()) {
                blocks.push(Block::Code {
                    label: "Inputs".into(),
                    text: serde_json::to_string_pretty(&inputs).unwrap_or_default(),
                });
            }
            (
                "workflow_run",
                change,
                Card { title: format!("Run {workflow} on {on} in {repo}"), blocks },
                fields(&[("repo", repo), ("branch", &on)]),
            )
        }
        "release" => {
            let tag = need(input, "tag")?;
            let name = text(input, "name").unwrap_or(tag);
            let said = text(input, "body").unwrap_or("");
            let draft = input.get("draft").and_then(Value::as_bool).unwrap_or(false);
            let mut value = json!({"tag_name": tag, "name": name, "body": said, "draft": draft});
            if let Some(target) = text(input, "target") {
                value["target_commitish"] = json!(target);
            }
            let change = Change { method: "POST", url: url(repo, "/releases"), body: body(value) };
            let mut blocks = vec![pairs(&[("Repo", repo.to_owned()), ("Tag", tag.to_owned()), ("Name", name.to_owned())])];
            if !said.is_empty() {
                blocks.push(Block::Code { label: "Notes".into(), text: said.to_owned() });
            }
            ("release", change, Card { title: format!("Release {tag} of {repo}"), blocks }, base)
        }
        other => return Err(format!("no tool {other}")),
    })
}

const CHANGES: [&str; 9] =
    ["branch_delete", "issue_write", "comment", "pr_create", "pr_update", "review", "merge", "workflow_run", "release"];

/// A read: what it fetches when it runs.
fn read(tool: &str, input: &Value, repo: &str) -> Result<(&'static str, Card, Fields), String> {
    let base = fields(&[("repo", repo)]);
    let card = |title: String| Card { title, blocks: vec![pairs(&[("Repo", repo.to_owned())])] };
    Ok(match tool {
        "clone" => ("clone", card(format!("Clone {repo}")), base),
        "fetch" => ("fetch", card(format!("Fetch {repo}")), base),
        "repo" => ("repo", card(format!("Read {repo}")), base),
        "file" => {
            let path = need(input, "path")?;
            ("file", card(format!("Read {path} of {repo}")), fields(&[("repo", repo), ("path", path)]))
        }
        "issues" => ("issues", card(format!("Read issues of {repo}")), base),
        "pulls" => ("pulls", card(format!("Read pull requests of {repo}")), base),
        "checks" => ("checks", card(format!("Read checks of {repo}")), base),
        "search" => ("search", card(format!("Search {repo}")), base),
        other => return Err(format!("no tool {other}")),
    })
}

fn run_read(tool: &str, input: &Value, repo: &str) -> Result<Output, String> {
    match tool {
        "clone" | "fetch" => {
            git::fetch(repo, &remote(repo), &[])?;
            let mirror = format!("{MIRRORS}/{repo}.git");
            let name = repo.split('/').nth(1).unwrap_or(repo);
            Ok(Output::text(if tool == "clone" {
                format!(
                    "{repo} is mirrored at {mirror}, read-only. Clone it into your work area with `git clone {mirror} {WORK}/{name}`, \
                     work there, and push with github_push. Fetch again with github_fetch, then `git fetch` in your clone."
                )
            } else {
                format!("{mirror} is up to date with GitHub; `git fetch` in your clone of it.")
            }))
        }
        "repo" => Ok(json_output(get(&url(repo, ""))?)),
        "file" => {
            let path: Vec<String> = need(input, "path")?.trim_matches('/').split('/').map(encode).collect();
            let at = text(input, "ref").map(|at| format!("?ref={}", encode(at))).unwrap_or_default();
            let answer =
                get_with(&url(repo, &format!("/contents/{}{at}", path.join("/"))), Some("application/vnd.github.raw+json"))?;
            if !(200..300).contains(&answer.status) {
                return Err(answered(&answer).err().unwrap_or_default());
            }
            Ok(Output::text(String::from_utf8_lossy(&answer.body).into_owned()))
        }
        "issues" => match number(input, "number") {
            Some(issue) => {
                let found = get(&url(repo, &format!("/issues/{issue}")))?;
                let comments = get(&url(repo, &format!("/issues/{issue}/comments?per_page=100")))?;
                Ok(json_output(json!({"issue": found, "comments": comments})))
            }
            None => {
                let state = text(input, "state").unwrap_or("open");
                Ok(json_output(get(&url(repo, &format!("/issues?state={}&per_page=50", encode(state))))?))
            }
        },
        "pulls" => match number(input, "number") {
            Some(pull) if input.get("diff").and_then(Value::as_bool).unwrap_or(false) => {
                let answer = get_with(&url(repo, &format!("/pulls/{pull}")), Some("application/vnd.github.diff"))?;
                if !(200..300).contains(&answer.status) {
                    return Err(answered(&answer).err().unwrap_or_default());
                }
                Ok(Output::text(String::from_utf8_lossy(&answer.body).into_owned()))
            }
            Some(pull) => Ok(json_output(get(&url(repo, &format!("/pulls/{pull}")))?)),
            None => {
                let state = text(input, "state").unwrap_or("open");
                Ok(json_output(get(&url(repo, &format!("/pulls?state={}&per_page=50", encode(state))))?))
            }
        },
        "checks" => {
            let at = need(input, "ref")?;
            Ok(json_output(get(&url(repo, &format!("/commits/{}/check-runs?per_page=100", encode(at))))?))
        }
        "search" => {
            let query = format!("{} repo:{repo}", need(input, "query")?);
            let kind = match text(input, "kind").unwrap_or("code") {
                "issues" => "issues",
                _ => "code",
            };
            Ok(json_output(get(&format!("{API}/search/{kind}?q={}&per_page=50", encode(&query)))?))
        }
        other => Err(format!("no tool {other}")),
    }
}

fn json_output(value: Value) -> Output {
    Output { text: serde_json::to_string_pretty(&value).unwrap_or_default(), data: Some(value) }
}

impl Plugin for GitHub {
    fn tools() -> Vec<Tool> {
        let s = |description: &str| json!({"type": "string", "description": description});
        let n = |description: &str| json!({"type": "integer", "description": description});
        vec![
            tool(
                "clone",
                "clone",
                "Mirror a GitHub repo for this thread at /silt/github, to clone it into /silt/work yourself.",
                json!({}),
                &[],
            ),
            tool("fetch", "fetch", "Bring the thread's mirror of a repo up to date with GitHub.", json!({}), &[]),
            tool(
                "push",
                "push",
                "Push a branch of your clone (in /silt/work, or your checkout) to GitHub. Silt pushes exactly your commit, from its mirror.",
                json!({
                    "path": s("Your clone: /silt/work/<name>, or . for your checkout"),
                    "branch": s("Your local branch"),
                    "remote_branch": s("The branch on GitHub, if it's named otherwise")
                }),
                &["branch"],
            ),
            tool("branch_delete", "branch_delete", "Delete a branch on GitHub.", json!({"branch": s("The branch")}), &["branch"]),
            tool("repo", "repo", "Read a repo: its description, default branch and settings.", json!({}), &[]),
            tool(
                "file",
                "file",
                "Read a file of a repo.",
                json!({"path": s("The file's path"), "ref": s("A branch, tag or commit")}),
                &["path"],
            ),
            tool(
                "issues",
                "issues",
                "List a repo's issues, or read one with its comments.",
                json!({"number": n("An issue's number"), "state": s("open, closed or all")}),
                &[],
            ),
            tool(
                "pulls",
                "pulls",
                "List a repo's pull requests, or read one, or its diff.",
                json!({"number": n("A pull request's number"), "diff": {"type": "boolean"}, "state": s("open, closed or all")}),
                &[],
            ),
            tool("checks", "checks", "Read the checks of a commit or branch.", json!({"ref": s("A commit or branch")}), &["ref"]),
            tool(
                "search",
                "search",
                "Search a repo's code or issues.",
                json!({"query": s("What to search for"), "kind": s("code or issues")}),
                &["query"],
            ),
            tool(
                "issue_write",
                "issue_write",
                "Open an issue, or edit one given its number.",
                json!({"number": n("The issue to edit"), "title": s("Its title"), "body": s("Its text"), "state": s("open or closed")}),
                &[],
            ),
            tool(
                "comment",
                "comment",
                "Comment on an issue or pull request.",
                json!({"number": n("The issue's or pull request's number"), "body": s("The comment")}),
                &["number", "body"],
            ),
            tool(
                "pr_create",
                "pr_create",
                "Open a pull request from a branch you pushed.",
                json!({"head": s("The branch with the changes"), "base": s("The branch to merge into"), "title": s("Its title"), "body": s("Its text"), "draft": {"type": "boolean"}}),
                &["head", "base", "title"],
            ),
            tool(
                "pr_update",
                "pr_update",
                "Edit a pull request: its title, text, base or state.",
                json!({"number": n("The pull request"), "title": s("Its title"), "body": s("Its text"), "base": s("The branch to merge into"), "state": s("open or closed")}),
                &["number"],
            ),
            tool(
                "review",
                "review",
                "Review a pull request.",
                json!({"number": n("The pull request"), "event": s("approve, request_changes or comment"), "body": s("The review")}),
                &["number", "event"],
            ),
            tool(
                "merge",
                "merge",
                "Merge a pull request at its head now.",
                json!({"number": n("The pull request"), "method": s("merge, squash or rebase")}),
                &["number"],
            ),
            tool(
                "workflow_run",
                "workflow_run",
                "Run a GitHub Actions workflow on a branch.",
                json!({"workflow": s("The workflow's file name or id"), "ref": s("The branch"), "inputs": {"type": "object"}}),
                &["workflow", "ref"],
            ),
            tool(
                "release",
                "release",
                "Make a release and its tag.",
                json!({"tag": s("The tag"), "name": s("Its name"), "body": s("Its notes"), "target": s("The commit or branch to tag"), "draft": {"type": "boolean"}}),
                &["tag"],
            ),
        ]
    }

    fn prepare(tool: &str, input: &Value, _context: &Context) -> Result<Call, String> {
        let repo = repo(input)?;
        if tool == "push" {
            let push = prepare_push(input)?;
            if push.old.as_deref() == Some(push.new.as_str()) {
                return Err(format!("{} is already at {} on GitHub", push.branch, short(&push.new)));
            }
            let effect = Effect::GitPush {
                remote: remote(&push.repo),
                reference: format!("refs/heads/{}", push.branch),
                old: push.old.clone().unwrap_or_default(),
                new: push.new.clone(),
            };
            return Ok(Call {
                action: push.action.into(),
                fields: fields(&[("repo", &push.repo), ("branch", &push.branch)]),
                card: push_card(&push),
                effects: vec![effect],
            });
        }
        if CHANGES.contains(&tool) {
            let (action, change, card, fields) = change(tool, input, &repo)?;
            return Ok(Call { action: action.into(), fields, card, effects: vec![change.effect()] });
        }
        let (action, card, fields) = read(tool, input, &repo)?;
        Ok(Call { action: action.into(), fields, card, effects: Vec::new() })
    }

    fn run(tool: &str, input: &Value, call: &Call, _context: &Context) -> Result<Output, String> {
        let repo = repo(input)?;
        if tool == "push" {
            let Some(Effect::GitPush { remote, reference, old, new }) = call.effects.first() else {
                return Err("this call names no push".into());
            };
            git::push(&repo, remote, new, reference, old)?;
            let branch = reference.trim_start_matches("refs/heads/");
            return Ok(Output::text(format!("Pushed {} to {branch} of {repo}.", short(new))));
        }
        if CHANGES.contains(&tool) {
            let (_, change, _, _) = change(tool, input, &repo)?;
            return Ok(json_output(change.send()?));
        }
        run_read(tool, input, &repo)
    }
}

silt_plugin_api::export!(GitHub);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_and_branches_are_checked() {
        assert_eq!(repo(&json!({"repo": "acme/api"})).unwrap(), "acme/api");
        assert_eq!(repo(&json!({"repo": "https://github.com/acme/api.git"})).unwrap(), "acme/api");
        for bad in ["acme", "acme/../x", "acme/api/x", "acme/a b"] {
            assert!(repo(&json!({"repo": bad})).is_err(), "{bad}");
        }
        assert_eq!(branch_name("refs/heads/ada/fix").unwrap(), "ada/fix");
        for bad in ["-f", "a..b", "a:b", "a b", "x.lock"] {
            assert!(branch_name(bad).is_err(), "{bad}");
        }
        assert_eq!(thread_path("/silt/work/api").unwrap(), "work/api");
        assert_eq!(thread_path(".").unwrap(), ".");
        assert!(thread_path("/etc").is_err());
    }

    #[test]
    fn a_change_is_the_same_bytes_prepared_and_run() {
        let input = json!({"repo": "acme/api", "head": "ada/fix", "base": "main", "title": "Fix it", "body": "Why\u{202e}"});
        let (_, one, card, fields) = change("pr_create", &input, "acme/api").unwrap();
        let (_, two, _, _) = change("pr_create", &input, "acme/api").unwrap();
        assert_eq!(one.body, two.body);
        assert_eq!(fields.get("branch").map(String::as_str), Some("ada/fix"));
        assert!(card.title.contains("acme/api"));
        assert!(matches!(one.effect(), Effect::Http { ref method, .. } if method == "POST"));
    }
}
