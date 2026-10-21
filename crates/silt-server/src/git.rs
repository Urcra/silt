//! The git mirror (ADR 0009): one bare repository per family, served over
//! git's smart HTTP protocol by running `git http-backend` as a CGI
//! program. Devices push commits that origin lacks to
//! `refs/silt/threads/<thread>` and fetch them on other machines.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;

use axum::body::{Body, Bytes};
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use silt_types::{FamilyId, ThreadId};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio_util::io::ReaderStream;

use crate::{ApiError, Caller, FamilyAccess, Server, UserId, bad_request, claim_family, family_access, not_found};

pub(crate) async fn serve(
    State(server): State<Server>,
    Extension(caller): Extension<Caller>,
    UrlPath((repo, rest)): UrlPath<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
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
    run_backend(&root, &format!("/{repo}/{rest}"), &method, &uri, &headers, body, &access).await.map_err(|error| internal(&error))
}

/// What a caller may do with one family's thread refs. The mirror is one
/// repository per family, while access is per thread (ADR 0010, 0014).
struct ThreadRights {
    visible: Vec<ThreadId>,
    writable: Vec<ThreadId>,
    known: Vec<ThreadId>,
}

async fn thread_rights(server: &Server, family: FamilyId, user: UserId) -> Result<ThreadRights, ApiError> {
    let viewer = server.viewer(user).await?;
    let family = crate::id_bytes(family.0);
    let rows = sqlx::query!(
        r#"SELECT t.id, t.owner_id AS "owner_id: UserId",
                EXISTS (SELECT 1 FROM thread_collaborators c WHERE c.thread_id = t.id AND c.user_id = ?) AS "shared!: bool"
         FROM threads t WHERE t.family_id = ?"#,
        user,
        family,
    )
    .fetch_all(&server.shared.db)
    .await?;
    let mut rights = ThreadRights { visible: Vec::new(), writable: Vec::new(), known: Vec::new() };
    for row in rows {
        let thread = ThreadId(crate::id_from(&row.id));
        rights.known.push(thread);
        if viewer.sees_thread(thread) {
            rights.visible.push(thread);
        }
        if row.owner_id == user || row.shared {
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
    if status.success() { Ok(()) } else { Err(std::io::Error::other("git init --bare failed")) }
}

/// The most CGI header `http-backend` may send before its body.
const MAX_CGI_HEAD: usize = 64 * 1024;

/// Runs `git http-backend` for one request, streaming the request body in
/// and the response body out, so neither a pack pushed nor one fetched is
/// held in memory.
async fn run_backend(
    root: &Path,
    path_info: &str,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Body,
    access: &[(String, String)],
) -> std::io::Result<Response> {
    let text_of = |name: &str| headers.get(name).and_then(|value| value.to_str().ok()).unwrap_or_default();
    let mut command = Command::new("git");
    command
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("PATH_INFO", path_info)
        .env("REQUEST_METHOD", method.as_str())
        .env("QUERY_STRING", uri.query().unwrap_or_default())
        .env("CONTENT_TYPE", text_of("content-type"))
        .env("HTTP_CONTENT_ENCODING", text_of("content-encoding"))
        .env("GIT_PROTOCOL", text_of("git-protocol"))
        .env("REMOTE_ADDR", "127.0.0.1")
        .envs(access.iter().map(|(key, value)| (key, value)))
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Without a length, as for a chunked push, http-backend reads to the end.
    if let Some(length) = headers.get(header::CONTENT_LENGTH).and_then(|value| value.to_str().ok()) {
        command.env("CONTENT_LENGTH", length);
    }
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().expect("stdin is piped");
    tokio::spawn(async move {
        if let Err(error) = feed(stdin, body).await {
            tracing::debug!(%error, "stopped sending a request to git http-backend");
        }
    });
    let errors = tokio::spawn(read_capped(child.stderr.take().expect("stderr is piped")));
    let mut stdout = child.stdout.take().expect("stdout is piped");

    let mut head = Vec::new();
    let split = loop {
        if let Some(split) = cgi_split(&head) {
            break split;
        }
        if head.len() > MAX_CGI_HEAD {
            return Err(std::io::Error::other("git http-backend sent too long a header"));
        }
        let mut buffer = [0u8; 8192];
        let read = stdout.read(&mut buffer).await?;
        if read == 0 {
            if head.is_empty() {
                let status = child.wait().await?;
                let errors = errors.await.unwrap_or_default();
                return Err(std::io::Error::other(format!("git http-backend {status}: {}", String::from_utf8_lossy(&errors))));
            }
            break (0, 0);
        }
        head.extend_from_slice(&buffer[..read]);
    };
    tokio::spawn(async move {
        match child.wait().await {
            Ok(status) if status.success() => {}
            outcome => {
                let errors = errors.await.unwrap_or_default();
                tracing::warn!(?outcome, errors = %String::from_utf8_lossy(&errors), "git http-backend failed");
            }
        }
    });
    let (status, response_headers) = cgi_head(&head[..split.0]);
    let rest = Bytes::copy_from_slice(&head[split.1..]);
    let body = futures_util::stream::once(async move { Ok(rest) }).chain(ReaderStream::new(stdout));
    Ok((status, response_headers, Body::from_stream(body)).into_response())
}

/// Copies the request body to http-backend, stopping at the size of the
/// largest pack a push may send.
async fn feed(mut stdin: ChildStdin, body: Body) -> std::io::Result<()> {
    let mut chunks = body.into_data_stream();
    let mut sent = 0u64;
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(std::io::Error::other)?;
        sent += chunk.len() as u64;
        if sent > MAX_PACK {
            return Err(std::io::Error::other("the request is larger than a push may be"));
        }
        stdin.write_all(&chunk).await?;
    }
    Ok(())
}

/// The first part of a stream, for error messages.
async fn read_capped(stream: impl AsyncRead + Unpin) -> Vec<u8> {
    let mut kept = Vec::new();
    let _ = stream.take(MAX_CGI_HEAD as u64).read_to_end(&mut kept).await;
    kept
}

/// Where CGI output's header ends and its body starts, once it holds both.
fn cgi_split(output: &[u8]) -> Option<(usize, usize)> {
    if let Some(end) = output.windows(4).position(|window| window == b"\r\n\r\n") {
        return Some((end, end + 4));
    }
    output.windows(2).position(|window| window == b"\n\n").map(|end| (end, end + 2))
}

/// The status and headers in a CGI header.
fn cgi_head(head: &[u8]) -> (StatusCode, HeaderMap) {
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
    (status, response_headers)
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
    fn cgi_output_splits_into_status_headers_and_body() {
        let output = b"Status: 404 Not Found\r\nContent-Type: text/plain\r\n\r\nnope";
        let (end, body) = cgi_split(output).expect("a whole header");
        assert_eq!(&output[body..], b"nope");
        let (status, headers) = cgi_head(&output[..end]);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(headers["content-type"], "text/plain");
        assert_eq!(cgi_split(b"Content-Type: x\n\nbody"), Some((15, 17)));
        assert_eq!(cgi_split(b"Content-Type: x\r\n"), None, "the header isn't over yet");
    }
}
