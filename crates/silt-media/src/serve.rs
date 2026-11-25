//! A blob as an HTTP response, whole or by range, for the daemon's and the
//! server's routes (ADR 0121).

use std::path::Path;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::range::{Ranged, range};

/// A page a blob is opened as runs nothing and loads nothing, whatever it
/// holds: an SVG or an HTML file is shown, never run, on Silt's origin.
const SANDBOX: &str = "sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'";

/// The media type a blob is served as: an image or a video it is, by its
/// first bytes, else bytes to save.
pub async fn media_type_of(path: &Path) -> std::io::Result<&'static str> {
    let mut head = vec![0u8; crate::HEAD];
    let mut file = tokio::fs::File::open(path).await?;
    let mut read = 0;
    while read < head.len() {
        let got = file.read(&mut head[read..]).await?;
        if got == 0 {
            break;
        }
        read += got;
    }
    Ok(crate::sniff(&head[..read]).map_or("application/octet-stream", |(media_type, _)| media_type))
}

/// The file at `path`, all of it or the part `range_header` asks for, as
/// immutable: a blob named by its hash never changes. `download` names the
/// file to save it as.
pub async fn file(path: &Path, etag: &str, range_header: Option<&str>, download: Option<&str>) -> Response {
    match respond(path, etag, range_header, download).await {
        Ok(response) => response,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn respond(path: &Path, etag: &str, range_header: Option<&str>, download: Option<&str>) -> std::io::Result<Response> {
    let size = tokio::fs::metadata(path).await?.len();
    let media_type = media_type_of(path).await?;
    let mut builder = Response::builder()
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header(header::ETAG, format!("\"{etag}\""))
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, SANDBOX)
        .header(header::CONTENT_TYPE, media_type);
    let disposition = match download {
        Some(name) => Some(format!("attachment; filename=\"{}\"", safe_name(name))),
        None if media_type == "application/octet-stream" => Some("attachment".to_owned()),
        None => None,
    };
    if let Some(disposition) = disposition.and_then(|value| HeaderValue::from_str(&value).ok()) {
        builder = builder.header(header::CONTENT_DISPOSITION, disposition);
    }
    let (status, start, length) = match range(range_header, size) {
        Ranged::Whole => (StatusCode::OK, 0, size),
        Ranged::Part { start, end } => {
            builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
            (StatusCode::PARTIAL_CONTENT, start, end - start + 1)
        }
        Ranged::Unsatisfiable => {
            return Ok(builder
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .body(Body::empty())
                .unwrap_or_default());
        }
    };
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let body = Body::from_stream(tokio_util::io::ReaderStream::with_capacity(file.take(length), 256 << 10));
    Ok(builder.status(status).header(header::CONTENT_LENGTH, length).body(body).unwrap_or_default())
}

/// A name for the save dialog, without what would end the header's quotes.
fn safe_name(name: &str) -> String {
    let name: String = name.chars().filter(|char| !char.is_control() && !matches!(char, '"' | '\\' | '/')).collect();
    if name.trim().is_empty() { "file".to_owned() } else { name }
}

/// Where the preview of `edge` px of the image at `original` is kept under
/// `cache`, made the first time it's asked for; the original itself when
/// it's no larger, or can't be decoded.
pub async fn preview(original: &Path, cache: &Path, key: &str, edge: u32) -> std::path::PathBuf {
    let cached = cache.join(format!("{key}-{edge}"));
    if cached.is_file() {
        return cached;
    }
    let (from, to) = (original.to_owned(), cached.clone());
    let made = tokio::task::spawn_blocking(move || -> Result<bool, crate::MediaError> {
        let Some(preview) = crate::preview(&from, edge)? else {
            return Ok(false);
        };
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = to.with_extension("tmp");
        std::fs::write(&temporary, preview.bytes)?;
        std::fs::rename(&temporary, &to)?;
        Ok(true)
    })
    .await;
    if matches!(made, Ok(Ok(true))) { cached } else { original.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body(response: Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("body").to_vec()
    }

    #[tokio::test]
    async fn a_blob_is_served_whole_or_by_range_with_its_type() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("blob");
        let mut bytes = b"\x1a\x45\xdf\xa3\x9f\x42\x86\x81\x01\x42\x82\x84webm".to_vec();
        bytes.extend((0..1000u32).map(|n| n as u8));
        std::fs::write(&path, &bytes).expect("write");
        let whole = file(&path, "abc", None, None).await;
        assert_eq!(whole.status(), StatusCode::OK);
        assert_eq!(whole.headers()[header::CONTENT_TYPE], "video/webm");
        assert_eq!(whole.headers()[header::ACCEPT_RANGES], "bytes");
        assert!(whole.headers().get(header::CONTENT_DISPOSITION).is_none());
        assert_eq!(body(whole).await, bytes);
        let part = file(&path, "abc", Some("bytes=4-7"), None).await;
        assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(part.headers()[header::CONTENT_RANGE], format!("bytes 4-7/{}", bytes.len()));
        assert_eq!(body(part).await, &bytes[4..8]);
        let past = file(&path, "abc", Some("bytes=5000-"), None).await;
        assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        let saved = file(&path, "abc", None, Some("a \"clip\".webm")).await;
        assert_eq!(saved.headers()[header::CONTENT_DISPOSITION], "attachment; filename=\"a clip.webm\"");
        assert_eq!(file(&dir.path().join("none"), "abc", None, None).await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn anything_else_is_bytes_to_save_and_runs_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("blob");
        std::fs::write(&path, b"<html><script>alert(1)</script></html>").expect("write");
        let response = file(&path, "abc", None, None).await;
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/octet-stream");
        assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "attachment");
        assert!(response.headers()[header::CONTENT_SECURITY_POLICY].to_str().expect("csp").starts_with("sandbox"));
    }
}
