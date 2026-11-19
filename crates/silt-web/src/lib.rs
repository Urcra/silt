//! The web UI from `web/dist`, embedded once for the daemon at `/` and the
//! server at `/app/`. `build.rs` builds it when the checkout has none.

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
pub struct Assets;

/// How long a browser may keep the UI's file at `path`: the build names
/// what's under `assets/` by its content, so those never change, and the
/// window loaded all of them again at each start.
pub fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_files_named_by_their_content_are_kept() {
        assert!(super::cache_control("assets/index-CLAhXZOp.js").contains("immutable"));
        assert_eq!(super::cache_control("sw.js"), "no-cache");
        assert_eq!(super::cache_control("manifest.webmanifest"), "no-cache");
    }
}
