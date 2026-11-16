//! Thumbnails for the Artifacts page (ADR 0075): an HTML version rendered
//! by a headless browser, where Node and Playwright are installed, the first
//! time the page asks for it, and kept in the thread's page data so other
//! machines and the hosted app show it too.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use silt_log::{Author, ConvOp, PageOp};
use silt_proto::page::Level;
use silt_types::ArtifactId;

use crate::core::{Core, CoreError};

/// The page document naming a thumbnail: its blob and the version it shows.
pub(crate) const THUMBNAIL: &str = "_meta/thumbnail";
/// The longest a render may take.
const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

/// A page at a desktop's size, light, shot at half scale as a JPEG.
const SCRIPT: &str = r#"const fs = require('fs');
let playwright;
try { playwright = require('playwright'); } catch (e) { console.log(JSON.stringify({ missing: true })); process.exit(0); }
const job = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
(async () => {
  const launch = process.env.SILT_CHROMIUM ? { executablePath: process.env.SILT_CHROMIUM } : {};
  const browser = await playwright.chromium.launch(launch);
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 0.5, colorScheme: 'light' });
  await page.setContent('<!doctype html><style>html,body{margin:0;height:100%}iframe{border:0;width:100%;height:100%}</style><iframe id="f" sandbox="allow-scripts"></iframe>');
  await page.$eval('#f', (frame, doc) => { frame.srcdoc = doc; }, job.page);
  await page.waitForTimeout(1500);
  await page.screenshot({ path: job.out, type: 'jpeg', quality: 70 });
  await browser.close();
  console.log(JSON.stringify({ done: true }));
})().catch((error) => { console.log(JSON.stringify({ failed: String(error && error.message || error).slice(0, 500) })); process.exit(2); });
"#;

/// A document printed with its scripts off: to A4 for a doc (ADR 0077), or
/// at the size its `@page` rule gives, as a deck's slides do (ADR 0076).
const PDF_SCRIPT: &str = r#"const fs = require('fs');
let playwright;
try { playwright = require('playwright'); } catch (e) { console.log(JSON.stringify({ missing: true })); process.exit(0); }
const job = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
(async () => {
  const launch = process.env.SILT_CHROMIUM ? { executablePath: process.env.SILT_CHROMIUM } : {};
  const browser = await playwright.chromium.launch(launch);
  const context = await browser.newContext({ javaScriptEnabled: false, offline: true });
  const page = await context.newPage();
  await page.setContent(job.page, { waitUntil: 'load' });
  await page.pdf({ path: job.out, format: 'A4', preferCSSPageSize: true, printBackground: true });
  await browser.close();
  console.log(JSON.stringify({ done: true }));
})().catch((error) => { console.log(JSON.stringify({ failed: String(error && error.message || error).slice(0, 500) })); process.exit(2); });
"#;

/// Renders `html`, styled with `style`, to a JPEG in `dir`; none where
/// there's no headless browser.
pub(crate) fn render(html: &str, style: Option<&str>, dir: &Path) -> Result<Option<Vec<u8>>, String> {
    let page = silt_design::frame::checked_document(html, style, silt_design::frame::Theme::Light);
    run(SCRIPT, &page, dir, "thumbnail.jpg")
}

/// Prints the document `page` to a PDF in `dir`; none where there's no
/// headless browser.
pub(crate) fn pdf(page: &str, dir: &Path) -> Result<Option<Vec<u8>>, String> {
    run(PDF_SCRIPT, page, dir, "printed.pdf")
}

/// Runs `script` on `page` in Node with Playwright, which writes `name` in
/// `dir`; none where either is missing.
fn run(script_text: &str, page: &str, dir: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let (job, out, script) = (dir.join("job.json"), dir.join(name), dir.join(format!("{name}.cjs")));
    let _ = std::fs::remove_file(&out);
    std::fs::write(&job, serde_json::json!({"page": page, "out": out}).to_string()).map_err(|error| error.to_string())?;
    std::fs::write(&script, script_text).map_err(|error| error.to_string())?;
    let mut command = Command::new(program("node"));
    command.arg(&script).arg(&job).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    if let Some(global) = npm_root() {
        command.env("NODE_PATH", global);
    }
    let Ok(mut child) = command.spawn() else { return Ok(None) };
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > RENDER_TIMEOUT => {
                let _ = child.kill();
                return Err(format!("rendering {name} took too long"));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(std::fs::read(&out).ok())
}

/// `name` found on `PATH`, as Windows names an npm install `npm.cmd`.
fn program(name: &str) -> std::path::PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    silt_os::find_program(name, &path).unwrap_or_else(|| name.into())
}

fn npm_root() -> Option<std::path::PathBuf> {
    let output = Command::new(program("npm")).args(["root", "-g"]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    (output.status.success() && !root.is_empty()).then(|| root.into())
}

impl Core {
    /// The thumbnail kept for `artifact`'s latest version, if there is one.
    pub(crate) async fn kept_thumbnail(&self, thread: &str, artifact: &str) -> Result<Option<String>, CoreError> {
        let thread = self.find_thread(thread)?;
        let id: ArtifactId = artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        let latest = self.latest_view(thread, artifact)?;
        let docs = self.page_docs(self.threads[&thread].family, thread, id);
        let Some(doc) = docs.get(THUMBNAIL).filter(|doc| doc["version"].as_u64() == Some(u64::from(latest.version))) else {
            return Ok(None);
        };
        let Some(blob) = doc["blob"].as_str().and_then(|blob| blob.parse().ok()) else { return Ok(None) };
        Ok(Some(BASE64.encode(self.store.blobs().get(blob).await?)))
    }

    /// What to render a thumbnail from: the latest version's page and style,
    /// for an HTML artifact.
    pub(crate) async fn thumbnail_source(
        &self,
        thread: &str,
        artifact: &str,
    ) -> Result<Option<(String, Option<String>, u32)>, CoreError> {
        let thread = self.find_thread(thread)?;
        let latest = self.latest_view(thread, artifact)?;
        if latest.kind != "html" {
            return Ok(None);
        }
        let read = |blob: &str| blob.parse().map_err(|_| CoreError::Invalid("not kept here".into()));
        let page = String::from_utf8_lossy(&self.store.blobs().get(read(&latest.blob)?).await?).into_owned();
        let style = match latest.style.as_deref() {
            Some(style) => Some(String::from_utf8_lossy(&self.store.blobs().get(read(style)?).await?).into_owned()),
            None => None,
        };
        Ok(Some((page, style, latest.version)))
    }

    /// Keeps a rendered thumbnail beside the session and names it in the
    /// page data, where this machine's person may write there.
    pub(crate) async fn keep_thumbnail(
        &mut self,
        thread: &str,
        artifact: &str,
        version: u32,
        jpeg: Vec<u8>,
    ) -> Result<(), CoreError> {
        let thread = self.find_thread(thread)?;
        let id: ArtifactId = artifact.parse().map_err(|_| CoreError::Invalid(format!("{artifact} isn't an artifact id")))?;
        if self.page_level(thread) < Level::Collaborator {
            return Ok(());
        }
        let Some(session) = self.artifact_session(thread, artifact) else { return Ok(()) };
        let blob = self.store.blobs().put(&jpeg).await?;
        let family = self.threads[&thread].family;
        let name = format!("artifact/{artifact}/v{version}/thumbnail.jpg");
        self.append_conv(family, thread, Author::User, ConvOp::Attachment { session, name, blob }).await?;
        let value = serde_json::json!({"blob": blob.to_string(), "version": version}).to_string();
        self.append_page(family, thread, PageOp::Set { artifact: id, path: THUMBNAIL.into(), value }).await
    }

    fn latest_view(&self, thread: silt_types::ThreadId, artifact: &str) -> Result<silt_proto::view::ArtifactView, CoreError> {
        self.history_entries(thread)
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .filter(|view| view.id == artifact)
            .max_by_key(|view| view.version)
            .cloned()
            .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {artifact}")))
    }
}
