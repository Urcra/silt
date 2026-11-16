//! `silt_artifact_check`: renders a version as a viewer's frame shows it,
//! at a phone's width and a desktop's, in light and dark, with a headless
//! browser where Node and Playwright are installed (ADR 0071).

use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;
use silt_daemon::protocol::ArtifactRead;

/// The longest a check may take, all four renders together.
const CHECK_TIMEOUT: Duration = Duration::from_secs(120);

/// Opens each page in a sandboxed frame inside a plain host page, as Silt's
/// panel does, and reports what the frame's script heard go wrong, the
/// console's errors, and a screenshot.
const SCRIPT: &str = r#"const fs = require('fs'), path = require('path');
let playwright;
try { playwright = require('playwright'); } catch (e) {
  try { playwright = require('playwright-core'); } catch (e2) { console.log(JSON.stringify({ missing: true })); process.exit(3); }
}
const [jobFile] = process.argv.slice(2);
const job = JSON.parse(fs.readFileSync(jobFile, 'utf8'));
const host = '<!doctype html><html><body style="margin:0"><iframe id="f" sandbox="allow-scripts" style="border:0;display:block;width:100vw;height:100vh"></iframe>' +
  '<script>window.__problems=[];window.__checked=false;addEventListener("message",function(e){var d=e.data;if(!d)return;' +
  'if(d.silt==="problems")window.__problems=d.problems;if(d.silt==="checked")window.__checked=true})</script></body></html>';
// What each visible element with text of its own shows: its colour and the
// ground behind it, which the check compares across themes.
const SAMPLE = () => {
  const clear = (c) => !c || c === 'transparent' || /^rgba\([^)]*,\s*0\)$/.test(c);
  const ground = (el) => { for (let e = el; e; e = e.parentElement) { const c = getComputedStyle(e).backgroundColor; if (!clear(c)) return c; } return 'rgb(255, 255, 255)'; };
  const name = (el) => {
    const parts = [];
    for (let e = el; e && parts.length < 3; e = e.parentElement) {
      let part = e.tagName.toLowerCase();
      if (e.id) { parts.unshift(part + '#' + e.id); break; }
      if (e.classList.length) part += '.' + [...e.classList].slice(0, 2).join('.');
      parts.unshift(part);
      if (e === document.body) break;
    }
    return parts.join(' > ');
  };
  const samples = [];
  for (const el of document.querySelectorAll('body, body *')) {
    if (samples.length >= 400) break;
    if (![...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())) continue;
    const s = getComputedStyle(el);
    if (s.visibility === 'hidden' || s.display === 'none' || !el.getClientRects().length) continue;
    samples.push({ selector: name(el), color: s.color, background: ground(el), size: parseFloat(s.fontSize) || 16, weight: parseInt(s.fontWeight, 10) || 400 });
  }
  return { body: getComputedStyle(document.body).backgroundColor, samples };
};
(async () => {
  const launch = process.env.SILT_CHROMIUM ? { executablePath: process.env.SILT_CHROMIUM } : {};
  const browser = await playwright.chromium.launch(launch);
  const results = [];
  for (const [label, width, height] of [['phone', 390, 844], ['desktop', 1280, 800]]) {
    for (const theme of ['light', 'dark']) {
      const page = await browser.newPage({ viewport: { width, height }, colorScheme: theme });
      const errors = [];
      page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text().slice(0, 300)); });
      page.on('pageerror', (error) => errors.push(String(error.message || error).slice(0, 300)));
      await page.setContent(host);
      await page.$eval('#f', (frame, doc) => { frame.srcdoc = doc; }, job.pages[theme]);
      await page.waitForFunction(() => window.__checked, null, { timeout: 15000 }).catch(() => errors.push('The page did not finish loading in 15 s'));
      await page.waitForTimeout(400);
      const problems = await page.evaluate(() => window.__problems);
      const inner = page.frames().find((frame) => frame !== page.mainFrame());
      const looked = inner ? await inner.evaluate(SAMPLE).catch(() => null) : null;
      const shot = path.join(job.out, `${label}-${theme}.png`);
      await page.screenshot({ path: shot });
      results.push({ label, theme, width, problems, errors: [...new Set(errors)].slice(0, 20), shot, body: looked && looked.body, samples: looked ? looked.samples : [] });
      await page.close();
    }
  }
  await browser.close();
  console.log(JSON.stringify({ results }));
})().catch((error) => { console.log(JSON.stringify({ failed: String(error && error.message || error).slice(0, 2000) })); process.exit(2); });
"#;

#[derive(Debug, Deserialize)]
struct Answer {
    #[serde(default)]
    missing: bool,
    #[serde(default)]
    failed: Option<String>,
    #[serde(default)]
    results: Vec<Render>,
}

#[derive(Debug, Deserialize)]
struct Render {
    label: String,
    theme: String,
    width: u32,
    #[serde(default)]
    problems: Vec<Problem>,
    #[serde(default)]
    errors: Vec<String>,
    shot: String,
    /// The body's computed background.
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    samples: Vec<Sample>,
}

/// An element with text of its own, as it rendered.
#[derive(Debug, Clone, Deserialize)]
struct Sample {
    selector: String,
    color: String,
    background: String,
    size: f64,
    weight: u32,
}

#[derive(Debug, Deserialize)]
struct Problem {
    kind: String,
    text: String,
}

/// Checks `read`, run from `checkout` so the project's own Playwright is
/// found first, writing its screenshots under `dir`.
pub(crate) fn check(read: &ArtifactRead, checkout: &Path, dir: &Path) -> Result<String, String> {
    use base64::Engine as _;
    if read.kind != "html" {
        return Ok(format!(
            "Only an HTML artifact is checked: a {} one shows as Silt renders it, with nothing of its own running.",
            read.kind
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(&read.data).map_err(|error| error.to_string())?;
    let html = String::from_utf8_lossy(&bytes);
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let pages = serde_json::json!({
        "light": silt_design::frame::checked_document(&html, read.style.as_deref(), silt_design::frame::Theme::Light),
        "dark": silt_design::frame::checked_document(&html, read.style.as_deref(), silt_design::frame::Theme::Dark),
    });
    let job = dir.join("job.json");
    std::fs::write(&job, serde_json::json!({"pages": pages, "out": dir}).to_string()).map_err(|error| error.to_string())?;
    let script = dir.join("check.cjs");
    std::fs::write(&script, SCRIPT).map_err(|error| error.to_string())?;

    let mut command = Command::new("node");
    command.arg(&script).arg(&job).current_dir(checkout).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(global) = npm_root() {
        let joined = match std::env::var_os("NODE_PATH") {
            Some(existing) => std::env::join_paths(std::env::split_paths(&existing).chain([global])),
            None => std::env::join_paths([global]),
        };
        if let Ok(joined) = joined {
            command.env("NODE_PATH", joined);
        }
    }
    let Ok(child) = command.spawn() else {
        return Ok(unavailable("Node isn't installed here"));
    };
    let output = wait(child)?;
    let answer: Answer =
        serde_json::from_slice(&output).map_err(|_| "the headless check said nothing it could read".to_owned())?;
    if answer.missing {
        return Ok(unavailable("Playwright isn't installed for Node here (`npm install -g playwright`, with a Chromium)"));
    }
    if let Some(failed) = answer.failed {
        return Ok(unavailable(&format!("the headless browser failed: {failed}")));
    }
    Ok(report(read, &answer.results))
}

fn unavailable(why: &str) -> String {
    format!("No headless check here: {why}. Publish as you are; the person's own browser reports what goes wrong in the panel.")
}

fn npm_root() -> Option<std::path::PathBuf> {
    let output = Command::new("npm").args(["root", "-g"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (output.status.success() && !root.is_empty()).then(|| root.into())
}

fn wait(mut child: std::process::Child) -> Result<Vec<u8>, String> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > CHECK_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("the headless check took more than {} s", CHECK_TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(error) => return Err(error.to_string()),
        }
    }
    child.wait_with_output().map(|output| output.stdout).map_err(|error| error.to_string())
}

/// The most theme or contrast findings a report lists of each.
const FINDINGS: usize = 10;

/// What ignores the theme, comparing each width's light and dark renders,
/// and what text lacks contrast in either (ADR 0073).
fn theme_findings(renders: &[Render]) -> (Vec<String>, Vec<String>) {
    use silt_design::palette::{Srgb, contrast};
    let white = Srgb::from_bytes(255, 255, 255);
    let mut theme = Vec::new();
    let mut low = Vec::new();
    for light in renders.iter().filter(|render| render.theme == "light") {
        let Some(dark) = renders.iter().find(|render| render.theme == "dark" && render.label == light.label) else {
            continue;
        };
        if light.body.is_some() && light.body == dark.body {
            theme.push(format!(
                "the page's background is {} in both themes: give body a background from a token both themes set",
                light.body.as_deref().unwrap_or_default()
            ));
        }
        for sample in &light.samples {
            let Some(other) = dark.samples.iter().find(|candidate| candidate.selector == sample.selector) else {
                continue;
            };
            if sample.color == other.color && sample.background != other.background {
                theme.push(format!("{}: its text is {} in both themes while its ground changes", sample.selector, sample.color));
            }
        }
    }
    for render in renders {
        for sample in &render.samples {
            let Some(ground) = Srgb::computed(&sample.background, white) else {
                continue;
            };
            let Some(text) = Srgb::computed(&sample.color, ground) else {
                continue;
            };
            let large = sample.size >= 24.0 || (sample.size >= 18.66 && sample.weight >= 700);
            let needed = if large { 3.0 } else { 4.5 };
            let ratio = contrast(text, ground);
            if ratio < needed {
                low.push(format!(
                    "{} in {}: {ratio:.2}:1 ({} on {}), under the {needed}:1 its text needs",
                    sample.selector, render.theme, sample.color, sample.background
                ));
            }
        }
    }
    for list in [&mut theme, &mut low] {
        let mut seen = std::collections::HashSet::new();
        list.retain(|finding| seen.insert(finding.clone()));
        list.truncate(FINDINGS);
    }
    (theme, low)
}

fn report(read: &ArtifactRead, renders: &[Render]) -> String {
    let mut out =
        format!("Checked artifact {} version {} at a phone's width and a desktop's, in light and dark.\n", read.id, read.version);
    let (theme, low) = theme_findings(renders);
    let mut clean = theme.is_empty() && low.is_empty();
    for finding in &theme {
        let _ = writeln!(out, "- theme: {finding}");
    }
    for finding in &low {
        let _ = writeln!(out, "- contrast: {finding}");
    }
    for render in renders {
        let found: Vec<String> = render
            .problems
            .iter()
            .map(|problem| format!("{}: {}", problem.kind, problem.text))
            .chain(render.errors.iter().map(|error| format!("console: {error}")))
            .collect();
        clean &= found.is_empty();
        let said = if found.is_empty() { "no problems".to_owned() } else { found.join("; ") };
        let _ = writeln!(out, "- {} {} ({}px): {said}. Screenshot: {}", render.label, render.theme, render.width, render.shot);
    }
    out.push_str(if clean {
        "Look at the screenshots once for layout, contrast and anything clipped, fix what you see, then publish."
    } else {
        "Fix these in the next version. Look at the screenshots once for layout and contrast too."
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_each_render_and_what_went_wrong() {
        let read = ArtifactRead {
            id: "0123456789ab".into(),
            version: 2,
            latest: 2,
            title: "t".into(),
            kind: "html".into(),
            media_type: "text/html".into(),
            design_system: None,
            data: String::new(),
            style: None,
        };
        let renders = vec![
            Render {
                label: "phone".into(),
                theme: "dark".into(),
                width: 390,
                problems: vec![Problem { kind: "overflow".into(), text: "The page is 620px wide in a 390px frame".into() }],
                errors: vec!["Refused to load the script 'https://esm.sh/x'".into()],
                shot: "/tmp/phone-dark.png".into(),
                body: None,
                samples: Vec::new(),
            },
            Render {
                label: "desktop".into(),
                theme: "light".into(),
                width: 1280,
                problems: vec![],
                errors: vec![],
                shot: "/tmp/d.png".into(),
                body: None,
                samples: Vec::new(),
            },
        ];
        let text = report(&read, &renders);
        assert!(
            text.contains("- phone dark (390px): overflow: The page is 620px wide in a 390px frame; console: Refused"),
            "{text}"
        );
        assert!(
            text.contains("- desktop light (1280px): no problems. Screenshot: /tmp/d.png") && text.contains("Fix these"),
            "{text}"
        );
        let svg = ArtifactRead { kind: "svg".into(), ..read };
        assert!(check(&svg, Path::new("."), Path::new("/nonexistent")).expect("answer").starts_with("Only an HTML artifact"));
    }

    #[test]
    fn text_that_ignores_the_theme_or_lacks_contrast_is_reported() {
        let sample = |selector: &str, color: &str, background: &str| Sample {
            selector: selector.into(),
            color: color.into(),
            background: background.into(),
            size: 16.0,
            weight: 400,
        };
        let render = |theme: &str, body: &str, samples: Vec<Sample>| Render {
            label: "desktop".into(),
            theme: theme.into(),
            width: 1280,
            problems: Vec::new(),
            errors: Vec::new(),
            shot: format!("/tmp/{theme}.png"),
            body: Some(body.into()),
            samples,
        };
        let renders = vec![
            render(
                "light",
                "rgb(255, 255, 255)",
                vec![
                    sample("h1", "rgb(20, 20, 20)", "rgb(255, 255, 255)"),
                    sample("p.note", "rgb(180, 180, 180)", "rgb(255, 255, 255)"),
                ],
            ),
            render(
                "dark",
                "rgb(20, 20, 20)",
                vec![
                    sample("h1", "rgb(20, 20, 20)", "rgb(20, 20, 20)"),
                    sample("p.note", "rgb(200, 200, 200)", "rgb(20, 20, 20)"),
                ],
            ),
        ];
        let (theme, low) = theme_findings(&renders);
        assert_eq!(theme, vec!["h1: its text is rgb(20, 20, 20) in both themes while its ground changes".to_owned()]);
        assert_eq!(low.len(), 2, "{low:?}");
        assert!(low[0].starts_with("p.note in light: 2.") && low[1].starts_with("h1 in dark: 1.00:1"), "{low:?}");

        let same = vec![render("light", "rgb(250, 250, 250)", Vec::new()), render("dark", "rgb(250, 250, 250)", Vec::new())];
        let (theme, _) = theme_findings(&same);
        assert!(theme[0].starts_with("the page's background is rgb(250, 250, 250) in both themes"), "{theme:?}");
    }
}
