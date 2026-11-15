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
}

pub(crate) type FrameKeys = silt_proto::frames::FrameKeys<FrameGrant>;

/// 256 random bits as hex.
pub(crate) fn new_key() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}
