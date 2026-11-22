//! The few plain pages a phone sees before it's paired (ADR 0112): getting
//! the certificate, and pairing. Drawn as the server's sign-in pages are,
//! on Nous's tokens.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Nous's tokens, as the web app styles itself with (ADR 0039).
const NOUS: &str = include_str!("../../../../web/src/nous.css");

const STYLE: &str = r#"
@font-face { font-family: 'IBM Plex Sans'; font-weight: 400; font-display: swap; src: url(/fonts/ibm-plex-sans-latin-400-normal.woff2) format('woff2'); }
@font-face { font-family: 'IBM Plex Sans'; font-weight: 500; font-display: swap; src: url(/fonts/ibm-plex-sans-latin-500-normal.woff2) format('woff2'); }
@font-face { font-family: 'IBM Plex Mono'; font-weight: 400; font-display: swap; src: url(/fonts/ibm-plex-mono-latin-400-normal.woff2) format('woff2'); }
body { margin: 0; min-height: 100vh; display: flex; justify-content: center; align-items: flex-start; padding: 12vh 16px 48px; box-sizing: border-box; }
main { width: 100%; max-width: 400px; display: flex; flex-direction: column; gap: 20px; }
.mark { font-weight: 500; font-size: 28px; letter-spacing: -0.04em; }
h1 { font-size: 18px; font-weight: 500; margin: 0; }
p { margin: 0; line-height: 1.5; }
.lead { color: var(--fg-2); margin-top: -12px; }
form { display: flex; flex-direction: column; gap: 12px; margin: 0; }
label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; color: var(--fg-2); }
input { min-height: 44px; box-sizing: border-box; padding: 0 10px; font: 16px var(--font-sans); color: var(--fg-1); background: var(--bg-elevated); border: 1px solid var(--border); border-radius: 2px; }
button, .button { display: inline-flex; align-items: center; justify-content: center; min-height: 44px; box-sizing: border-box; padding: 0 14px; font: 500 13px var(--font-sans); color: var(--fg-on-accent); background: var(--accent); border: 1px solid transparent; border-radius: 2px; text-decoration: none; cursor: pointer; }
button:hover, .button:hover { background: var(--accent-hover); }
.secondary { color: var(--fg-1); background: var(--surface-1); border-color: var(--border); }
.note { display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; font-size: 12px; color: var(--fg-2); background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px; }
.note b { color: var(--fg-1); font-weight: 500; }
.error { padding: 10px 12px; color: var(--fg-1); background: var(--danger-tint); border-radius: 4px; }
a { color: var(--accent-fg); }
.code { font-family: var(--font-mono); font-size: 22px; letter-spacing: 0.12em; color: var(--fg-1); }
.finger { font-family: var(--font-mono); font-size: 12px; letter-spacing: 0.04em; color: var(--fg-1); }
.small { font-size: 12px; color: var(--fg-2); }
ol { margin: 0; padding-left: 18px; display: flex; flex-direction: column; gap: 6px; line-height: 1.5; font-size: 13px; }
.tabs { display: flex; gap: 6px; }
.tab { font-size: 12px; padding: 2px 8px; border: 1px solid var(--border); border-radius: 2px; color: var(--fg-2); text-decoration: none; }
.tab.on { border-color: var(--accent); background: var(--accent-tint); color: var(--accent-fg); }
.spin { width: 14px; height: 14px; border-radius: 50%; border: 2px solid var(--accent-tint); border-top-color: var(--accent); display: inline-block; vertical-align: -2px; margin-right: 8px; animation: turn 0.9s linear infinite; }
@keyframes turn { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
"#;

pub(crate) fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// A page titled `title`, with `body`, HTML its caller escaped; `refresh`
/// looks again after that many seconds, for a page waiting on the person.
pub(crate) fn page(status: StatusCode, title: &str, body: &str, refresh: Option<u32>) -> Response {
    let refresh = refresh.map(|seconds| format!(r#"<meta http-equiv="refresh" content="{seconds}">"#)).unwrap_or_default();
    let html = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">{refresh}
<meta name="referrer" content="no-referrer">
<title>{title} · Silt</title>
<style>{NOUS}{STYLE}</style></head><body><main><span class="mark" aria-hidden="true">silt</span><h1>{title}</h1>{body}</main></body></html>"#,
        title = escape(title),
    );
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; font-src 'self'; form-action 'self'; frame-ancestors 'none'",
            ),
        ],
        html,
    )
        .into_response()
}

/// IBM Plex, from the UI's build, so the pages draw in Silt's faces rather
/// than whatever sans the phone, or the machine taking screen references,
/// has.
pub(crate) async fn font(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    match silt_web::font(&name) {
        Some(data) => {
            ([(header::CONTENT_TYPE, "font/woff2"), (header::CACHE_CONTROL, "public, max-age=86400")], data).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Which phone the page is for, by its browser: the steps differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Apple,
    Android,
}

impl Platform {
    pub(crate) fn of(user_agent: &str) -> Self {
        if user_agent.contains("Android") { Self::Android } else { Self::Apple }
    }
}

/// `ada's Silt on workstation`, the person in a span the screen references
/// mask, as it differs between machines.
fn whose(machine: &str, person: Option<&str>) -> String {
    match person {
        Some(person) => format!(r#"<span data-volatile>{}</span>'s Silt on {}"#, escape(person), escape(machine)),
        None => format!("Silt on {}", escape(machine)),
    }
}

/// The name the authority installs as, as the steps name it.
fn installed(machine: &str, person: Option<&str>) -> String {
    match person {
        Some(person) => format!(r#"Silt on {} (<span data-volatile>{}</span>)"#, escape(machine), escape(person)),
        None => format!("Silt on {}", escape(machine)),
    }
}

/// Getting this machine's certificate, served over plain HTTP.
pub(crate) fn trust(machine: &str, person: Option<&str>, fingerprint: &str, platform: Platform) -> Response {
    let (whose, installed) = (whose(machine, person), installed(machine, person));
    let machine = escape(machine);
    let fingerprint =
        escape(fingerprint).split(' ').collect::<Vec<_>>().chunks(8).map(|line| line.join(" ")).collect::<Vec<_>>().join("<br>");
    let (get, steps) = match platform {
        Platform::Apple => (
            "silt.mobileconfig",
            format!(
                "<li>Allow the download, then in Settings open <b>Profile Downloaded</b> and install <b>{installed}</b>.</li>\
                 <li>In Settings › General › About › Certificate Trust Settings, turn on <b>{installed}</b>.</li>"
            ),
        ),
        Platform::Android => (
            "silt-ca.crt",
            format!(
                "<li>Open the downloaded file. If Android only offers to save it, go to Settings › Security › More security settings › \
                 Encryption &amp; credentials › Install a certificate › CA certificate, and pick it.</li>\
                 <li>Name it <b>{installed}</b>.</li>"
            ),
        ),
    };
    let (apple, android) = match platform {
        Platform::Apple => (" on", ""),
        Platform::Android => ("", " on"),
    };
    page(
        StatusCode::OK,
        &format!("Trust {machine}"),
        &format!(
            r#"<p class="lead">So this phone can open {whose}, safely, over this network.</p>
<p>Check that {machine}'s Devices page shows the same fingerprint:</p>
<p class="finger" data-testid="fingerprint" data-volatile>{fingerprint}</p>
<a class="button" href="/{get}">Get the certificate</a>
<div class="tabs"><a class="tab{apple}" href="/trust?for=apple">iPhone</a><a class="tab{android}" href="/trust?for=android">Android</a></div>
<ol>{steps}<li>Back on {machine}, choose <b>Next: pair the phone</b> and scan the new code.</li></ol>
<div class="note"><b>Only for {machine}</b>This certificate can vouch for {machine}'s own names on this network. It can't stand in for any website.</div>"#
        ),
        None,
    )
}

/// The authority as an iOS profile, which installs it.
pub(crate) fn mobileconfig(machine: &str, label: &str, der: &[u8]) -> String {
    use base64::Engine as _;
    let certificate = base64::engine::general_purpose::STANDARD.encode(der);
    let (machine, label) = (escape(machine), escape(label));
    let id = |part: &str| format!("dev.silt.lan.{}", blake3::hash(format!("{part}{certificate}").as_bytes()).to_hex());
    let uuid = |part: &str| {
        let hash = blake3::hash(format!("{part}{certificate}").as_bytes());
        uuid::Builder::from_random_bytes(hash.as_bytes()[..16].try_into().expect("16 bytes"))
            .into_uuid()
            .hyphenated()
            .to_string()
            .to_uppercase()
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>PayloadContent</key><array><dict>
<key>PayloadCertificateFileName</key><string>silt-ca.crt</string>
<key>PayloadContent</key><data>{certificate}</data>
<key>PayloadDescription</key><string>Lets this phone open Silt on {machine} over your network.</string>
<key>PayloadDisplayName</key><string>{label}</string>
<key>PayloadIdentifier</key><string>{cert_id}</string>
<key>PayloadType</key><string>com.apple.security.root</string>
<key>PayloadUUID</key><string>{cert_uuid}</string>
<key>PayloadVersion</key><integer>1</integer>
</dict></array>
<key>PayloadDisplayName</key><string>{label}</string>
<key>PayloadIdentifier</key><string>{profile_id}</string>
<key>PayloadType</key><string>Configuration</string>
<key>PayloadUUID</key><string>{profile_uuid}</string>
<key>PayloadVersion</key><integer>1</integer>
</dict></plist>
"#,
        cert_id = id("certificate"),
        cert_uuid = uuid("certificate"),
        profile_id = id("profile"),
        profile_uuid = uuid("profile"),
    )
}

pub(crate) fn enter(error: Option<&str>) -> Response {
    let error = error.map(|error| format!(r#"<div class="error">{}</div>"#, escape(error))).unwrap_or_default();
    page(
        StatusCode::OK,
        "Pair this phone",
        &format!(
            r#"{error}<p class="small">Type the code on the machine's Devices page, under Phones on this network.</p>
<form method="get" action="/pair"><label>Code <input name="code" autocomplete="off" autocapitalize="characters" required></label><button type="submit">Continue</button></form>"#
        ),
        None,
    )
}

pub(crate) fn used_up(machine: &str) -> Response {
    let machine = escape(machine);
    page(
        StatusCode::NOT_FOUND,
        "Pair this phone",
        &format!(
            r#"<div class="error">This code is used up or has expired.</div>
<p class="small">Show a new code on {machine}'s Devices page, under Phones on this network.</p>
<form method="get" action="/pair"><label>Or type the new code <input name="code" autocomplete="off" autocapitalize="characters" required></label><button type="submit">Continue</button></form>"#
        ),
        None,
    )
}

pub(crate) fn waiting(machine: &str, person: Option<&str>, code: &str) -> Response {
    let whose = whose(machine, person);
    let (machine, code) = (escape(machine), escape(code));
    page(
        StatusCode::OK,
        "Pair this phone",
        &format!(
            r#"<p class="lead">With {whose}, over this network.</p>
<p><span class="spin" aria-hidden="true"></span>Waiting for you to allow this phone on {machine}.</p>
<div class="note"><b>Check the code</b>{machine} should show <span class="code" data-testid="pairing-code" data-volatile>{code}</span></div>
<p class="small">Keep this page open. It goes on by itself once you allow it.</p>"#
        ),
        Some(2),
    )
}

pub(crate) fn denied(machine: &str) -> Response {
    let machine = escape(machine);
    page(
        StatusCode::FORBIDDEN,
        "Not paired",
        &format!(
            r#"<div class="error">{machine} didn't allow this phone.</div>
<p class="small">If that was a mistake, show a new code on {machine}'s Devices page and scan it again.</p>"#
        ),
        None,
    )
}

/// Paired: what the phone may do, and how it gets notifications. A phone
/// paired past a certificate warning needs the certificate for those
/// (ADR 0113), and `trust` is where it gets it.
pub(crate) fn paired(machine: &str, platform: Platform, can: &str, trust: &str) -> Response {
    let (machine, can, trust) = (escape(machine), escape(can), escape(trust));
    let install = match platform {
        Platform::Apple => {
            "Add Silt to your Home Screen first: Share › Add to Home Screen. Then open it from there and turn notifications on."
        }
        Platform::Android => {
            "Open Silt and turn notifications on. Add it to your Home Screen from the browser's menu to open it like an app."
        }
    };
    page(
        StatusCode::OK,
        "Paired",
        &format!(
            r#"<p class="lead">This phone can now {can} on {machine}.</p>
<div class="note"><b>Get notified when an agent needs you</b>{install}</div>
<div class="note"><b>Went past a certificate warning?</b>Then notifications and the Home Screen app need {machine}’s certificate first. The phone stays paired.<a href="{trust}">Get the certificate</a></div>
<a class="button" href="/needs-you">Open Silt</a>"#
        ),
        None,
    )
}
