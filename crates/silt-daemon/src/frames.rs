//! Frame keys on the daemon (ADR 0074): each opens one version of one
//! artifact, whose page is composed afresh from the thread's history each
//! time it's served, so a key stops working once its artifact does.

use silt_design::frame::Theme;

/// What a frame key opens, and how its page is composed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameGrant {
    pub(crate) thread: String,
    pub(crate) artifact: String,
    /// The version, or the latest when none.
    pub(crate) version: Option<u32>,
    pub(crate) theme: Option<Theme>,
    /// The origin of the page holding the frame.
    pub(crate) parent: Option<String>,
    /// What the holding page kept for this viewer, as a JSON object (ADR 0072).
    pub(crate) boot: Option<String>,
}

/// `boot` from a frame request: an object within the limit, as JSON text.
pub(crate) fn boot_of(boot: Option<serde_json::Value>) -> Option<String> {
    boot.filter(serde_json::Value::is_object)
        .map(|boot| boot.to_string())
        .filter(|text| text.len() <= silt_design::frame::BOOT_BYTES)
}

pub(crate) type FrameKeys = silt_proto::frames::FrameKeys<FrameGrant>;

/// 256 random bits as hex.
pub(crate) fn new_key() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}
