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

/// One of IBM Plex's files the UI's build carries, by its name without the
/// hash the build adds, such as `ibm-plex-sans-latin-400-normal.woff2`: for
/// the plain pages outside the UI, so they draw in its faces rather than the
/// machine's own sans. None for any other name, or without a built UI.
pub fn font(name: &str) -> Option<Vec<u8>> {
    let stem = name.strip_suffix(".woff2").filter(|stem| stem.starts_with("ibm-plex-"))?;
    let prefix = format!("assets/{stem}-");
    Assets::iter()
        .find(|path| path.strip_prefix(&prefix).is_some_and(|hash| hash.ends_with(".woff2")))
        .and_then(|path| Assets::get(&path))
        .map(|file| file.data.into_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_files_named_by_their_content_are_kept() {
        assert!(super::cache_control("assets/index-CLAhXZOp.js").contains("immutable"));
        assert_eq!(super::cache_control("sw.js"), "no-cache");
        assert_eq!(super::cache_control("manifest.webmanifest"), "no-cache");
    }

    #[test]
    fn fonts_are_plex_s_alone_by_their_name_without_the_build_s_hash() {
        assert!(super::font("index.html").is_none());
        assert!(super::font("../index.html").is_none());
        // Only where the UI was built: a checkout without `web/dist` embeds none.
        if super::Assets::get("index.html").is_some() {
            for name in ["ibm-plex-sans-latin-400-normal.woff2", "ibm-plex-mono-latin-400-normal.woff2"] {
                assert!(super::font(name).is_some_and(|data| data.starts_with(b"wOF2")), "{name}");
            }
        }
    }
}
