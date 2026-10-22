//! Silt's docs site: the guide, self-hosting and the command-line reference
//! from `docs/`, rendered to static HTML that needs no service to host.

use std::fs;
use std::path::{Path, PathBuf};

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

/// The site's pages, in the order the sidebar lists them, by their path
/// under `docs/`.
pub const PAGES: &[&str] = &[
    "guide/index.md",
    "guide/handoff.md",
    "guide/forks-and-merge-requests.md",
    "guide/history-and-search.md",
    "guide/agents.md",
    "guide/web-ui.md",
    "live-and-agents.md",
    "guide/sharing.md",
    "guide/hosted-app-and-phones.md",
    "guide/servers.md",
    "self-hosting.md",
    "reference/cli.md",
    "architecture.md",
    "contributing.md",
];

/// The landing page, rendered as the site's `index.html` (ADR 0033).
pub const LANDING: &str = "landing.md";

/// Where docs that aren't pages of the site, such as the ADRs, are read.
const REPOSITORY_DOCS: &str = "https://github.com/Urcra/silt/blob/main/docs";

#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    #[error("could not read {path}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("could not write {path}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("{page} links to {target}, which isn't in docs/")]
    BrokenLink { page: String, target: String },
}

/// A page rendered: its title, its body as HTML, and the docs it links to,
/// relative to `docs/`.
#[derive(Debug, PartialEq, Eq)]
pub struct Rendered {
    pub title: String,
    pub body: String,
    pub links: Vec<String>,
}

/// Renders `markdown`, the page at `page` under `docs/`: its links to other
/// pages go to their HTML, links to other docs to the repository, and its
/// first heading is its title.
pub fn render(page: &str, markdown: &str) -> Rendered {
    let mut title = None;
    let mut in_title = false;
    let mut links = Vec::new();
    let events: Vec<Event> = Parser::new_ext(markdown, Options::ENABLE_TABLES)
        .map(|event| match event {
            Event::Start(Tag::Heading { level: HeadingLevel::H1, .. }) if title.is_none() => {
                in_title = true;
                title = Some(String::new());
                event
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                in_title = false;
                event
            }
            Event::Text(ref text) | Event::Code(ref text) if in_title => {
                if let Some(title) = &mut title {
                    title.push_str(text);
                }
                event
            }
            Event::Start(Tag::Link { link_type, dest_url, title: link_title, id }) => {
                let dest_url = match local_target(page, &dest_url) {
                    Some((target, html)) => {
                        links.push(target);
                        CowStr::from(html)
                    }
                    None => dest_url,
                };
                Event::Start(Tag::Link { link_type, dest_url, title: link_title, id })
            }
            other => other,
        })
        .collect();
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    Rendered { title: title.unwrap_or_else(|| page.to_owned()), body, links }
}

/// A link to another Markdown file of the docs: the file it names under
/// `docs/`, and where the link goes: its page's HTML, or the file in the
/// repository when it isn't a page. `None` for links elsewhere.
fn local_target(page: &str, dest: &str) -> Option<(String, String)> {
    if dest.contains("://") || dest.starts_with('#') || dest.starts_with("mailto:") {
        return None;
    }
    let (file, anchor) = dest.split_once('#').map_or((dest, None), |(file, anchor)| (file, Some(anchor)));
    let stem = file.strip_suffix(".md")?;
    let mut parts: Vec<&str> = page.split('/').collect();
    parts.pop();
    for part in file.split('/') {
        match part {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            part => parts.push(part),
        }
    }
    let target = parts.join("/");
    let anchor = anchor.map(|anchor| format!("#{anchor}")).unwrap_or_default();
    let href = if PAGES.contains(&target.as_str()) {
        format!("{stem}.html{anchor}")
    } else {
        format!("{REPOSITORY_DOCS}/{target}{anchor}")
    };
    Some((target, href))
}

/// The page's HTML document, with the site's sidebar.
pub fn document(page: &str, rendered: &Rendered, titles: &[(String, String)]) -> String {
    let depth = page.matches('/').count();
    let up = "../".repeat(depth);
    let nav: String = titles
        .iter()
        .map(|(other, title)| {
            let href = format!("{up}{}", other.replace(".md", ".html"));
            let current = if other == page { " aria-current=\"page\"" } else { "" };
            format!("<li><a href=\"{href}\"{current}>{}</a></li>", escape(title))
        })
        .collect();
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} · Silt</title>\n<style>{STYLE}</style></head>\n\
         <body><nav aria-label=\"docs\"><a class=\"brand\" href=\"{up}guide/index.html\">Silt</a><ul>{nav}</ul></nav>\n\
         <main>{body}</main></body></html>\n",
        title = escape(&rendered.title),
        body = rendered.body,
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Builds the site from `docs` into `out`, failing on a link to a page the
/// site doesn't have. Returns the files written.
pub fn build(docs: &Path, out: &Path) -> Result<Vec<PathBuf>, SiteError> {
    let mut pages = Vec::new();
    for page in PAGES {
        let path = docs.join(page);
        let markdown = fs::read_to_string(&path).map_err(|source| SiteError::Read { path: path.clone(), source })?;
        pages.push((*page, render(page, &markdown)));
    }
    for (page, rendered) in &pages {
        if let Some(target) = rendered.links.iter().find(|target| !docs.join(target).is_file()) {
            return Err(SiteError::BrokenLink { page: (*page).to_owned(), target: target.clone() });
        }
    }
    let titles: Vec<(String, String)> =
        pages.iter().map(|(page, rendered)| ((*page).to_owned(), rendered.title.clone())).collect();
    let mut written = Vec::new();
    for (page, rendered) in &pages {
        let path = out.join(page.replace(".md", ".html"));
        write(&path, &document(page, rendered, &titles))?;
        written.push(path);
    }
    let path = docs.join(LANDING);
    let markdown = fs::read_to_string(&path).map_err(|source| SiteError::Read { path: path.clone(), source })?;
    let landing = render(LANDING, &markdown);
    if let Some(target) = landing.links.iter().find(|target| !docs.join(target).is_file()) {
        return Err(SiteError::BrokenLink { page: LANDING.to_owned(), target: target.clone() });
    }
    let index = out.join("index.html");
    write(&index, &landing_document(&landing))?;
    written.push(index);
    Ok(written)
}

/// The landing page (ADR 0033), the site's root: the docs' colours, without
/// their sidebar.
pub fn landing_document(rendered: &Rendered) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Silt · {title}</title>\n<style>{STYLE}</style></head>\n\
         <body class=\"landing\"><header><a class=\"brand\" href=\"index.html\">Silt</a><a href=\"guide/index.html\">Docs</a></header>\n\
         <main>{body}</main></body></html>\n",
        title = escape(&rendered.title),
        body = rendered.body,
    )
}

fn write(path: &Path, text: &str) -> Result<(), SiteError> {
    let failed = |source| SiteError::Write { path: path.to_owned(), source };
    fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(failed)?;
    fs::write(path, text).map_err(|source| SiteError::Write { path: path.to_owned(), source })
}

/// Nous: warm neutrals, one amber accent, IBM Plex where it's installed.
const STYLE: &str = "\
:root{--bg:#faf8f5;--fg:#24211d;--muted:#6b645b;--line:#e4ded5;--accent:#b4690e;--code:#f1ece4}\
@media(prefers-color-scheme:dark){:root{--bg:#1b1916;--fg:#ece7df;--muted:#a39b90;--line:#38332d;--accent:#e8a33d;--code:#26231f}}\
*{box-sizing:border-box}body{margin:0;display:flex;flex-wrap:wrap;background:var(--bg);color:var(--fg);\
font:16px/1.6 'IBM Plex Sans',system-ui,sans-serif}\
nav{flex:0 0 15rem;padding:1.5rem 1rem;border-right:1px solid var(--line);min-height:100vh}\
nav ul{list-style:none;padding:0;margin:1rem 0}nav li{margin:.3rem 0}nav a{color:var(--muted);text-decoration:none}\
nav a[aria-current]{color:var(--accent);font-weight:600}.brand{font-weight:700;color:var(--fg)!important}\
main{flex:1 1 30rem;max-width:46rem;padding:1.5rem 1rem;min-width:0}a{color:var(--accent)}\
code,pre{font-family:'IBM Plex Mono',ui-monospace,monospace;font-size:.9em;background:var(--code);border-radius:4px}\
code{padding:.1em .3em}pre{padding:.75rem 1rem;overflow-x:auto}pre code{padding:0}\
h1,h2,h3{line-height:1.25}h2{margin-top:2rem}\
@media(max-width:40rem){nav{min-height:0;border-right:0;border-bottom:1px solid var(--line);flex-basis:100%}}\
body.landing{display:block}.landing header{display:flex;gap:1.5rem;align-items:center;max-width:48rem;margin:0 auto;\
padding:1rem;border-bottom:1px solid var(--line)}.landing header a{text-decoration:none}\
.landing main{max-width:48rem;margin:0 auto;padding:2.5rem 1rem 4rem}.landing h1{font-size:clamp(2rem,6vw,3rem);margin:0 0 1rem}\
.landing h1+p{font-size:1.2rem;color:var(--muted)}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_between_pages_go_to_their_html_and_others_stay() {
        let rendered = render(
            "guide/handoff.md",
            "# Hand off\n\nSee [the reference](../reference/cli.md#silt-resume), [servers](servers.md), \
             [the decision](../adr/0009-sync-and-handoff.md) and [Rust](https://www.rust-lang.org).",
        );
        assert_eq!(rendered.title, "Hand off");
        assert_eq!(rendered.links, ["reference/cli.md", "guide/servers.md", "adr/0009-sync-and-handoff.md"]);
        assert!(rendered.body.contains("href=\"../reference/cli.html#silt-resume\""), "{}", rendered.body);
        assert!(rendered.body.contains("href=\"servers.html\""));
        assert!(rendered.body.contains("href=\"https://github.com/Urcra/silt/blob/main/docs/adr/0009-sync-and-handoff.md\""));
        assert!(rendered.body.contains("href=\"https://www.rust-lang.org\""));
    }

    #[test]
    fn the_sidebar_marks_the_page_and_reaches_the_others_from_its_folder() {
        let titles = vec![
            ("guide/index.md".to_owned(), "Silt".to_owned()),
            ("self-hosting.md".to_owned(), "Self-hosting <now>".to_owned()),
        ];
        let rendered = render("guide/index.md", "# Silt\n");
        let html = document("guide/index.md", &rendered, &titles);
        assert!(html.contains("<a href=\"../guide/index.html\" aria-current=\"page\">Silt</a>"), "{html}");
        assert!(html.contains("<a href=\"../self-hosting.html\">Self-hosting &lt;now&gt;</a>"), "{html}");
    }

    #[test]
    fn the_repositorys_docs_build_with_every_link_in_place() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let out = tempfile::tempdir().expect("temp dir");
        let written = build(&docs, out.path()).expect("the site builds");
        assert_eq!(written.len(), PAGES.len() + 1);
        let handoff = fs::read_to_string(out.path().join("guide/handoff.html")).expect("read");
        assert!(handoff.contains("<title>Hand off a thread to another machine · Silt</title>"));
        let landing = fs::read_to_string(out.path().join("index.html")).expect("read");
        assert!(landing.contains("class=\"landing\"") && landing.contains("href=\"guide/index.html\""), "{landing}");
        assert!(landing.contains("href=\"guide/handoff.html\""), "the landing page reaches the guide");
    }
}
