//! Reading conversation steps out of a log of deltas, whichever agent
//! recorded them.

use silt_agents::history::HistoryReader;
use silt_agents::tool::Shown;
use silt_agents::{Entry, Transcripts};
use silt_log::{ConvOp, Delta, Op, SessionFile};

use crate::sessions::file_key;

/// The steps a delta adds to its session's own conversation, leaving out
/// its subagents'; `transcripts` must have seen the session's earlier
/// deltas, in order.
pub(crate) fn native_entries(transcripts: &mut Transcripts, delta: &Delta) -> Vec<Entry> {
    match &delta.op {
        Op::Conv(ConvOp::Native { session, file: SessionFile::Main, line }) => {
            transcripts.entries(&session.agent, &session.id, &file_key(&SessionFile::Main), line)
        }
        _ => Vec::new(),
    }
}

/// The steps a delta adds, its subagents' among them, as search finds them.
pub(crate) fn all_entries(transcripts: &mut Transcripts, delta: &Delta) -> Vec<Entry> {
    match &delta.op {
        Op::Conv(ConvOp::Native { session, file, line }) => {
            transcripts.entries(&session.agent, &session.id, &file_key(file), line)
        }
        _ => Vec::new(),
    }
}

/// What a delta adds to a thread's history, a subagent's entries marked
/// with its transcript.
pub(crate) fn shown_entries(reader: &mut HistoryReader, delta: &Delta) -> Vec<Shown> {
    match &delta.op {
        Op::Conv(ConvOp::Native { session, file, line }) => {
            let side = match file {
                SessionFile::Main => None,
                SessionFile::Subagent(name) => Some(name.as_str()),
            };
            reader.line(&session.agent, &session.id, side, line)
        }
        _ => Vec::new(),
    }
}
