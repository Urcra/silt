//! The git mirror (ADR 0009): one bare repository per family, served over
//! git's smart HTTP protocol by running `git http-backend` as a CGI
//! program. Devices push commits that origin lacks to
//! `refs/silt/threads/<thread>` and fetch them on other machines.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use silt_types::{FamilyId, ThreadId};
use sqlx::Row;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::{ApiError, Caller, FamilyAccess, Server, bad_request, claim_family, family_access, not_found};

pub(crate) async fn serve(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath((repo, rest)): UrlPath<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let family = repo
        .strip_suffix(".git")
        .and_then(|id| FamilyId::from_str(id).ok())
        .ok_or_else(|| bad_request("expected /git/<family>.git/…"))?;
    let pushing = rest == "git-receive-pack" || uri.query() == Some("service=git-receive-pack");
    match family_access(&server, family, caller.user).await? {
        FamilyAccess::Visible => {}
        FamilyAccess::Unknown if pushing => claim_family(&server, family, caller).await?,
        FamilyAccess::Unknown | FamilyAccess::Hidden => return Err(not_found()),
    }
    let root = server.git_root();
    let dir = root.join(&repo);
    if !dir.exists() {
        init_bare(&dir).await.map_err(|error| internal(&error))?;
    }
    let hooks = install_hooks(&root).await.map_err(|error| internal(&error))?;
    let rights = thread_rights(&server, family, caller.user).await?;
    let mut access = access_config(&rights, &hooks);
    // `http-backend` only accepts pushes from an authenticated user.
    access.push(("REMOTE_USER".to_owned(), caller.user.to_string()));
    run_backend(&root, &format!("/{repo}/{rest}"), &method, &uri, &headers, body, &access)
        .await
        .map_err(|error| internal(&error))
}

/// What a caller may do with one family's thread refs. The mirror is one
/// repository per family, while access is per thread (ADR 0010, 0014).
struct ThreadRights {
    visible: Vec<ThreadId>,
    writable: Vec<ThreadId>,
    known: Vec<ThreadId>,
}

async fn thread_rights(server: &Server, family: FamilyId, user: i64) -> Result<ThreadRights, ApiError> {
    let viewer = server.viewer(user).await?;
    let rows = sqlx::query(
        "SELECT t.id, t.owner_id, EXISTS (SELECT 1 FROM thread_collaborators c WHERE c.thread_id = t.id AND c.user_id = ?) AS shared
         FROM threads t WHERE t.family_id = ?",
    )
    .bind(user)
    .bind(crate::id_bytes(family.0))
    .fetch_all(&server.shared.db)
    .await?;
    let mut rights = ThreadRights {
        visible: Vec::new(),
        writable: Vec::new(),
        known: Vec::new(),
    };
    for row in rows {
        let thread = ThreadId(crate::id_from(row.get("id")));
        rights.known.push(thread);
        if viewer.sees_thread(thread) {
            rights.visible.push(thread);
        }
        if row.get::<i64, _>("owner_id") == user || row.get::<i64, _>("shared") != 0 {
            rights.writable.push(thread);
        }
    }
    Ok(rights)
}

fn thread_ref(thread: ThreadId) -> String {
    format!("refs/silt/threads/{thread}")
}

/// Per-request git configuration: fetches see only the caller's visible
/// threads' refs, and pushes go through the hook that checks writes.
fn access_config(rights: &ThreadRights, hooks: &Path) -> Vec<(String, String)> {
    let mut config = vec![
        ("uploadpack.hideRefs".to_owned(), "refs/".to_owned()),
        ("receive.hideRefs".to_owned(), "refs/".to_owned()),
        ("core.hooksPath".to_owned(), hooks.to_string_lossy().into_owned()),
        ("receive.denyDeletes".to_owned(), "true".to_owned()),
        ("receive.maxInputSize".to_owned(), MAX_PACK.to_string()),
    ];
    for thread in &rights.visible {
        config.push(("uploadpack.hideRefs".to_owned(), format!("!{}", thread_ref(*thread))));
    }
    for thread in &rights.writable {
        config.push(("receive.hideRefs".to_owned(), format!("!{}", thread_ref(*thread))));
    }
    let refs = |threads: &[ThreadId]| threads.iter().map(|thread| thread_ref(*thread)).collect::<Vec<_>>().join(" ");
    let mut environment: Vec<(String, String)> = vec![("GIT_CONFIG_COUNT".to_owned(), config.len().to_string())];
    for (index, (key, value)) in config.into_iter().enumerate() {
        environment.push((format!("GIT_CONFIG_KEY_{index}"), key));
        environment.push((format!("GIT_CONFIG_VALUE_{index}"), value));
    }
    environment.push(("SILT_WRITABLE_REFS".to_owned(), refs(&rights.writable)));
    environment.push(("SILT_KNOWN_REFS".to_owned(), refs(&rights.known)));
    environment
}

/// Largest pack one push may send.
const MAX_PACK: u64 = 512 * 1024 * 1024;

/// Lets a push update only `refs/silt/threads/<thread>` for threads its user
/// may write, or that the server doesn't know yet (a device pushes a new
/// thread's commits before registering it), and never delete one.
const PRE_RECEIVE: &str = r#"#!/bin/sh
# Written by the Silt server.
while read old new ref; do
    case "$new" in
        *[!0]*) ;;
        *) echo "silt: refs can't be deleted" >&2; exit 1 ;;
    esac
    case " $SILT_WRITABLE_REFS " in *" $ref "*) continue ;; esac
    case " $SILT_KNOWN_REFS " in *" $ref "*) echo "silt: $ref belongs to a thread you can't write" >&2; exit 1 ;; esac
    case "$ref" in
        refs/silt/threads/*) ;;
        *) echo "silt: only thread refs can be pushed" >&2; exit 1 ;;
    esac
done
"#;

async fn install_hooks(root: &Path) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    let hooks = root.with_file_name("git-hooks");
    let hook = hooks.join("pre-receive");
    if tokio::fs::read_to_string(&hook).await.ok().as_deref() != Some(PRE_RECEIVE) {
        tokio::fs::create_dir_all(&hooks).await?;
        let staged = hooks.join(".pre-receive.new");
        tokio::fs::write(&staged, PRE_RECEIVE).await?;
        tokio::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).await?;
        tokio::fs::rename(&staged, &hook).await?;
    }
    Ok(hooks)
}

fn internal(error: &std::io::Error) -> ApiError {
    tracing::error!(%error, "the git mirror failed");
    crate::ServerError::Io(std::io::Error::new(error.kind(), error.to_string())).into()
}

async fn init_bare(dir: &Path) -> std::io::Result<()> {
    let status = Command::new("git").args(["init", "-q", "--bare"]).arg(dir).status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("git init --bare failed"))
    }
}

/// Runs `git http-backend` for one request and turns its CGI output into
/// a response.
async fn run_backend(
    root: &Path,
    path_info: &str,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
    access: &[(String, String)],
) -> std::io::Result<Response> {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let mut child = Command::new("git")
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", path_info)
        .env("REQUEST_METHOD", method.as_str())
        .env("QUERY_STRING", uri.query().unwrap_or_default())
        .env("CONTENT_TYPE", header("content-type"))
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("HTTP_CONTENT_ENCODING", header("content-encoding"))
        .env("GIT_PROTOCOL", header("git-protocol"))
        .env("REMOTE_ADDR", "127.0.0.1")
        .envs(access.iter().map(|(key, value)| (key, value)))
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let writer = tokio::spawn(async move {
        let written = stdin.write_all(&body).await;
        drop(stdin);
        written
    });
    let output = child.wait_with_output().await?;
    writer.await.map_err(std::io::Error::other)??;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(std::io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()));
    }
    Ok(cgi_response(&output.stdout))
}

/// Splits CGI output into status, headers and body.
fn cgi_response(output: &[u8]) -> Response {
    let (head, body) = match output.windows(4).position(|window| window == b"\r\n\r\n") {
        Some(end) => (&output[..end], &output[end + 4..]),
        None => match output.windows(2).position(|window| window == b"\n\n") {
            Some(end) => (&output[..end], &output[end + 2..]),
            None => (&output[..0], output),
        },
    };
    let mut status = StatusCode::OK;
    let mut response_headers = HeaderMap::new();
    for line in String::from_utf8_lossy(head).lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("status") {
            status = value
                .split_whitespace()
                .next()
                .and_then(|code| code.parse().ok())
                .and_then(|code| StatusCode::from_u16(code).ok())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        } else if let (Ok(name), Ok(value)) = (HeaderName::from_str(name), HeaderValue::from_str(value)) {
            response_headers.append(name, value);
        }
    }
    (status, response_headers, Body::from(body.to_vec())).into_response()
}

impl Server {
    pub(crate) fn git_root(&self) -> PathBuf {
        self.shared.data_dir.join("git")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgi_output_becomes_a_response() {
        let response = cgi_response(b"Status: 404 Not Found\r\nContent-Type: text/plain\r\n\r\nnope");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()["content-type"], "text/plain");
        assert_eq!(cgi_response(b"Content-Type: x\n\nbody").status(), StatusCode::OK);
    }
}
