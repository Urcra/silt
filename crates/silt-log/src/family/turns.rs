//! What a session's turn marks say: which tool is open, read from the log
//! rather than from one process's memory, so any process recording the
//! session attributes edits alike and can close a tool another opened, and
//! which marks are recorded already, so recording one twice changes nothing
//! (ADR 0018).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::delta::TurnMark;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct Turns {
    /// The tool started last and not finished since, unless the turn ended.
    open_tool: Option<String>,
    started: HashSet<String>,
    finished: HashSet<String>,
    last: Option<TurnMark>,
}

impl Turns {
    pub(super) fn record(&mut self, mark: &TurnMark) {
        match mark {
            TurnMark::PromptSubmitted | TurnMark::Stopped => self.open_tool = None,
            TurnMark::ToolStarted { tool_use_id, .. } => {
                self.started.insert(tool_use_id.clone());
                self.open_tool = Some(tool_use_id.clone());
            }
            TurnMark::ToolFinished { tool_use_id } => {
                self.finished.insert(tool_use_id.clone());
                if self.open_tool.as_ref() == Some(tool_use_id) {
                    self.open_tool = None;
                }
            }
        }
        self.last = Some(mark.clone());
    }

    pub(super) fn open_tool(&self) -> Option<&str> {
        self.open_tool.as_deref()
    }

    pub(super) fn in_turn(&self) -> bool {
        self.last.as_ref().is_some_and(|mark| *mark != TurnMark::Stopped)
    }

    /// Whether `mark` is one already recorded: a tool's start or finish
    /// seen before, or a prompt or stop right after the same.
    pub(super) fn has(&self, mark: &TurnMark) -> bool {
        match mark {
            TurnMark::ToolStarted { tool_use_id, .. } => self.started.contains(tool_use_id),
            TurnMark::ToolFinished { tool_use_id } => self.finished.contains(tool_use_id),
            TurnMark::PromptSubmitted | TurnMark::Stopped => self.last.as_ref() == Some(mark),
        }
    }
}
