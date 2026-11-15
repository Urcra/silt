//! Docs and sheets on the daemon (ADR 0077): their text and cells are the
//! thread's page data, edited by people and the agent at once and folded
//! along the thread's lineage.

use silt_log::PageOp;
use silt_proto::edited::{DocRequest, SheetRequest};
use silt_proto::page::Level;
use silt_types::{ArtifactId, ThreadId};

use crate::core::{Core, CoreError};

impl Core {
    /// A doc's text, or edits of it as this machine's person.
    pub(crate) async fn doc_data(
        &mut self,
        thread: &str,
        artifact: &str,
        asked: DocRequest,
    ) -> Result<serde_json::Value, CoreError> {
        let thread = self.find_thread(thread)?;
        let id = self.edited_artifact(thread, artifact, "doc")?;
        match asked {
            DocRequest::Read => Ok(serde_json::json!({ "text": self.doc_text(thread, id) })),
            DocRequest::Edit { edits } => {
                self.may_edit(thread, id)?;
                let text = self.doc_text(thread, id);
                let ops = silt_proto::edited::doc_ops(&text, &edits).map_err(CoreError::Invalid)?;
                let family = self.threads[&thread].family;
                for op in ops {
                    self.append_page(family, thread, PageOp::Text { artifact: id, op }).await?;
                }
                Ok(serde_json::json!({ "text": self.doc_text(thread, id) }))
            }
        }
    }

    /// A sheet's cells and values, or changes to it as this machine's person.
    pub(crate) async fn sheet_data(
        &mut self,
        thread: &str,
        artifact: &str,
        asked: SheetRequest,
    ) -> Result<serde_json::Value, CoreError> {
        use silt_crdt::TextOp;
        let thread = self.find_thread(thread)?;
        let id = self.edited_artifact(thread, artifact, "sheet")?;
        let family = self.threads[&thread].family;
        let lines = |sheet, axis, op| PageOp::Lines { artifact: id, sheet, axis, op };
        match asked {
            SheetRequest::Read { sheet, range } => {
                let view = silt_proto::edited::sheet_view(&self.sheet_cells(thread, id, sheet), today());
                match range {
                    Some(range) => Ok(
                        serde_json::json!({ "values": silt_proto::edited::read_range(&view, &range).map_err(CoreError::Invalid)? }),
                    ),
                    None => Ok(serde_json::to_value(view).unwrap_or_default()),
                }
            }
            SheetRequest::Set { sheet, cells } => {
                self.may_edit(thread, id)?;
                let (rows, columns) =
                    silt_proto::edited::sheet_growth(&self.sheet_cells(thread, id, sheet), &cells).map_err(CoreError::Invalid)?;
                if rows > 0 {
                    let at = self.sheet_cells(thread, id, sheet).rows.len();
                    self.append_page(
                        family,
                        thread,
                        lines(sheet, silt_log::Axis::Rows, TextOp::Insert { pos: at, content: "r".repeat(rows) }),
                    )
                    .await?;
                }
                if columns > 0 {
                    let at = self.sheet_cells(thread, id, sheet).columns.len();
                    let op = TextOp::Insert { pos: at, content: "c".repeat(columns) };
                    self.append_page(family, thread, lines(sheet, silt_log::Axis::Columns, op)).await?;
                }
                let ops = silt_proto::edited::cell_ops(id, sheet, &self.sheet_cells(thread, id, sheet), &cells)
                    .map_err(CoreError::Invalid)?;
                let written = ops.len();
                for op in ops {
                    self.append_page(family, thread, op).await?;
                }
                Ok(serde_json::json!({ "written": written }))
            }
            SheetRequest::Insert { sheet, axis, at, count } => {
                self.may_edit(thread, id)?;
                let len = axis_len(&self.sheet_cells(thread, id, sheet), axis);
                let most = match axis {
                    silt_proto::edited::AxisName::Rows => silt_sheet::ROWS,
                    silt_proto::edited::AxisName::Columns => silt_sheet::COLUMNS,
                };
                if count == 0 || len + count > most {
                    return Err(CoreError::Invalid(format!("a sheet has at most {most} of them")));
                }
                let mark = if axis == silt_proto::edited::AxisName::Rows { "r" } else { "c" };
                let op = TextOp::Insert { pos: at.min(len), content: mark.repeat(count) };
                self.append_page(family, thread, lines(sheet, axis.into(), op)).await?;
                Ok(serde_json::json!({ "inserted": count }))
            }
            SheetRequest::Remove { sheet, axis, at, count } => {
                self.may_edit(thread, id)?;
                let len = axis_len(&self.sheet_cells(thread, id, sheet), axis);
                if count == 0 || at.checked_add(count).is_none_or(|end| end > len) {
                    return Err(CoreError::Invalid("those aren't in the sheet".into()));
                }
                self.append_page(family, thread, lines(sheet, axis.into(), TextOp::Delete { pos: at, len: count })).await?;
                Ok(serde_json::json!({ "removed": count }))
            }
        }
    }

    fn doc_text(&self, thread: ThreadId, artifact: ArtifactId) -> String {
        let family_id = self.threads[&thread].family;
        match self.families.get(&family_id) {
            Some(family) => family.doc_text(thread, artifact),
            None => {
                let log = self.local_log(family_id);
                silt_log::doc_text_in(log, &crate::page::graph_of(log), thread, artifact)
            }
        }
    }

    fn sheet_cells(&self, thread: ThreadId, artifact: ArtifactId, sheet: u16) -> silt_log::page::SheetCells {
        let family_id = self.threads[&thread].family;
        match self.families.get(&family_id) {
            Some(family) => family.sheet_cells(thread, artifact, sheet),
            None => {
                let log = self.local_log(family_id);
                silt_log::sheet_cells_in(log, &crate::page::graph_of(log), thread, artifact, sheet)
            }
        }
    }

    /// The artifact `id` of the thread, which must be a `kind`.
    fn edited_artifact(&self, thread: ThreadId, id: &str, kind: &str) -> Result<ArtifactId, CoreError> {
        let parsed: ArtifactId = id.parse().map_err(|_| CoreError::Invalid(format!("{id} isn't an artifact id")))?;
        let found = self
            .history_entries(thread)
            .iter()
            .filter_map(|(_, entry)| entry.artifact.as_ref())
            .find(|view| view.id == id)
            .map(|view| view.kind.clone())
            .ok_or_else(|| CoreError::Invalid(format!("this thread has no artifact {id}")))?;
        if found != kind {
            return Err(CoreError::Invalid(format!("artifact {id} is a {found}, not a {kind}")));
        }
        Ok(parsed)
    }

    /// Whether this machine's person may edit the doc or sheet: anyone who
    /// may comment, unless its version asks for a collaborator.
    fn may_edit(&self, thread: ThreadId, artifact: ArtifactId) -> Result<(), CoreError> {
        let needs = match self.declared(thread, artifact, "edit").ok().and_then(|edit| edit.get("level").cloned()) {
            Some(serde_json::Value::String(level)) if level == "collaborator" => Level::Collaborator,
            _ => Level::Comment,
        };
        if self.page_level(thread) < needs {
            return Err(CoreError::Invalid("you may not edit this".into()));
        }
        Ok(())
    }
}

fn axis_len(cells: &silt_log::page::SheetCells, axis: silt_proto::edited::AxisName) -> usize {
    match axis {
        silt_proto::edited::AxisName::Rows => cells.rows.len(),
        silt_proto::edited::AxisName::Columns => cells.columns.len(),
    }
}

/// Today as a date serial, days since 1899-12-30.
fn today() -> f64 {
    (crate::core::now_ms() as f64 / 86_400_000.0).floor() + 25_569.0
}
