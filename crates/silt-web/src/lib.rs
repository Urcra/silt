//! The web UI from `web/dist`, embedded once for the daemon at `/` and the
//! server at `/app/`. `build.rs` builds it when the checkout has none.

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
pub struct Assets;
