//! Silt's docs site: the guide, self-hosting and the command-line reference
//! from `docs/`, rendered to static HTML that needs no service to host, in
//! Nous as the Docs and Landing page boards draw it (ADR 0033, ADR 0039).

pub mod check;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use pulldown_cmark::{BlockQuoteKind, CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

/// The sidebar's groups, each with its pages in order, by their path under `docs/`.
pub const GROUPS: &[(&str, &[&str])] = &[
    (
        "Guide",
        &[
            "guide/index.md",
            "guide/import.md",
            "guide/handoff.md",
            "guide/forks-and-merge-requests.md",
            "guide/history-and-search.md",
            "guide/agents.md",
            "guide/web-ui.md",
            "guide/themes.md",
            "guide/artifacts.md",
            "live-and-agents.md",
            "guide/sharing.md",
            "guide/hosted-app-and-phones.md",
            "guide/servers.md",
            "guide/workspaces.md",
            "guide/plugins.md",
            "guide/writing-plugins.md",
            "guide/sandbox.md",
            "guide/local-models.md",
            "guide/cloud.md",
            "guide/git-and-forges.md",
            "guide/windows.md",
        ],
    ),
    ("Running a server", &["self-hosting.md"]),
    ("Reference", &["reference/cli.md"]),
    ("Working on Silt", &["architecture.md", "contributing.md"]),
];

/// The site's pages, in the order the sidebar lists them, by their path
/// under `docs/`.
pub const PAGES: &[&str] = &[
    "guide/index.md",
    "guide/import.md",
    "guide/handoff.md",
    "guide/forks-and-merge-requests.md",
    "guide/history-and-search.md",
    "guide/agents.md",
    "guide/web-ui.md",
    "guide/themes.md",
    "guide/artifacts.md",
    "live-and-agents.md",
    "guide/sharing.md",
    "guide/hosted-app-and-phones.md",
    "guide/servers.md",
    "guide/workspaces.md",
    "guide/plugins.md",
    "guide/writing-plugins.md",
    "guide/sandbox.md",
    "guide/local-models.md",
    "guide/cloud.md",
    "guide/git-and-forges.md",
    "guide/windows.md",
    "self-hosting.md",
    "reference/cli.md",
    "architecture.md",
    "contributing.md",
];

/// The landing page, rendered as the site's `index.html` (ADR 0033).
pub const LANDING: &str = "landing.md";

/// Where docs that aren't pages of the site, such as the ADRs, are read.
const REPOSITORY: &str = "https://github.com/Urcra/silt";
const REPOSITORY_DOCS: &str = "https://github.com/Urcra/silt/blob/main/docs";

#[derive(Debug, thiserror::Error)]
pub enum SiteError {
    #[error("could not read {path}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("could not write {path}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("{page} links to {target}, which isn't in docs/")]
    BrokenLink { page: String, target: String },
    #[error("{page} is in docs/guide but not in the site's PAGES")]
    NotAPage { page: String },
    #[error("{page} isn't reached by following links from {GUIDE}")]
    Unreachable { page: String },
    #[error("{page} names `silt {command}`, which isn't in the command-line reference")]
    UnknownCommand { page: String, command: String },
}

/// Where the guide starts, and the page every other one is reached from.
pub const GUIDE: &str = "guide/index.md";

/// The generated command-line reference, which the commands the docs name
/// are checked against.
pub const REFERENCE: &str = "reference/cli.md";

/// A page rendered: its title, its body as HTML, the docs it links to,
/// relative to `docs/`, and its sections for "On this page".
#[derive(Debug, PartialEq, Eq)]
pub struct Rendered {
    pub title: String,
    pub body: String,
    pub links: Vec<String>,
    /// Each second-level heading: its anchor and its text.
    pub sections: Vec<(String, String)>,
}

/// A heading's anchor: its words in lower case, joined by hyphens.
fn slug(text: &str) -> String {
    let mut slug = String::new();
    for char in text.chars() {
        if char.is_alphanumeric() {
            slug.extend(char.to_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// Renders `markdown`, the page at `page` under `docs/`: its links to other
/// pages go to their HTML, links to other docs to the repository, its first
/// heading is its title, its sections get anchors, and its code blocks a
/// bar with their language and a copy button.
pub fn render(page: &str, markdown: &str) -> Rendered {
    let mut title = None;
    let mut heading: Option<(HeadingLevel, String)> = None;
    let mut links = Vec::new();
    let mut sections = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_GFM) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level, String::new()));
            }
            Event::End(TagEnd::Heading(level)) => {
                let text = heading.take().map(|(_, text)| text).unwrap_or_default();
                if level == HeadingLevel::H1 && title.is_none() {
                    title = Some(text.clone());
                    events.push(Event::Html(CowStr::from(format!("<h1>{}</h1>", escape(&text)))));
                } else {
                    let id = slug(&text);
                    if level == HeadingLevel::H2 {
                        sections.push((id.clone(), text.clone()));
                    }
                    let tag = match level {
                        HeadingLevel::H1 | HeadingLevel::H2 => "h2",
                        HeadingLevel::H3 => "h3",
                        _ => "h4",
                    };
                    events.push(Event::Html(CowStr::from(format!("<{tag} id=\"{id}\">{}</{tag}>", escape(&text)))));
                }
            }
            Event::Text(text) | Event::Code(text) if heading.is_some() => {
                if let Some((_, heading)) = &mut heading {
                    heading.push_str(&text);
                }
            }
            _ if heading.is_some() => {}
            Event::Start(Tag::Link { link_type, dest_url, title: link_title, id }) => {
                let dest_url = match local_target(page, &dest_url) {
                    Some((target, html)) => {
                        links.push(target);
                        CowStr::from(html)
                    }
                    None => dest_url,
                };
                events.push(Event::Start(Tag::Link { link_type, dest_url, title: link_title, id }));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match &kind {
                    CodeBlockKind::Fenced(info) => info.split_whitespace().next().unwrap_or("").to_owned(),
                    CodeBlockKind::Indented => String::new(),
                };
                events.push(Event::Html(CowStr::from(format!(
                    "<div class=\"codeblock\"><div class=\"codebar\"><span>{}</span>{COPY_BUTTON}</div>",
                    escape(&language)
                ))));
                events.push(Event::Start(Tag::CodeBlock(kind)));
            }
            Event::End(TagEnd::CodeBlock) => {
                events.push(Event::End(TagEnd::CodeBlock));
                events.push(Event::Html(CowStr::from("</div>")));
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                let tone = match kind {
                    Some(BlockQuoteKind::Warning | BlockQuoteKind::Caution) => "warn",
                    Some(BlockQuoteKind::Tip) => "tip",
                    _ => "info",
                };
                events.push(Event::Html(CowStr::from(format!("<aside class=\"callout {tone}\">{CALLOUT_ICON}<div>"))));
            }
            Event::End(TagEnd::BlockQuote(_)) => events.push(Event::Html(CowStr::from("</div></aside>"))),
            Event::Start(Tag::Table(alignments)) => {
                events.push(Event::Html(CowStr::from("<div class=\"table\">")));
                events.push(Event::Start(Tag::Table(alignments)));
            }
            Event::End(TagEnd::Table) => {
                events.push(Event::End(TagEnd::Table));
                events.push(Event::Html(CowStr::from("</div>")));
            }
            other => events.push(other),
        }
    }
    let mut body = String::new();
    html::push_html(&mut body, events.into_iter());
    Rendered { title: title.unwrap_or_else(|| page.to_owned()), body, links, sections }
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

/// The group a page is listed under.
fn group_of(page: &str) -> &'static str {
    GROUPS.iter().find(|(_, pages)| pages.contains(&page)).map_or("Docs", |(name, _)| name)
}

/// The site's sections in the docs' top bar: the group each stands for, its label and its first page.
const SECTIONS: &[(&str, &str, &str)] = &[
    ("Guide", "Guides", "guide/index.html"),
    ("Running a server", "Self-hosting", "self-hosting.html"),
    ("Reference", "CLI reference", "reference/cli.html"),
];

/// The docs' top bar: the mark, the sections with the page's own marked, the source and installing.
fn topbar(up: &str, group: &str) -> String {
    let sections: String = SECTIONS
        .iter()
        .map(|(of, label, href)| {
            let current = if *of == group { " aria-current=\"true\"" } else { "" };
            format!("<a href=\"{up}{href}\"{current}>{label}</a>")
        })
        .collect();
    format!(
        "<header class=\"topbar\"><div class=\"bar\"><a class=\"brand\" href=\"{up}index.html\">{MARK}silt <b>docs</b></a>\
         <nav aria-label=\"site\">{sections}<a href=\"{REPOSITORY}/blob/main/CHANGELOG.md\">Changelog</a></nav>\
         <span class=\"right\"><a href=\"{REPOSITORY}\">Source</a><a class=\"button\" href=\"{up}guide/install.html\">Install</a></span></div></header>"
    )
}

/// Silt's mark, three waves, as the boards draw it beside the name.
const MARK: &str = "<svg class=\"mark\" viewBox=\"0 0 20 20\" width=\"20\" height=\"20\" aria-hidden=\"true\">\
<path d=\"M2 6c3-2 5 2 8 0s5-2 8 0\"/><path d=\"M2 10c3-2 5 2 8 0s5-2 8 0\" opacity=\".7\"/><path d=\"M2 14c3-2 5 2 8 0s5-2 8 0\" opacity=\".4\"/></svg>";

const COPY_BUTTON: &str = "<button type=\"button\" class=\"copy\"><svg viewBox=\"0 0 16 16\" width=\"14\" height=\"14\" aria-hidden=\"true\">\
<rect x=\"5\" y=\"5\" width=\"8.5\" height=\"8.5\" rx=\"1.5\"/><path d=\"M10.5 5V3.5A1.5 1.5 0 0 0 9 2H3.5A1.5 1.5 0 0 0 2 3.5V9a1.5 1.5 0 0 0 1.5 1.5H5\"/></svg><span>Copy</span></button>";

const CALLOUT_ICON: &str = "<svg class=\"icon\" viewBox=\"0 0 16 16\" width=\"16\" height=\"16\" aria-hidden=\"true\">\
<circle cx=\"8\" cy=\"8\" r=\"6.5\"/><path d=\"M8 7.5v4M8 5v.01\"/></svg>";

/// Nous's faces. The app bundles them; the site is plain files anyone can
/// host, so it takes them from Google Fonts rather than shipping binaries.
const FONTS: &str = "<link rel=\"preconnect\" href=\"https://fonts.gstatic.com\" crossorigin>\
<link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=IBM+Plex+Mono:wght@400;500&family=IBM+Plex+Sans:wght@400;500;600&display=swap\">\n";

/// Copies a code block's text from its button, folds the pages away on a
/// phone, and marks the section being read in "On this page".
const SCRIPT: &str = "<script>document.addEventListener('click',function(e){var b=e.target.closest('.copy');if(!b)return;\
var c=b.closest('.codeblock,.pill').querySelector('code'),l=b.querySelector('span');navigator.clipboard&&navigator.clipboard.writeText(c.innerText).then(function(){\
l.textContent='Copied';setTimeout(function(){l.textContent='Copy'},1500)})});\
var p=document.querySelector('.pages');if(p&&matchMedia('(max-width:760px)').matches)p.open=false;\
var t=[].slice.call(document.querySelectorAll('.toc a'));function m(){var c=t[0];t.forEach(function(a){var h=document.getElementById(a.hash.slice(1));\
if(h&&h.getBoundingClientRect().top<120)c=a});t.forEach(function(a){if(a===c)a.setAttribute('aria-current','location');else a.removeAttribute('aria-current')})}\
if(t.length){addEventListener('scroll',m,{passive:true});m()}</script>";

/// The page's HTML document: the top bar, the grouped sidebar, the page,
/// its neighbours, and what's on it.
pub fn document(page: &str, rendered: &Rendered, titles: &[(String, String)]) -> String {
    let depth = page.matches('/').count();
    let up = "../".repeat(depth);
    let title_of = |wanted: &str| titles.iter().find(|(other, _)| other == wanted).map(|(_, title)| title.as_str());
    let link = |other: &str, title: &str| {
        let href = format!("{up}{}", other.replace(".md", ".html"));
        let current = if other == page { " aria-current=\"page\"" } else { "" };
        format!("<li><a href=\"{href}\"{current}>{}</a></li>", escape(title))
    };
    let mut nav = String::new();
    let mut listed: Vec<&str> = Vec::new();
    for (group, pages) in GROUPS {
        let items: String = pages
            .iter()
            .filter_map(|other| title_of(other).map(|title| (other, title)))
            .map(|(other, title)| {
                listed.push(other);
                link(other, title)
            })
            .collect();
        if !items.is_empty() {
            nav.push_str(&format!("<p class=\"group\">{}</p><ul>{items}</ul>", escape(group)));
        }
    }
    let others: String =
        titles.iter().filter(|(other, _)| !listed.contains(&other.as_str())).map(|(other, title)| link(other, title)).collect();
    if !others.is_empty() {
        nav.push_str(&format!("<ul>{others}</ul>"));
    }
    let at = titles.iter().position(|(other, _)| other == page);
    let neighbour = |index: Option<usize>, rel: &str, label: &str| {
        index.and_then(|index| titles.get(index)).map_or(String::new(), |(other, title)| {
            format!(
                "<a class=\"{rel}\" rel=\"{rel}\" href=\"{up}{}\"><span>{label}</span>{}</a>",
                other.replace(".md", ".html"),
                escape(title)
            )
        })
    };
    let pager = format!(
        "<nav class=\"pager\" aria-label=\"pages\">{}{}</nav>",
        neighbour(at.and_then(|at| at.checked_sub(1)), "prev", "← Previous"),
        neighbour(at.map(|at| at + 1), "next", "Next →"),
    );
    let group = group_of(page);
    let (group_label, group_href) =
        SECTIONS.iter().find(|(of, _, _)| *of == group).map_or(("Docs", "guide/index.html"), |(_, label, href)| (*label, *href));
    let toc = if rendered.sections.is_empty() {
        String::new()
    } else {
        let items: String =
            rendered.sections.iter().map(|(id, text)| format!("<li><a href=\"#{id}\">{}</a></li>", escape(text))).collect();
        format!("<aside class=\"toc\" aria-label=\"on this page\"><p class=\"group\">On this page</p><ul>{items}</ul></aside>")
    };
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{title} · Silt</title>\n{FONTS}<style>{NOUS}{STYLE}</style></head>\n<body class=\"docs\">{topbar}\n\
         <div class=\"layout\"><nav class=\"sidebar\" aria-label=\"docs\"><details class=\"pages\" open><summary>Pages</summary>{nav}</details></nav>\n\
         <main><article><nav class=\"crumb\" aria-label=\"breadcrumb\"><a href=\"{up}{group_href}\">{group_label}</a><span aria-hidden=\"true\">/</span>\
         <span aria-current=\"page\">{title}</span></nav>{body}{pager}\
         <p class=\"edit\"><a href=\"{REPOSITORY}/edit/main/docs/{page}\">Edit this page</a></p></article></main>{toc}</div>{SCRIPT}</body></html>\n",
        title = escape(&rendered.title),
        topbar = topbar(&up, group),
        body = rendered.body,
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Builds the site from `docs` into `out`. It fails on a link to a doc that
/// isn't there, a guide page the site leaves out or can't reach, and a
/// `silt` command the reference doesn't have. Returns the files written.
pub fn build(docs: &Path, out: &Path) -> Result<Vec<PathBuf>, SiteError> {
    let mut pages = Vec::new();
    let mut markdowns = Vec::new();
    for page in PAGES {
        let markdown = read(&docs.join(page))?;
        pages.push((*page, render(page, &markdown)));
        markdowns.push((*page, markdown));
    }
    for (page, rendered) in &pages {
        if let Some(target) = rendered.links.iter().find(|target| !docs.join(target).is_file()) {
            return Err(SiteError::BrokenLink { page: (*page).to_owned(), target: target.clone() });
        }
    }
    check_guide_is_in_the_site(docs)?;
    let links: Vec<(&str, &[String])> = pages.iter().map(|(page, rendered)| (*page, &rendered.links[..])).collect();
    if let Some(page) = check::unreachable(&links, GUIDE).first() {
        return Err(SiteError::Unreachable { page: (*page).to_owned() });
    }
    let known = check::reference_commands(&read(&docs.join(REFERENCE))?);
    for (page, markdown) in markdowns.iter().filter(|(page, _)| *page != REFERENCE) {
        check_commands(page, markdown, &known)?;
    }
    let titles: Vec<(String, String)> =
        pages.iter().map(|(page, rendered)| ((*page).to_owned(), rendered.title.clone())).collect();
    let mut written = Vec::new();
    for (page, rendered) in &pages {
        let path = out.join(page.replace(".md", ".html"));
        write(&path, &document(page, rendered, &titles))?;
        written.push(path);
    }
    let markdown = read(&docs.join(LANDING))?;
    check_commands(LANDING, &markdown, &known)?;
    let landing = Landing::read(&markdown);
    if let Some(target) = landing.links().iter().find(|target| !docs.join(target).is_file()) {
        return Err(SiteError::BrokenLink { page: LANDING.to_owned(), target: target.clone() });
    }
    let index = out.join("index.html");
    write(&index, &landing_document(&landing))?;
    written.push(index);
    Ok(written)
}

/// `landing.md` cut into the parts the Landing board lays out: the hero
/// (its title, first paragraph and links), and each `##` section as a feature.
pub struct Landing {
    pub hero: Rendered,
    pub features: Vec<Rendered>,
}

impl Landing {
    pub fn read(markdown: &str) -> Landing {
        let mut chunks = markdown.split("\n## ");
        let hero = render(LANDING, chunks.next().unwrap_or(""));
        let features = chunks.map(|chunk| render(LANDING, &format!("# {chunk}"))).collect();
        Landing { hero, features }
    }

    fn links(&self) -> Vec<String> {
        self.features.iter().chain([&self.hero]).flat_map(|part| part.links.iter().cloned()).collect()
    }
}

const FEATURE_COLUMNS: usize = 3;

/// A list whose items are ticked, as the board's checklists are.
fn checklist(items: &[&str]) -> String {
    let items: String = items.iter().map(|item| format!("<li>{item}</li>")).collect();
    format!("<ul class=\"checks\">{items}</ul>")
}

/// The landing page (ADR 0033, the Landing page board), the site's root.
/// Copy marked as a placeholder stands in until the docs for people and agents write it.
pub fn landing_document(landing: &Landing) -> String {
    let mut features: String = landing
        .features
        .iter()
        .map(|feature| {
            format!(
                "<article class=\"feature\">{}</article>",
                feature.body.replacen("<h1>", "<h3>", 1).replacen("</h1>", "</h3>", 1)
            )
        })
        .collect();
    let short = (FEATURE_COLUMNS - landing.features.len() % FEATURE_COLUMNS) % FEATURE_COLUMNS;
    features.push_str(&"<div class=\"feature blank\" aria-hidden=\"true\"></div>".repeat(short));
    let hero = landing.hero.body.replacen("<h1>", "<h1 class=\"display\">", 1);
    let install = format!(
        "<div class=\"pill\"><span class=\"prompt\" aria-hidden=\"true\">$</span><code>cargo install --locked --git {REPOSITORY} silt-cli</code>{COPY_BUTTON}</div>"
    );
    format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Silt · {title}</title>\n{FONTS}<style>{NOUS}{STYLE}</style></head>\n\
         <body class=\"landing\"><header class=\"topbar\"><div class=\"bar wrap\"><a class=\"brand\" href=\"index.html\">{MARK}silt</a>\
         <nav aria-label=\"site\"><a href=\"#how\">How it works</a><a href=\"#features\">Features</a><a href=\"#self-host\">Self-host</a>\
         <a href=\"guide/index.html\">Docs</a></nav><span class=\"right\"><a href=\"{REPOSITORY}\">Source</a>\
         <a class=\"button\" href=\"guide/index.html\">Get started</a></span></div></header>\n\
         <main>\
         <section class=\"hero wrap\"><p class=\"eyebrow\"><span class=\"dot\"></span>Open source · AGPL-3.0 · self-hosted</p>{hero}\
         <!-- placeholder: until there are releases -->{install}</section>\n\
         <section id=\"how\" class=\"band wrap\"><p class=\"eyebrow\">How it works</p><h2>Leave your desk. The thread comes with you.</h2>\
         <!-- placeholder -->\
         <ol class=\"steps\">\
         <li><span class=\"label\">1 · Laptop</span><h3>Work as you do now</h3><p>Start your agent through Silt, or record a session already running.</p><pre><code>silt claude</code></pre></li>\
         <li><span class=\"label\">2 · Workstation</span><h3>Pick it up somewhere else</h3><p>Resume on another machine with the code and the conversation as you left them.</p><pre><code>silt resume &lt;thread&gt;</code></pre></li>\
         <li><span class=\"label\">3 · A fork</span><h3>Try something else</h3><p>Fork from any turn; the original carries on.</p><pre><code>silt fork &lt;thread&gt;</code></pre></li>\
         <li><span class=\"label\">4 · Review</span><h3>Merge it back with approval</h3><p>Offer the fork as a merge request, reviewed beside the conversation that made it.</p><pre><code>silt mr create</code></pre></li>\
         </ol></section>\n\
         <section class=\"agents wrap\"><!-- placeholder --><p class=\"eyebrow\">Works with</p><ul><li>Claude Code</li><li>Codex</li><li>Gemini CLI</li><li>OpenCode</li><li>Copilot CLI</li></ul></section>\n\
         <section id=\"features\" class=\"band wrap\"><p class=\"eyebrow\">What you get</p><h2>Version control for agent work, not just its output.</h2>\
         <div class=\"features\">{features}</div></section>\n\
         <section class=\"band wrap split\"><!-- placeholder --><div><p class=\"eyebrow\">Agents from the browser</p><h2>Answer your agents from anywhere.</h2>\
         <p>Your agents keep running on your machines. The web app, on a laptop or a phone, shows what each is doing and what it waits on.</p>{browser}</div>\
         <div class=\"ask\" aria-hidden=\"true\"><span class=\"label\"><span class=\"dot warn\"></span>Agent is asking</span><code>Edit src/retry.rs</code>\
         <span class=\"meta\">claude · Retry uploads · on workstation</span><span class=\"buttons\"><span class=\"button\">Allow</span><span class=\"button secondary\">Reject</span></span></div></section>\n\
         <section id=\"self-host\" class=\"band wrap split\"><!-- placeholder --><div><p class=\"eyebrow\">Self-host</p><h2>One binary, your server.</h2>\
         <p>The sync server and your machine’s daemon are the same binary. Run the server where your team can reach it; read <a href=\"self-hosting.html\">Self-hosting</a>.</p>{server}</div>\
         <pre class=\"term\"><code>silt server init --data /var/lib/silt --public-url https://silt.example.com\nsilt server run --data /var/lib/silt</code></pre></section>\n\
         <section class=\"band wrap\"><!-- placeholder --><p class=\"eyebrow\">Where it is</p><h2>Early, and used every day.</h2><div class=\"status\">\
         <div><h3><span class=\"dot ok\"></span>Works today</h3>{today}</div><div><h3><span class=\"dot info\"></span>Next</h3>{next}</div>\
         <div><h3><span class=\"dot\"></span>Later</h3>{later}</div></div></section>\n\
         <section class=\"cta wrap\"><!-- placeholder --><h2>Start a thread you can pick up anywhere.</h2>{install}\
         <p><a class=\"button\" href=\"guide/index.html\">Get started</a><a class=\"button secondary\" href=\"{REPOSITORY}\">Read the source</a></p></section>\
         </main>\n<footer><div class=\"wrap\"><a class=\"brand\" href=\"index.html\">{MARK}silt</a>\
         <nav aria-label=\"footer\"><a href=\"guide/index.html\">Docs</a><a href=\"self-hosting.html\">Self-hosting</a><a href=\"reference/cli.html\">CLI reference</a>\
         <a href=\"{REPOSITORY}\">Source</a><a href=\"{REPOSITORY}/blob/main/CHANGELOG.md\">Changelog</a></nav>\
         <span class=\"meta\">AGPL-3.0</span></div></footer>{SCRIPT}</body></html>\n",
        title = escape(&landing.hero.title),
        browser = checklist(&[
            "Allow or reject what an agent asks, in place",
            "Review a merge request a file at a time",
            "Steer a running agent, or queue the next prompt",
            "Hear when an agent needs you",
        ]),
        server = checklist(&[
            "Sign in with GitHub, OIDC or a password",
            "Orgs, teams and who sees each thread",
            "A systemd unit written for you",
            "Your history stays on your disks",
        ]),
        today = checklist(&[
            "Hand-off between machines",
            "Forks and merge requests",
            "Search across compaction, and the history graph",
            "Several servers, and projects kept on one machine",
            "The web app on phones",
        ]),
        next = checklist(&["Faster recording and smaller storage", "Docs for people and agents", "A desktop app"]),
        later = checklist(&["Capture you can rely on", "Your history from day one", "Git and forge interop", "macOS"]),
    )
}

/// Fails on the first `silt` command `markdown`, the doc at `page`, names
/// that isn't among the `known` commands of the reference.
pub fn check_commands(page: &str, markdown: &str, known: &BTreeSet<Vec<String>>) -> Result<(), SiteError> {
    match check::named_commands(markdown).into_iter().find(|words| !check::is_documented(words, known)) {
        Some(words) => Err(SiteError::UnknownCommand { page: page.to_owned(), command: words.join(" ") }),
        None => Ok(()),
    }
}

fn check_guide_is_in_the_site(docs: &Path) -> Result<(), SiteError> {
    let guide = docs.join("guide");
    let entries = fs::read_dir(&guide).map_err(|source| SiteError::Read { path: guide.clone(), source })?;
    for entry in entries {
        let entry = entry.map_err(|source| SiteError::Read { path: guide.clone(), source })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let page = format!("guide/{name}");
        if name.ends_with(".md") && !PAGES.contains(&page.as_str()) {
            return Err(SiteError::NotAPage { page });
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, SiteError> {
    fs::read_to_string(path).map_err(|source| SiteError::Read { path: path.to_owned(), source })
}

fn write(path: &Path, text: &str) -> Result<(), SiteError> {
    let failed = |source| SiteError::Write { path: path.to_owned(), source };
    fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(failed)?;
    fs::write(path, text).map_err(|source| SiteError::Write { path: path.to_owned(), source })
}

/// Nous's tokens, the same file the web app styles itself with (ADR 0039).
const NOUS: &str = include_str!("../../../web/src/nous.css");

/// The site's own rules, on Nous's tokens, as the Docs and Landing page boards draw them.
const STYLE: &str = r#"
*{box-sizing:border-box}
body{margin:0;font-size:14px}
a{color:var(--accent-fg)}a:hover{color:var(--fg-1)}
code,pre{font-family:var(--font-mono)}pre code{font-size:inherit}
:not(pre)>code{padding:1px 5px;font-size:.9em;background:var(--surface-1);border:1px solid var(--border-subtle);border-radius:2px}
svg{fill:none;stroke:currentColor;stroke-width:1.5;stroke-linecap:round;stroke-linejoin:round}
.mark{stroke:var(--accent);stroke-width:1.6}
.dot{display:inline-block;width:6px;height:6px;border-radius:50%;background:var(--fg-3)}.dot.ok{background:var(--ok)}.dot.info{background:var(--info)}.dot.warn{background:var(--warn)}
.topbar{position:sticky;top:0;z-index:2;background:var(--bg);border-bottom:1px solid var(--border)}
.bar{display:flex;align-items:center;gap:8px 24px;height:56px;padding:0 24px;font-size:13px}
.topbar nav{display:flex;gap:4px 20px}.topbar nav a,.topbar .right a:not(.button){color:var(--fg-2);text-decoration:none}
.topbar nav a:hover,.topbar .right a:not(.button):hover{color:var(--fg-1)}.topbar nav a[aria-current]{color:var(--fg-1);font-weight:500}
.topbar .right{margin-left:auto;display:flex;align-items:center;gap:16px}
.brand{display:inline-flex;align-items:center;gap:8px;font-weight:500;font-size:17px;letter-spacing:-.04em;color:var(--fg-1)!important;text-decoration:none}
.brand b{font-weight:400;color:var(--fg-2);letter-spacing:0;font-size:14px}
.button{display:inline-flex;align-items:center;justify-content:center;gap:6px;min-height:32px;padding:0 12px;font:500 13px var(--font-sans);color:var(--fg-on-accent)!important;background:var(--accent);border:1px solid transparent;border-radius:2px;text-decoration:none;cursor:pointer}
.button:hover{background:var(--accent-hover)}.button.secondary{color:var(--fg-1)!important;background:var(--surface-1);border-color:var(--border)}.button.secondary:hover{background:var(--surface-2)}
.group{margin:20px 0 6px;font-family:var(--font-mono);font-size:11px;font-weight:500;letter-spacing:.06em;text-transform:uppercase;color:var(--fg-2)}
.layout{display:grid;grid-template-columns:272px minmax(0,1fr) 240px}
.sidebar{padding:4px 16px 32px;border-right:1px solid var(--border);position:sticky;top:57px;align-self:start;height:calc(100vh - 57px);overflow-y:auto}
.sidebar ul,.toc ul{list-style:none;margin:0;padding:0}
.sidebar a{display:block;padding:5px 10px;border-radius:2px;color:var(--fg-2);text-decoration:none;font-size:13px;line-height:1.4}
.sidebar a:hover{background:var(--surface-2);color:var(--fg-1)}
.sidebar a[aria-current]{color:var(--fg-1);font-weight:500;box-shadow:inset 2px 0 var(--accent);border-radius:0}
.pages>summary{display:none}
.docs main{min-width:0;padding:40px 48px 64px}
.docs article{max-width:720px;margin:0 auto;font-size:15px;line-height:1.7}
.crumb{display:flex;flex-wrap:wrap;gap:6px;margin:0 0 12px;font-size:13px;color:var(--fg-2)}.crumb a{color:var(--fg-2);text-decoration:none}.crumb a:hover{color:var(--fg-1)}.crumb [aria-current]{color:var(--fg-1)}
.docs h1{font-size:36px;font-weight:500;letter-spacing:-.02em;line-height:1.15;margin:0 0 16px}
.docs h1+p{font-size:18px;line-height:1.6;color:var(--fg-2)}
.docs h2{font-size:22px;font-weight:500;letter-spacing:-.01em;line-height:1.3;margin:48px 0 12px;scroll-margin-top:72px}
.docs h3{font-size:17px;font-weight:500;margin:32px 0 8px;scroll-margin-top:72px}
.callout{display:grid;grid-template-columns:20px minmax(0,1fr);gap:10px;margin:20px 0;padding:14px 16px;border:1px solid transparent;border-radius:4px;font-size:14px;line-height:1.6;background:var(--info-tint)}
.callout .icon{margin-top:3px;color:var(--info)}.callout.warn{background:var(--warn-tint)}.callout.warn .icon{color:var(--warn)}.callout.tip{background:var(--ok-tint)}.callout.tip .icon{color:var(--ok)}
.callout p{margin:0}.callout p+p{margin-top:8px}
.table{margin:20px 0;overflow-x:auto}
table{border-collapse:collapse;font-size:14px;line-height:1.5;min-width:100%}
th,td{padding:8px 12px 8px 0;text-align:left;vertical-align:top;border-bottom:1px solid var(--border)}
th{font-weight:500;border-bottom-color:var(--border-strong)}td:first-child{font-weight:500;white-space:nowrap}
.codeblock,.steps pre,.term{color:#efe6d6;background:var(--tobacco-900);border:1px solid light-dark(transparent,var(--border));border-radius:4px}
.codeblock{margin:20px 0;overflow:hidden}
.codebar{display:flex;align-items:center;gap:8px;padding:4px 6px 4px 14px;border-bottom:1px solid #3a3127;font-family:var(--font-mono);font-size:11px;color:#b3a68f}
.copy{display:inline-flex;align-items:center;gap:6px;margin-left:auto;padding:4px 8px;font:500 12px var(--font-sans);color:#b3a68f;background:transparent;border:1px solid transparent;border-radius:2px;cursor:pointer}
.copy:hover{color:#efe6d6;background:#2e2620}
.codeblock pre{margin:0;padding:14px;overflow-x:auto;font-size:13px;line-height:1.6}
.pager{display:flex;gap:12px;margin-top:56px}
.pager a{flex:1;display:flex;flex-direction:column;gap:2px;padding:14px 16px;border:1px solid var(--border);border-radius:4px;text-decoration:none;color:var(--fg-1);font-weight:500}
.pager a:hover{background:var(--surface-1)}.pager a span{font-size:12px;font-weight:400;color:var(--fg-2)}.pager .next{text-align:right}
.edit{margin-top:32px;padding-top:16px;border-top:1px solid var(--border);font-size:13px}.edit a{color:var(--fg-2);text-decoration:none}.edit a:hover{color:var(--fg-1)}
.toc{padding:36px 24px 16px 0;position:sticky;top:57px;align-self:start;font-size:13px}
.toc ul{border-left:1px solid var(--border)}
.toc a{display:block;margin-left:-1px;padding:4px 0 4px 12px;border-left:2px solid transparent;color:var(--fg-2);text-decoration:none}
.toc a:hover{color:var(--fg-1)}.toc a[aria-current]{color:var(--fg-1);border-left-color:var(--accent)}
@media(max-width:1100px){.layout{grid-template-columns:240px minmax(0,1fr)}.toc{display:none}}
.wrap{max-width:1200px;margin-left:auto;margin-right:auto;padding-left:24px;padding-right:24px}
.landing .bar{height:64px}
.eyebrow{display:flex;align-items:center;gap:8px;margin:0 0 12px;font-family:var(--font-mono);font-size:11px;letter-spacing:.08em;text-transform:uppercase;color:var(--accent-fg)}
.eyebrow .dot{background:var(--ok)}
.hero{padding-top:112px;padding-bottom:80px}
.display{max-width:16ch;font-size:clamp(44px,8vw,88px);font-weight:500;line-height:1.02;letter-spacing:-.03em;margin:0 0 24px}
.hero .display+p{max-width:620px;font-size:18px;line-height:1.6;color:var(--fg-2)}
.hero .display+p+p{display:flex;flex-wrap:wrap;gap:10px;margin:28px 0;font-size:0}
.hero .display+p+p a{display:inline-flex;align-items:center;min-height:40px;padding:0 16px;font:500 14px var(--font-sans);text-decoration:none;border:1px solid var(--border);border-radius:2px;color:var(--fg-1);background:var(--surface-1)}
.hero .display+p+p a:first-child{color:var(--fg-on-accent);background:var(--accent);border-color:transparent}
.pill{display:flex;align-items:center;gap:10px;max-width:560px;padding:4px 4px 4px 14px;font-size:13px;background:var(--bg-elevated);border:1px solid var(--border);border-radius:2px}
.pill code{flex:1;min-width:0;padding:6px 0;background:none;border:0;word-break:break-all}.pill .prompt{font-family:var(--font-mono);color:var(--accent-fg)}
.pill .copy{color:var(--fg-2)}.pill .copy:hover{color:var(--fg-1);background:var(--surface-2)}
.band{padding-top:96px;padding-bottom:96px;border-top:1px solid var(--border)}
.band h2{max-width:18ch;font-size:clamp(28px,4vw,40px);font-weight:500;letter-spacing:-.02em;line-height:1.12;margin:0 0 40px}
.steps{list-style:none;margin:0;padding:0;display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:32px}
.steps li{display:flex;flex-direction:column;gap:8px}
.label{font-family:var(--font-mono);font-size:11px;color:var(--fg-2);letter-spacing:.04em}
.steps h3,.feature h3,.status h3{font-size:17px;font-weight:500;margin:0}.steps p{margin:0 0 8px;color:var(--fg-2);font-size:14px;line-height:1.6}
.steps pre,.term,.feature .codeblock pre{white-space:pre-wrap;word-break:break-word}
.steps pre,.term{margin:auto 0 0;padding:12px 14px;overflow-x:auto;font-size:13px;line-height:1.6}
.agents{padding-bottom:64px}.agents .eyebrow{color:var(--fg-2)}.agents ul{list-style:none;margin:0;padding:0;display:flex;flex-wrap:wrap;gap:8px}
.agents li{font-family:var(--font-mono);font-size:13px;padding:6px 12px;background:var(--bg-elevated);border:1px solid var(--border);border-radius:2px}
.features{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:1px;background:var(--border);border:1px solid var(--border);border-radius:4px;overflow:hidden}
.feature{padding:28px;background:var(--bg);font-size:14px;line-height:1.65}.feature p{color:var(--fg-2)}.feature .codeblock{margin:16px 0 0}
.feature ul{padding-left:18px;color:var(--fg-2)}
.split{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:48px;align-items:center}.split>div>p:not(.eyebrow){color:var(--fg-2);font-size:15px;line-height:1.65;max-width:520px}
.checks{list-style:none;margin:20px 0 0;padding:0;display:flex;flex-direction:column;gap:10px;font-size:14px}
.checks li{display:flex;gap:10px}.checks li::before{content:'';flex:none;width:12px;height:6px;margin-top:6px;border-left:1.5px solid var(--ok);border-bottom:1.5px solid var(--ok);transform:rotate(-45deg)}
.ask{display:flex;flex-direction:column;gap:10px;max-width:380px;justify-self:center;width:100%;padding:16px;background:var(--bg-elevated);border:1px solid var(--border);border-radius:8px;box-shadow:var(--shadow-2)}
.ask .label{display:flex;align-items:center;gap:6px;text-transform:uppercase}.ask code{font-size:15px;font-weight:500}.ask .meta{font-family:var(--font-mono);font-size:12px;color:var(--fg-2)}
.ask .buttons{display:grid;grid-template-columns:1fr 1fr;gap:8px}.ask .button{min-height:44px;font-size:14px;cursor:default}
.status{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:32px}.status h3{display:flex;align-items:center;gap:8px}.status .checks li::before{display:none}.status .checks{color:var(--fg-2)}
.cta{padding-top:112px;padding-bottom:112px;border-top:1px solid var(--border);text-align:center}
.cta h2{max-width:20ch;margin:0 auto 32px;font-size:clamp(32px,5vw,56px);font-weight:500;letter-spacing:-.02em;line-height:1.08}
.cta .pill{margin:0 auto 24px;text-align:left}.cta p{display:flex;justify-content:center;gap:10px}.cta .button{min-height:40px;padding:0 16px;font-size:14px}
footer{border-top:1px solid var(--border);font-size:13px;color:var(--fg-2)}
footer .wrap{display:flex;flex-wrap:wrap;align-items:center;gap:12px 24px;padding-top:32px;padding-bottom:32px}
footer nav{display:flex;flex-wrap:wrap;gap:4px 20px}footer nav a{color:var(--fg-2);text-decoration:none}footer nav a:hover{color:var(--fg-1)}
footer .meta{margin-left:auto;font-family:var(--font-mono);font-size:11px}
@media(max-width:900px){.steps{grid-template-columns:repeat(2,minmax(0,1fr))}.features,.status,.split{grid-template-columns:minmax(0,1fr)}.feature.blank{display:none}}
@media(max-width:760px){.layout{display:block}.sidebar{position:static;height:auto;padding:0 16px;border-right:0;border-bottom:1px solid var(--border)}
.pages>summary{display:flex;align-items:center;gap:8px;min-height:44px;font-size:14px;font-weight:500;cursor:pointer;list-style:none}
.pages>summary::-webkit-details-marker{display:none}.pages>summary::after{content:'';width:6px;height:6px;margin-left:auto;border-right:1.5px solid var(--fg-2);border-bottom:1.5px solid var(--fg-2);transform:rotate(45deg)}
.pages[open]>summary::after{transform:rotate(-135deg)}.pages[open]{padding-bottom:12px}
.docs main{padding:24px 16px 48px}.docs h1{font-size:30px}.bar,.wrap{padding-left:16px;padding-right:16px}
.docs .topbar nav{order:3;width:100%;overflow-x:auto;padding-bottom:4px}.docs .bar{flex-wrap:wrap;height:auto;padding-top:6px}
.landing .topbar nav,.landing .topbar .right a:not(.button){display:none}
.steps{grid-template-columns:minmax(0,1fr)}.hero{padding-top:56px;padding-bottom:56px}.band{padding-top:64px;padding-bottom:64px}
.sidebar a,.toc a,.topbar a,.copy,.button,footer nav a,.edit a,.crumb a,.brand{min-height:44px;display:flex;align-items:center}
.topbar nav a,.copy,.button{display:inline-flex}.hero .display+p+p a{min-height:44px}footer .meta{margin-left:0}}
"#;

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
    fn sections_get_anchors_and_code_blocks_a_copy_button() {
        let rendered = render("guide/handoff.md", "# Hand off\n\n## On the laptop, first\n\n```sh\nsilt claude\n```\n");
        assert_eq!(rendered.sections, [("on-the-laptop-first".to_owned(), "On the laptop, first".to_owned())]);
        assert!(rendered.body.contains("<h2 id=\"on-the-laptop-first\">On the laptop, first</h2>"), "{}", rendered.body);
        assert!(rendered.body.contains("<span>sh</span><button type=\"button\" class=\"copy\">"), "{}", rendered.body);
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
        assert!(html.contains("<p class=\"group\">Running a server</p>"), "{html}");
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
        let grouped: Vec<&str> = GROUPS.iter().flat_map(|(_, pages)| pages.iter().copied()).collect();
        assert_eq!(grouped, PAGES, "the sidebar's groups hold every page of the site, in order");
    }

    #[test]
    fn the_repositorys_own_docs_name_only_commands_the_reference_has() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let known = check::reference_commands(&fs::read_to_string(root.join("docs").join(REFERENCE)).expect("read"));
        for doc in ["README.md", "CONTRIBUTING.md", "CLAUDE.md"] {
            let markdown = fs::read_to_string(root.join(doc)).expect("read");
            check_commands(doc, &markdown, &known).expect("every command is in the reference");
        }
    }

    #[test]
    fn a_guide_page_left_out_of_the_site_fails_the_build() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let copy = tempfile::tempdir().expect("temp dir");
        copy_dir(&docs, copy.path());
        fs::write(copy.path().join("guide/lost.md"), "# Lost\n").expect("write");
        let out = tempfile::tempdir().expect("temp dir");
        let error = build(copy.path(), out.path()).expect_err("the build fails");
        assert!(matches!(error, SiteError::NotAPage { ref page } if page == "guide/lost.md"), "{error}");
    }

    #[test]
    fn a_command_the_reference_lacks_fails_the_build() {
        let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
        let copy = tempfile::tempdir().expect("temp dir");
        copy_dir(&docs, copy.path());
        let handoff = copy.path().join("guide/handoff.md");
        let text = fs::read_to_string(&handoff).expect("read");
        fs::write(&handoff, format!("{text}\nThen `silt frobnicate --now`.\n")).expect("write");
        let out = tempfile::tempdir().expect("temp dir");
        let error = build(copy.path(), out.path()).expect_err("the build fails");
        assert_eq!(error.to_string(), "guide/handoff.md names `silt frobnicate`, which isn't in the command-line reference");
    }

    fn copy_dir(from: &Path, to: &Path) {
        for entry in fs::read_dir(from).expect("read dir") {
            let entry = entry.expect("entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("type").is_dir() {
                fs::create_dir_all(&target).expect("mkdir");
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).expect("copy");
            }
        }
    }
}
