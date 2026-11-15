//! What an artifact's frame may load (ADR 0071): scripts from a few CDNs
//! and fonts from Google Fonts, fetched directly or, where the person turned
//! it on, through Silt's own vendor route, which takes only pinned files.

/// The scripts' CDNs, as the policy names them.
pub const SCRIPT_SOURCES: [&str; 5] = [
    "https://cdnjs.cloudflare.com",
    "https://cdn.jsdelivr.net/npm/",
    "https://unpkg.com",
    "https://cdn.tailwindcss.com",
    "https://code.jquery.com",
];
pub const STYLE_SOURCES: [&str; 1] = ["https://fonts.googleapis.com"];
pub const FONT_SOURCES: [&str; 1] = ["https://fonts.gstatic.com"];

/// Every host the vendor route fetches from.
pub const VENDOR_HOSTS: [&str; 7] = [
    "cdnjs.cloudflare.com",
    "cdn.jsdelivr.net",
    "unpkg.com",
    "cdn.tailwindcss.com",
    "code.jquery.com",
    "fonts.googleapis.com",
    "fonts.gstatic.com",
];

/// The policy a frame runs under: the CDNs themselves, or only `vendor`, an
/// absolute URL ending in `/` that serves them. It never allows `connect-src`.
pub fn policy(vendor: Option<&str>) -> String {
    let (scripts, styles, fonts) = match vendor {
        Some(vendor) => (vendor.to_owned(), vendor.to_owned(), vendor.to_owned()),
        None => (SCRIPT_SOURCES.join(" "), STYLE_SOURCES.join(" "), FONT_SOURCES.join(" ")),
    };
    format!(
        "default-src 'none'; script-src 'unsafe-inline' {scripts}; style-src 'unsafe-inline' {styles}; font-src data: {fonts}; img-src data: blob:; media-src data: blob:"
    )
}

/// `text` with every allowlisted CDN address going through `vendor`, an
/// absolute URL ending in `/`.
pub fn through_vendor(text: &str, vendor: &str) -> String {
    let mut out = text.to_owned();
    for host in VENDOR_HOSTS {
        out = out.replace(&format!("https://{host}/"), &format!("{vendor}{host}/"));
        out = out.replace(&format!("//{host}/"), &format!("{vendor}{host}/"));
    }
    // A protocol-relative address rewritten above may now read `https:https://`.
    out.replace(&format!("https:{vendor}"), vendor).replace(&format!("http:{vendor}"), vendor)
}

/// A stylesheet from Google Fonts with its font files' addresses relative,
/// so they too come from the vendor route that served it.
pub fn fonts_css_through_vendor(css: &str) -> String {
    css.replace("https://fonts.gstatic.com/", "../fonts.gstatic.com/")
}

/// Whether the vendor route may fetch `path` (with `query`) from `host`:
/// an allowlisted host and, for code, an exact version.
pub fn vendorable(host: &str, path: &str, query: Option<&str>) -> Result<(), String> {
    if !VENDOR_HOSTS.contains(&host) {
        return Err(format!("{host} isn't a CDN artifacts load from"));
    }
    let clean = path.starts_with('/')
        && !path.contains("..")
        && !path.contains("//")
        && path.bytes().all(|b| b.is_ascii_graphic() && b != b'\\')
        && query.is_none_or(|query| query.bytes().all(|b| b.is_ascii_graphic() && b != b'\\'));
    if !clean || path.len() > 512 || query.is_some_and(|query| query.len() > 1024) {
        return Err("that path isn't one a CDN serves".into());
    }
    let parts: Vec<&str> = path[1..].split('/').collect();
    let pinned = match host {
        "cdnjs.cloudflare.com" => {
            parts.len() >= 5 && parts[0] == "ajax" && parts[1] == "libs" && exact(parts[3]) && !parts[4].is_empty()
        }
        "cdn.jsdelivr.net" => parts.first() == Some(&"npm") && package_at_version(&parts[1..]),
        "unpkg.com" => package_at_version(&parts),
        "cdn.tailwindcss.com" => parts.first().is_some_and(|version| exact(version)),
        "code.jquery.com" => match parts.as_slice() {
            [file] => file.strip_prefix("jquery-").and_then(|rest| rest.strip_suffix(".js")).is_some_and(|rest| {
                let version = rest.trim_end_matches(".min").trim_end_matches(".slim");
                exact(version)
            }),
            ["ui", version, ..] => exact(version),
            _ => false,
        },
        "fonts.googleapis.com" => {
            matches!(parts.as_slice(), ["css2"] | ["css"]) && query.is_some_and(|query| query.contains("family="))
        }
        "fonts.gstatic.com" => parts.first() == Some(&"s") && parts.len() >= 3,
        _ => false,
    };
    if pinned { Ok(()) } else { Err(format!("{host}{path} isn't a file at an exact version")) }
}

/// `name@1.2.3` or `@scope/name@1.2.3`, then anything.
fn package_at_version(parts: &[&str]) -> bool {
    let name = match parts {
        [scope, name, ..] if scope.starts_with('@') => name,
        [name, ..] => name,
        [] => return false,
    };
    name.rsplit_once('@').is_some_and(|(package, version)| !package.is_empty() && exact(version))
}

/// An exact version: digits and dots, three parts for semver, and an
/// optional pre-release, but no range, tag or `latest`.
fn exact(version: &str) -> bool {
    let (core, pre) = version.split_once('-').map_or((version, None), |(core, pre)| (core, Some(pre)));
    let numbers: Vec<&str> = core.split('.').collect();
    (2..=4).contains(&numbers.len())
        && numbers.iter().all(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
        && pre.is_none_or(|pre| !pre.is_empty() && pre.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.'))
}

/// The viewer's theme, set on a frame's root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn parse(theme: &str) -> Option<Self> {
        match theme {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

/// The first script's opening, before the theme and the parent's origin.
pub const FIRST_SCRIPT_START: &str = "(function(){var root=document.documentElement;";

/// The rest of the first script: it follows the theme the page holding the
/// frame sends, and tells that page what text is selected, for a comment to
/// quote, what went wrong, and when the page has settled (ADR 0071).
pub const FIRST_SCRIPT_END: &str = "addEventListener('message',function(e){var d=e.data;if(e.source!==parent||!d||d.silt!=='theme')return;\
if(d.theme==='light'||d.theme==='dark')root.setAttribute('data-theme',d.theme)});\
document.addEventListener('mouseup',function(){var t=String(getSelection()||'').trim();parent.postMessage({silt:'selection',text:t.slice(0,500)},to)});\
var seen={},problems=[];\
function report(kind,text){text=String(text).slice(0,300);if(seen[kind+text]||problems.length>=20)return;seen[kind+text]=1;\
problems.push({kind:kind,text:text});parent.postMessage({silt:'problems',problems:problems.slice()},to)}\
document.addEventListener('securitypolicyviolation',function(e){report('blocked',(e.blockedURI||'inline')+' ('+e.effectiveDirective+')')});\
addEventListener('error',function(e){if(e.message)report('error',e.message+(e.lineno?' (line '+e.lineno+')':''))});\
addEventListener('unhandledrejection',function(e){var r=e.reason;report('error','Unhandled rejection: '+(r&&r.message||r))});\
addEventListener('load',function(){setTimeout(function(){var w=root.scrollWidth,v=root.clientWidth;\
if(w>v+1)report('overflow','The page is '+w+'px wide in a '+v+'px frame');parent.postMessage({silt:'checked'},to)},300)});})();";

/// Whether `origin` is a bare `http(s)://host[:port]`, safe to write into
/// the first script as the target of its messages.
pub fn is_origin(origin: &str) -> bool {
    let rest = origin.strip_prefix("https://").or_else(|| origin.strip_prefix("http://"));
    rest.is_some_and(|host| {
        !host.is_empty()
            && host.len() <= 255
            && host.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']'))
    })
}

/// The first script a frame runs: the theme on its root, and messages to
/// `parent`, the origin of the page holding it, or to any when unknown.
pub fn first_script(theme: Option<Theme>, parent: Option<&str>) -> String {
    let theme = theme.map(|theme| format!("root.setAttribute('data-theme','{}');", theme.as_str())).unwrap_or_default();
    let to = parent.filter(|origin| is_origin(origin)).unwrap_or("*");
    format!("{FIRST_SCRIPT_START}{theme}var to='{to}';{FIRST_SCRIPT_END}")
}

/// What goes into a frame ahead of the page.
#[derive(Debug, Clone, Copy)]
pub struct FrameParts<'a> {
    /// The CSS the version's design system compiled to.
    pub style: Option<&'a str>,
    pub theme: Option<Theme>,
    /// The first script; an offline copy has none.
    pub probe: bool,
    /// Silt's vendor route, an absolute URL ending in `/`, when libraries
    /// and fonts load through it.
    pub vendor: Option<&'a str>,
    /// The origin of the page holding the frame.
    pub parent: Option<&'a str>,
    /// The policy as a `<meta>`; a page served with it as a header leaves
    /// it out.
    pub policy_meta: bool,
}

impl Default for FrameParts<'_> {
    fn default() -> Self {
        Self { style: None, theme: None, probe: true, vendor: None, parent: None, policy_meta: true }
    }
}

/// A page's doctype, or the standard one when it has none, and the rest.
pub fn split_doctype(html: &str) -> (&str, &str) {
    let trimmed = html.trim_start();
    let doctype_end = trimmed
        .get(..9)
        .filter(|start| start.eq_ignore_ascii_case("<!doctype"))
        .and_then(|_| trimmed.find('>'))
        .map(|end| end + 1);
    match doctype_end {
        Some(end) => trimmed.split_at(end),
        None => ("<!doctype html>", trimmed),
    }
}

/// An HTML version as its frame loads it, on every surface: its doctype,
/// the policy, the first script and the design system's CSS ahead of
/// anything of its own, so it renders as written.
pub fn compose(html: &str, parts: &FrameParts<'_>) -> String {
    let (html, style) = match parts.vendor {
        Some(vendor) => (through_vendor(html, vendor), parts.style.map(|css| through_vendor(css, vendor))),
        None => (html.to_owned(), parts.style.map(str::to_owned)),
    };
    let (doctype, rest) = split_doctype(&html);
    let policy = if parts.policy_meta {
        format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">", policy(parts.vendor))
    } else {
        String::new()
    };
    let script =
        if parts.probe { format!("<script>{}</script>", first_script(parts.theme, parts.parent)) } else { String::new() };
    // The CSS can't close its own element, whatever a peer put in it.
    let style =
        style.map(|css| format!("<style data-silt=\"design-system\">{}</style>", css.replace("</", "<\\/"))).unwrap_or_default();
    format!("{doctype}{policy}{script}{style}{rest}")
}

/// An HTML version as a frame loads it to be checked, in `theme`.
pub fn checked_document(html: &str, style: Option<&str>, theme: Theme) -> String {
    compose(html, &FrameParts { style, theme: Some(theme), ..FrameParts::default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_names_the_cdns_or_only_the_vendor_route() {
        let direct = policy(None);
        assert!(direct.contains("script-src 'unsafe-inline' https://cdnjs.cloudflare.com https://cdn.jsdelivr.net/npm/"));
        assert!(direct.contains("font-src data: https://fonts.gstatic.com;") && !direct.contains("connect-src"));
        let proxied = policy(Some("http://127.0.0.1:7777/vendor/"));
        assert!(proxied.contains("script-src 'unsafe-inline' http://127.0.0.1:7777/vendor/;"), "{proxied}");
        assert!(!proxied.contains("cdnjs"));
    }

    #[test]
    fn only_pinned_files_of_the_allowlist_are_fetched() {
        for (host, path, query) in [
            ("cdnjs.cloudflare.com", "/ajax/libs/d3/7.9.0/d3.min.js", None),
            ("cdn.jsdelivr.net", "/npm/chart.js@4.4.7/dist/chart.umd.min.js", None),
            ("cdn.jsdelivr.net", "/npm/@observablehq/plot@0.6.16", None),
            ("unpkg.com", "/react@18.3.1/umd/react.production.min.js", None),
            ("cdn.tailwindcss.com", "/3.4.17", None),
            ("code.jquery.com", "/jquery-3.7.1.min.js", None),
            ("fonts.googleapis.com", "/css2", Some("family=Inter:wght@400;600&display=swap")),
            ("fonts.gstatic.com", "/s/inter/v13/abc.woff2", None),
        ] {
            assert!(vendorable(host, path, query).is_ok(), "{host}{path}");
        }
        for (host, path, query) in [
            ("esm.sh", "/react@18.3.1", None),
            ("unpkg.com", "/react", None),
            ("unpkg.com", "/react@latest/index.js", None),
            ("cdn.jsdelivr.net", "/npm/react@18/index.js", None),
            ("cdn.jsdelivr.net", "/npm/react@^18.3.1/index.js", None),
            ("cdnjs.cloudflare.com", "/ajax/libs/d3/../../etc/passwd", None),
            ("cdn.tailwindcss.com", "/", None),
            ("fonts.googleapis.com", "/css2", None),
        ] {
            assert!(vendorable(host, path, query).is_err(), "{host}{path}");
        }
    }

    #[test]
    fn a_checked_page_gets_the_policy_theme_and_tokens_first() {
        let page = checked_document("<!DOCTYPE html><p>hi</p>", Some(":root{--bg:#fff}</style>"), Theme::Dark);
        assert!(page.starts_with("<!DOCTYPE html><meta http-equiv=\"Content-Security-Policy\""), "{page}");
        assert!(page.contains("setAttribute('data-theme','dark')") && page.contains("'securitypolicyviolation'"));
        assert!(page.contains("var to='*';") && page.contains("silt:'checked'"));
        assert!(page.ends_with(":root{--bg:#fff}<\\/style></style><p>hi</p>"), "{page}");
    }

    #[test]
    fn messages_go_only_to_a_plain_origin() {
        assert!(first_script(None, Some("http://127.0.0.1:7777")).contains("var to='http://127.0.0.1:7777';"));
        assert!(first_script(None, Some("https://silt.example")).contains("var to='https://silt.example';"));
        for forged in ["https://a.example/'+alert(1)+'", "javascript:alert(1)", "https://", "https://a b", "file:///x"] {
            assert!(first_script(None, Some(forged)).contains("var to='*';"), "{forged}");
        }
    }

    #[test]
    fn a_page_without_a_doctype_gets_one_and_vendor_addresses_are_rewritten() {
        let vendor = "http://127.0.0.1:7777/vendor/";
        let page = compose(
            "  <script src=\"https://unpkg.com/react@18.3.1/x.js\"></script>",
            &FrameParts {
                probe: false,
                vendor: Some(vendor),
                style: Some("@import url(https://fonts.googleapis.com/css2?family=Inter);"),
                ..FrameParts::default()
            },
        );
        assert!(page.starts_with("<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline' http://127.0.0.1:7777/vendor/;"), "{page}");
        assert!(
            page.contains("url(http://127.0.0.1:7777/vendor/fonts.googleapis.com/css2")
                && page.ends_with("vendor/unpkg.com/react@18.3.1/x.js\"></script>"),
            "{page}"
        );
        let bare = compose("<p>x</p>", &FrameParts { probe: false, policy_meta: false, ..FrameParts::default() });
        assert_eq!(bare, "<!doctype html><p>x</p>");
    }

    /// The web UI composes frames in TypeScript; this writes what it must
    /// agree with, and the cases a vitest checks it against.
    #[test]
    fn frame_ts() {
        let quoted = |text: &str| serde_json::to_string(text).expect("a string serializes");
        let list =
            |items: &[&str]| format!("[{}] as const", items.iter().map(|item| quoted(item)).collect::<Vec<_>>().join(", "));
        let case = |html, style, theme, probe, vendor, parent, policy_meta| {
            (html, FrameParts { style, theme, probe, vendor, parent, policy_meta })
        };
        let cases = [
            case("<!DOCTYPE html><html><body><p>hi</p></body></html>", None, None, true, None, None, true),
            case(
                "  <p>no doctype</p>",
                Some(":root{--bg:#fff}</style>"),
                Some(Theme::Dark),
                true,
                None,
                Some("http://127.0.0.1:7777"),
                true,
            ),
            case(
                "<!doctype html><script src=\"https://cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js\"></script>",
                Some("@import url(//fonts.googleapis.com/css2?family=Inter);"),
                Some(Theme::Light),
                true,
                Some("http://127.0.0.1:7777/vendor/"),
                Some("https://silt.example"),
                true,
            ),
            case("<p>offline</p>", Some("a{}"), None, false, None, None, true),
            case("<p>served with a header</p>", None, Some(Theme::Light), true, None, Some("https://a.example/'+x+'"), false),
            case("<!DOCTYPE html  >\n<p>odd doctype</p>", None, None, true, None, None, true),
        ];
        let fixtures = cases
            .iter()
            .map(|(html, parts)| {
                let out = compose(html, parts);
                let option = |value: Option<&str>| value.map_or_else(|| "null".to_owned(), quoted);
                format!(
                    "  {{ html: {}, style: {}, theme: {}, probe: {}, vendor: {}, parent: {}, policyMeta: {}, out: {} }},",
                    quoted(html),
                    option(parts.style),
                    option(parts.theme.map(Theme::as_str)),
                    parts.probe,
                    option(parts.vendor),
                    option(parts.parent),
                    parts.policy_meta,
                    quoted(&out)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let generated = format!(
            "// What an artifact's frame gets, written by `UPDATE_FRAME=1 cargo test -p silt-design frame_ts`\n// from crates/silt-design/src/frame.rs. Don't edit by hand.\n\n\
export const SCRIPT_SOURCES = {}\nexport const STYLE_SOURCES = {}\nexport const FONT_SOURCES = {}\nexport const VENDOR_HOSTS = {}\n\n\
export const FIRST_SCRIPT_START = {}\nexport const FIRST_SCRIPT_END = {}\n\n\
/** Frames composed by `silt_design::frame::compose`, which the web UI's must match. */\n\
export const FRAME_FIXTURES: {{ html: string; style: string | null; theme: 'light' | 'dark' | null; probe: boolean; vendor: string | null; parent: string | null; policyMeta: boolean; out: string }}[] = [\n{fixtures}\n]\n",
            list(&SCRIPT_SOURCES),
            list(&STYLE_SOURCES),
            list(&FONT_SOURCES),
            list(&VENDOR_HOSTS),
            quoted(FIRST_SCRIPT_START),
            quoted(FIRST_SCRIPT_END),
        );
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/src/frame.generated.ts");
        if std::env::var_os("UPDATE_FRAME").is_some() {
            std::fs::write(&path, &generated).expect("writes the frame's contract");
        }
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            written == generated,
            "web/src/frame.generated.ts is out of date: run `UPDATE_FRAME=1 cargo test -p silt-design frame_ts`"
        );
    }

    #[test]
    fn cdn_addresses_go_through_the_vendor_route() {
        let vendor = "http://127.0.0.1:7777/vendor/";
        let html = r#"<script src="https://cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js"></script><link href="//fonts.googleapis.com/css2?family=Inter">"#;
        let out = through_vendor(html, vendor);
        assert!(out.contains(r#"src="http://127.0.0.1:7777/vendor/cdnjs.cloudflare.com/ajax/libs/d3/7.9.0/d3.min.js""#), "{out}");
        assert!(out.contains(r#"href="http://127.0.0.1:7777/vendor/fonts.googleapis.com/css2?family=Inter""#), "{out}");
        assert_eq!(
            fonts_css_through_vendor("src: url(https://fonts.gstatic.com/s/inter/v13/a.woff2)"),
            "src: url(../fonts.gstatic.com/s/inter/v13/a.woff2)"
        );
    }
}
