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

/// The script a checked page runs first: the theme on its root, and what
/// goes wrong told to the page holding its frame, as the web UI's frame
/// script does for a viewer.
const CHECK_SCRIPT: &str = "(function(){var root=document.documentElement;var seen={},problems=[];\
function report(kind,text){text=String(text).slice(0,300);if(seen[kind+text]||problems.length>=20)return;seen[kind+text]=1;\
problems.push({kind:kind,text:text});parent.postMessage({silt:'problems',problems:problems.slice()},'*')}\
document.addEventListener('securitypolicyviolation',function(e){report('blocked',(e.blockedURI||'inline')+' ('+e.effectiveDirective+')')});\
addEventListener('error',function(e){if(e.message)report('error',e.message+(e.lineno?' (line '+e.lineno+')':''))});\
addEventListener('unhandledrejection',function(e){var r=e.reason;report('error','Unhandled rejection: '+(r&&r.message||r))});\
addEventListener('load',function(){setTimeout(function(){var w=root.scrollWidth,v=root.clientWidth;\
if(w>v+1)report('overflow','The page is '+w+'px wide in a '+v+'px frame');parent.postMessage({silt:'checked'},'*')},300)});})();";

/// An HTML version as a frame loads it to be checked: the policy, the
/// theme, the check's script and the design system's CSS, then the page.
pub fn checked_document(html: &str, style: Option<&str>, theme: &str) -> String {
    let trimmed = html.trim_start();
    let doctype_end = trimmed
        .get(..9)
        .filter(|start| start.eq_ignore_ascii_case("<!doctype"))
        .and_then(|_| trimmed.find('>'))
        .map(|end| end + 1);
    let (doctype, rest) = match doctype_end {
        Some(end) => trimmed.split_at(end),
        None => ("<!doctype html>", trimmed),
    };
    let theme = if theme == "dark" { "dark" } else { "light" };
    let style =
        style.map(|css| format!("<style data-silt=\"design-system\">{}</style>", css.replace("</", "<\\/"))).unwrap_or_default();
    format!(
        "{doctype}<meta http-equiv=\"Content-Security-Policy\" content=\"{}\"><script>document.documentElement.setAttribute('data-theme','{theme}');{CHECK_SCRIPT}</script>{style}{rest}",
        policy(None)
    )
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
        let page = checked_document("<!DOCTYPE html><p>hi</p>", Some(":root{--bg:#fff}</style>"), "dark");
        assert!(page.starts_with("<!DOCTYPE html><meta http-equiv=\"Content-Security-Policy\""), "{page}");
        assert!(page.contains("setAttribute('data-theme','dark')") && page.contains("'securitypolicyviolation'"));
        assert!(page.ends_with(":root{--bg:#fff}<\\/style></style><p>hi</p>"), "{page}");
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
