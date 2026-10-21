//! Reading conversation steps out of a log of deltas, whichever agent
//! recorded them.

use silt_agents::{Entry, Transcripts};
use silt_log::{ConvOp, Delta, Op};

use crate::sessions::file_key;

/// The steps a delta adds; `transcripts` must have seen the session's
/// earlier deltas, in order.
pub(crate) fn native_entries(transcripts: &mut Transcripts, delta: &Delta) -> Vec<Entry> {
    match &delta.op {
        Op::Conv(ConvOp::Native { session, file, line }) => {
            transcripts.entries(&session.agent, &session.id, &file_key(file), line)
        }
        _ => Vec::new(),
    }
}
