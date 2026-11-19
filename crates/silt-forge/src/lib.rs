//! The forges a thread's pull request lives on (ADR 0042), as requests to
//! make and answers to read: GitHub, GitLab and Forgejo. No I/O here: the
//! server's forge bridge sends them, and GitHub's plugin builds its URLs
//! with them (ADR 0103).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForgeKind {
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "gitlab")]
    GitLab,
    Forgejo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Patch,
    Put,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: Method,
    pub url: String,
    pub body: Option<Value>,
}

impl Call {
    fn get(url: String) -> Self {
        Self { method: Method::Get, url, body: None }
    }
}

/// The API of one repository on one forge.
#[derive(Debug, Clone)]
pub struct Api {
    pub kind: ForgeKind,
    /// The API's root, without a trailing slash.
    pub base: String,
    /// `owner/name`, as the forge names the repository.
    pub path: String,
}

/// Where a forge's API is when the config doesn't say.
pub fn default_api(kind: ForgeKind, host: &str) -> String {
    match kind {
        ForgeKind::GitHub if host == "github.com" => "https://api.github.com".into(),
        ForgeKind::GitHub => format!("https://{host}/api/v3"),
        ForgeKind::GitLab => format!("https://{host}/api/v4"),
        ForgeKind::Forgejo => format!("https://{host}/api/v1"),
    }
}

/// The header a token goes in.
pub fn auth_header(kind: ForgeKind, token: &str) -> (&'static str, String) {
    match kind {
        ForgeKind::GitHub => ("Authorization", format!("Bearer {token}")),
        ForgeKind::GitLab => ("PRIVATE-TOKEN", token.to_owned()),
        ForgeKind::Forgejo => ("Authorization", format!("token {token}")),
    }
}

/// Splits a project (`github.com/acme/web`) into its host and repository path.
pub fn split_repo(repo: &str) -> Option<(&str, &str)> {
    let (host, path) = repo.split_once('/')?;
    (!host.is_empty() && path.contains('/') && !path.split('/').any(|part| part.is_empty() || part == ".."))
        .then_some((host, path))
}

/// Encodes one URL path segment or query value.
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

impl Api {
    /// The repository's own API URL, which every call about it starts with.
    pub fn repo(&self) -> String {
        match self.kind {
            ForgeKind::GitLab => format!("{}/projects/{}", self.base, encode(&self.path)),
            ForgeKind::GitHub | ForgeKind::Forgejo => {
                let path: Vec<String> = self.path.split('/').map(encode).collect();
                format!("{}/repos/{}", self.base, path.join("/"))
            }
        }
    }

    /// Who the token is.
    pub fn me(&self) -> Call {
        Call::get(format!("{}/user", self.base))
    }

    /// The open pull requests from `branch`.
    pub fn open_pulls(&self, branch: &str) -> Call {
        let branch = encode(branch);
        Call::get(match self.kind {
            ForgeKind::GitHub => {
                let owner = encode(self.path.split('/').next().unwrap_or_default());
                format!("{}/pulls?state=open&head={owner}:{branch}&per_page=100", self.repo())
            }
            ForgeKind::GitLab => format!("{}/merge_requests?state=opened&source_branch={branch}&per_page=100", self.repo()),
            ForgeKind::Forgejo => format!("{}/pulls?state=open&limit=50", self.repo()),
        })
    }

    fn pull(&self, pull: u64) -> String {
        match self.kind {
            ForgeKind::GitLab => format!("{}/merge_requests/{pull}/notes", self.repo()),
            ForgeKind::GitHub | ForgeKind::Forgejo => format!("{}/issues/{pull}/comments", self.repo()),
        }
    }

    pub fn comments(&self, pull: u64) -> Call {
        let page = match self.kind {
            ForgeKind::Forgejo => "limit=50",
            ForgeKind::GitHub | ForgeKind::GitLab => "per_page=100",
        };
        Call::get(format!("{}?{page}", self.pull(pull)))
    }

    pub fn create(&self, pull: u64, body: &str) -> Call {
        Call { method: Method::Post, url: self.pull(pull), body: Some(json!({ "body": body })) }
    }

    pub fn update(&self, pull: u64, comment: u64, body: &str) -> Call {
        let (method, url) = match self.kind {
            ForgeKind::GitLab => (Method::Put, format!("{}/{comment}", self.pull(pull))),
            ForgeKind::GitHub | ForgeKind::Forgejo => (Method::Patch, format!("{}/issues/comments/{comment}", self.repo())),
        };
        Call { method, url, body: Some(json!({ "body": body })) }
    }
}

/// The token's user name, from [`Api::me`]'s answer.
pub fn read_me(kind: ForgeKind, answer: &Value) -> Option<String> {
    let key = match kind {
        ForgeKind::GitLab => "username",
        ForgeKind::GitHub | ForgeKind::Forgejo => "login",
    };
    answer.get(key)?.as_str().map(str::to_owned)
}

/// The number of the first open pull request from `branch`, from
/// [`Api::open_pulls`]'s answer. Forgejo can't filter by branch, so it's
/// done here, and for every forge a pull request from a fork's branch of
/// the same name is left alone.
pub fn read_pull(kind: ForgeKind, answer: &Value, branch: &str, path: &str) -> Option<u64> {
    answer.as_array()?.iter().find_map(|pull| match kind {
        ForgeKind::GitLab => {
            let same_project = pull.get("source_project_id") == pull.get("target_project_id");
            (pull.get("source_branch")?.as_str()? == branch && same_project).then(|| pull.get("iid")?.as_u64()).flatten()
        }
        ForgeKind::GitHub | ForgeKind::Forgejo => {
            let head = pull.get("head")?;
            let from = head.get("repo")?.get("full_name")?.as_str()?;
            (head.get("ref")?.as_str()? == branch && from.eq_ignore_ascii_case(path))
                .then(|| pull.get("number")?.as_u64())
                .flatten()
        }
    })
}

/// The id of the comment `me` left with `marker`, from [`Api::comments`]'s
/// answer. Anyone can paste the marker; only the token's own comment counts.
pub fn read_comment(kind: ForgeKind, answer: &Value, marker: &str, me: &str) -> Option<u64> {
    let author_key = match kind {
        ForgeKind::GitLab => ("author", "username"),
        ForgeKind::GitHub | ForgeKind::Forgejo => ("user", "login"),
    };
    answer.as_array()?.iter().find_map(|comment| {
        let author = comment.get(author_key.0)?.get(author_key.1)?.as_str()?;
        let body = comment.get("body")?.as_str()?;
        (author == me && body.starts_with(marker)).then(|| comment.get("id")?.as_u64()).flatten()
    })
}

/// What a pull request's comment says of its thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadComment {
    pub thread: String,
    pub title: String,
    pub link: String,
    pub turns: usize,
    pub commits: usize,
    /// The open merge request's summary, when the thread has one.
    pub summary: Option<String>,
    pub forks: Vec<Linked>,
    pub merged: Vec<Linked>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub title: String,
    pub link: String,
    /// `in progress`, `merge request open`, `merged` or `declined`.
    pub state: String,
}

pub fn marker(thread: &str) -> String {
    format!("<!-- silt:thread:{thread} -->")
}

impl ThreadComment {
    pub fn body(&self) -> String {
        let plural = |count: usize, word: &str| if count == 1 { format!("1 {word}") } else { format!("{count} {word}s") };
        let mut body = format!(
            "{}\n**Silt thread** [{}]({}): {}, {}.\n",
            marker(&self.thread),
            escape(&self.title),
            self.link,
            plural(self.turns, "turn"),
            plural(self.commits, "commit"),
        );
        if let Some(summary) = self.summary.as_deref().map(str::trim).filter(|summary| !summary.is_empty()) {
            body.push('\n');
            for line in summary.lines() {
                body.push_str(&format!("> {}\n", escape(line)));
            }
        }
        for (heading, linked) in [("Forks", &self.forks), ("Merged in", &self.merged)] {
            if linked.is_empty() {
                continue;
            }
            body.push_str(&format!("\n**{heading}**\n"));
            for item in linked {
                body.push_str(&format!("- [{}]({}): {}\n", escape(&item.title), item.link, item.state));
            }
        }
        body
    }
}

/// Text from a thread, which anyone writing in it chose, kept from being
/// read as Markdown or HTML.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars().filter(|ch| !ch.is_control()) {
        if "\\`*_{}[]()<>#+-.!|~&".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(kind: ForgeKind) -> Api {
        Api { kind, base: "https://forge.example/api".into(), path: "acme/web app".into() }
    }

    #[test]
    fn each_forge_is_asked_in_its_own_words() {
        let github = api(ForgeKind::GitHub);
        assert_eq!(
            github.open_pulls("feat/x").url,
            "https://forge.example/api/repos/acme/web%20app/pulls?state=open&head=acme:feat%2Fx&per_page=100"
        );
        assert_eq!(github.update(4, 9, "hi").url, "https://forge.example/api/repos/acme/web%20app/issues/comments/9");
        let gitlab = api(ForgeKind::GitLab);
        assert_eq!(
            gitlab.open_pulls("main").url,
            "https://forge.example/api/projects/acme%2Fweb%20app/merge_requests?state=opened&source_branch=main&per_page=100"
        );
        let update = gitlab.update(4, 9, "hi");
        assert_eq!(
            (update.method, update.url.as_str()),
            (Method::Put, "https://forge.example/api/projects/acme%2Fweb%20app/merge_requests/4/notes/9")
        );
        assert_eq!(
            api(ForgeKind::Forgejo).create(4, "hi").url,
            "https://forge.example/api/repos/acme/web%20app/issues/4/comments"
        );
        assert_eq!(default_api(ForgeKind::GitHub, "github.com"), "https://api.github.com");
        assert_eq!(auth_header(ForgeKind::GitLab, "t"), ("PRIVATE-TOKEN", "t".to_owned()));
    }

    #[test]
    fn a_pull_request_from_a_fork_with_the_same_branch_is_left_alone() {
        let pulls = json!([
            {"number": 3, "head": {"ref": "main", "repo": {"full_name": "someone/web"}}},
            {"number": 5, "head": {"ref": "main", "repo": {"full_name": "acme/web"}}},
        ]);
        assert_eq!(read_pull(ForgeKind::Forgejo, &pulls, "main", "acme/web"), Some(5));
        assert_eq!(read_pull(ForgeKind::Forgejo, &pulls, "other", "acme/web"), None);
        let requests = json!([{"iid": 7, "source_branch": "main", "source_project_id": 1, "target_project_id": 1}]);
        assert_eq!(read_pull(ForgeKind::GitLab, &requests, "main", "acme/web"), Some(7));
    }

    #[test]
    fn only_the_tokens_own_comment_with_the_marker_is_updated() {
        let marker = marker("ab");
        let comments = json!([
            {"id": 1, "user": {"login": "mallory"}, "body": format!("{marker}\nfake")},
            {"id": 2, "user": {"login": "silt-bot"}, "body": "unrelated"},
            {"id": 3, "user": {"login": "silt-bot"}, "body": format!("{marker}\nours")},
        ]);
        assert_eq!(read_comment(ForgeKind::GitHub, &comments, &marker, "silt-bot"), Some(3));
        assert_eq!(read_comment(ForgeKind::GitHub, &comments, &marker, "nobody"), None);
    }

    #[test]
    fn the_comment_names_the_thread_its_forks_and_merges_with_their_text_escaped() {
        let comment = ThreadComment {
            thread: "ab".into(),
            title: "Fix [login](javascript:x) <b>".into(),
            link: "https://silt.example/app/t/ab".into(),
            turns: 1,
            commits: 2,
            summary: Some("Fixes it\nand *more*".into()),
            forks: vec![Linked { title: "Try it".into(), link: "https://silt.example/app/t/cd".into(), state: "merged".into() }],
            merged: Vec::new(),
        };
        let body = comment.body();
        assert!(body.starts_with("<!-- silt:thread:ab -->\n"), "{body}");
        assert!(
            body.contains(r"[Fix \[login\]\(javascript:x\) \<b\>](https://silt.example/app/t/ab): 1 turn, 2 commits."),
            "{body}"
        );
        assert!(body.contains("> Fixes it\n> and \\*more\\*\n"), "{body}");
        assert!(
            body.contains("**Forks**\n- [Try it](https://silt.example/app/t/cd): merged\n") && !body.contains("Merged in"),
            "{body}"
        );
    }
}
