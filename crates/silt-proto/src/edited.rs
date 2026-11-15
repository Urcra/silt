//! Docs and sheets (ADR 0077): artifacts edited in place, whose state is
//! the thread's page data. What a request asks becomes page ops here, alike
//! on the daemon and the server, and a sheet's values are computed here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use silt_log::page::SheetCells;
use silt_log::{Axis, PageOp};
use silt_types::ArtifactId;

/// The kinds edited in place.
pub const EDITED: &[&str] = &["doc", "sheet", "whiteboard"];
/// The longest doc, in bytes.
pub const DOC_TEXT_BYTES: usize = 1 << 20;

pub fn is_edited(kind: &str) -> bool {
    EDITED.contains(&kind)
}

/// A change to a doc's text: replace `old`, which occurs once, with `new`;
/// or at character `pos`, delete `delete` characters and insert `insert`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DocEdit {
    Replace { old: String, new: String },
    Splice { pos: usize, delete: usize, insert: String },
}

/// What a doc's page or an agent asks of a doc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DocRequest {
    Read,
    Edit { edits: Vec<DocEdit> },
}

/// The text operations that make `edits` to `text`, in order, each against
/// the text the ones before it left; all or none.
pub fn doc_ops(text: &str, edits: &[DocEdit]) -> Result<Vec<silt_crdt::TextOp>, String> {
    use silt_crdt::TextOp;
    let mut current: Vec<char> = text.chars().collect();
    let mut ops = Vec::new();
    for (at, edit) in edits.iter().enumerate() {
        let (pos, delete, insert) = match edit {
            DocEdit::Replace { old, new } => {
                if old.is_empty() {
                    return Err(format!("edit {}: its old text is empty", at + 1));
                }
                let haystack: String = current.iter().collect();
                let mut found = haystack.match_indices(old.as_str());
                let (byte, _) = found.next().ok_or_else(|| format!("edit {}: its old text isn't in the doc", at + 1))?;
                if found.next().is_some() {
                    return Err(format!("edit {}: its old text occurs more than once; give more of it", at + 1));
                }
                (haystack[..byte].chars().count(), old.chars().count(), new.clone())
            }
            DocEdit::Splice { pos, delete, insert } => (*pos, *delete, insert.clone()),
        };
        if pos.checked_add(delete).is_none_or(|end| end > current.len()) {
            return Err(format!("edit {}: past the end of the doc", at + 1));
        }
        if delete > 0 {
            ops.push(TextOp::Delete { pos, len: delete });
        }
        if !insert.is_empty() {
            ops.push(TextOp::Insert { pos, content: insert.clone() });
        }
        current.splice(pos..pos + delete, insert.chars());
        if current.len() > DOC_TEXT_BYTES {
            return Err("the doc would be over 1 MiB".into());
        }
    }
    Ok(ops)
}

/// What a sheet's page or an agent asks of a sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SheetRequest {
    /// The sheet's cells and values; with `range` such as `A1:D20`, only those.
    Read {
        #[serde(default)]
        sheet: u16,
        #[serde(default)]
        range: Option<String>,
    },
    /// Cells typed by address, `""` clearing one.
    Set {
        #[serde(default)]
        sheet: u16,
        cells: BTreeMap<String, String>,
    },
    /// Rows or columns put in before `at`, from 0.
    Insert {
        #[serde(default)]
        sheet: u16,
        axis: AxisName,
        at: usize,
        #[serde(default = "one")]
        count: usize,
    },
    /// Rows or columns taken out from `at`, cells and all.
    Remove {
        #[serde(default)]
        sheet: u16,
        axis: AxisName,
        at: usize,
        #[serde(default = "one")]
        count: usize,
    },
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisName {
    Rows,
    Columns,
}

impl From<AxisName> for Axis {
    fn from(name: AxisName) -> Self {
        match name {
            AxisName::Rows => Axis::Rows,
            AxisName::Columns => Axis::Columns,
        }
    }
}

/// A sheet as its page shows it: rows and columns by id, what each cell
/// holds (formulas with addresses) and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SheetView {
    pub rows: Vec<String>,
    pub columns: Vec<String>,
    /// By row, then column; `""` for an empty cell.
    pub raw: Vec<Vec<String>>,
    pub values: Vec<Vec<String>>,
}

fn id_text(id: &silt_crdt::CharId) -> String {
    match id {
        silt_crdt::CharId::Delta(delta) => delta.to_string(),
        silt_crdt::CharId::Base { source, offset } => format!("{source}+{offset}"),
    }
}

/// The sheet's view, values computed as of `today`, a date serial.
pub fn sheet_view(cells: &SheetCells, today: f64) -> SheetView {
    let rows: Vec<String> = cells.rows.iter().map(id_text).collect();
    let columns: Vec<String> = cells.columns.iter().map(id_text).collect();
    let raw: Vec<Vec<String>> = cells
        .rows
        .iter()
        .map(|row| {
            cells
                .columns
                .iter()
                .map(|column| {
                    cells.cells.get(&(*row, *column)).map_or_else(String::new, |raw| silt_sheet::from_ids(raw, &rows, &columns))
                })
                .collect()
        })
        .collect();
    let values = silt_sheet::evaluate(&silt_sheet::Sheet { cells: &raw, today })
        .into_iter()
        .map(|row| row.into_iter().map(|value| value.to_string()).collect())
        .collect();
    SheetView { rows, columns, raw, values }
}

/// What a `Set` needs first: how many rows and columns to add at the end so
/// every address it names exists.
pub fn sheet_growth(cells: &SheetCells, set: &BTreeMap<String, String>) -> Result<(usize, usize), String> {
    let mut needed = (0, 0);
    for address in set.keys() {
        let (row, column) = silt_sheet::parse_address(address).ok_or_else(|| format!("{address} isn't a cell's address"))?;
        if row >= silt_sheet::ROWS || column >= silt_sheet::COLUMNS {
            return Err(format!("{address} is past a sheet's {} rows or {} columns", silt_sheet::ROWS, silt_sheet::COLUMNS));
        }
        needed = (needed.0.max(row + 1), needed.1.max(column + 1));
    }
    Ok((needed.0.saturating_sub(cells.rows.len()), needed.1.saturating_sub(cells.columns.len())))
}

/// The cell writes a `Set` makes once its rows and columns exist, each
/// formula kept with its references by ids.
pub fn cell_ops(
    artifact: ArtifactId,
    sheet: u16,
    cells: &SheetCells,
    set: &BTreeMap<String, String>,
) -> Result<Vec<PageOp>, String> {
    let rows: Vec<String> = cells.rows.iter().map(id_text).collect();
    let columns: Vec<String> = cells.columns.iter().map(id_text).collect();
    set.iter()
        .map(|(address, raw)| {
            let (row, column) = silt_sheet::parse_address(address).ok_or_else(|| format!("{address} isn't a cell's address"))?;
            let (Some(row), Some(column)) = (cells.rows.get(row), cells.columns.get(column)) else {
                return Err(format!("{address} isn't in the sheet"));
            };
            if raw.len() > silt_log::page::CELL_BYTES {
                return Err(format!("{address} is over {} KiB", silt_log::page::CELL_BYTES >> 10));
            }
            Ok(PageOp::Cell { artifact, sheet, row: *row, column: *column, raw: silt_sheet::to_ids(raw, &rows, &columns) })
        })
        .collect()
}

/// The values in `range` of `view`, such as `A1:D20`, by row.
pub fn read_range(view: &SheetView, range: &str) -> Result<Vec<Vec<String>>, String> {
    let (start, end) = range.split_once(':').unwrap_or((range, range));
    let (Some((top, left)), Some((bottom, right))) = (silt_sheet::parse_address(start), silt_sheet::parse_address(end)) else {
        return Err(format!("{range} isn't a range"));
    };
    Ok((top.min(bottom)..=top.max(bottom))
        .map(|row| {
            (left.min(right)..=left.max(right))
                .map(|column| view.values.get(row).and_then(|cells| cells.get(column)).cloned().unwrap_or_default())
                .collect()
        })
        .collect())
}

/// A sheet as CSV: what each cell shows.
pub fn csv(view: &SheetView) -> String {
    let cell =
        |text: &str| if text.contains([',', '"', '\n']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_owned() };
    view.values.iter().map(|row| row.iter().map(|value| cell(value)).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join("\n")
}

/// A sheet as an `.xlsx` workbook, formulas kept as formulas with their
/// values cached, so it opens alike in a spreadsheet app.
#[cfg(feature = "xlsx")]
pub fn xlsx(view: &SheetView, name: &str) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::{Formula, Workbook};
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    let title: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if !title.trim().is_empty() {
        sheet.set_name(title.trim()).map_err(|error| error.to_string())?;
    }
    for (row, cells) in view.raw.iter().enumerate() {
        for (column, raw) in cells.iter().enumerate() {
            let (r, c) =
                (u32::try_from(row).map_err(|_| "too many rows")?, u16::try_from(column).map_err(|_| "too many columns")?);
            let value = view.values.get(row).and_then(|values| values.get(column)).cloned().unwrap_or_default();
            let written = if raw.starts_with('=') {
                sheet.write_formula(r, c, Formula::new(raw.as_str()).set_result(value)).map(|_| ())
            } else if raw.is_empty() {
                Ok(())
            } else if let Ok(number) = raw.trim().parse::<f64>() {
                sheet.write_number(r, c, number).map(|_| ())
            } else {
                sheet.write_string(r, c, raw.as_str()).map(|_| ())
            };
            written.map_err(|error| error.to_string())?;
        }
    }
    book.save_to_buffer().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use silt_crdt::TextOp;

    #[test]
    fn edits_become_text_operations_each_against_the_text_before_it() {
        let ops = doc_ops(
            "# Plan\nship it\n",
            &[
                DocEdit::Replace { old: "ship it".into(), new: "ship it Friday".into() },
                DocEdit::Splice { pos: 0, delete: 2, insert: String::new() },
            ],
        )
        .expect("ops");
        assert_eq!(
            ops,
            [
                TextOp::Delete { pos: 7, len: 7 },
                TextOp::Insert { pos: 7, content: "ship it Friday".into() },
                TextOp::Delete { pos: 0, len: 2 }
            ]
        );
        assert!(doc_ops("a a", &[DocEdit::Replace { old: "a".into(), new: "b".into() }]).is_err(), "ambiguous");
        assert!(doc_ops("abc", &[DocEdit::Splice { pos: 2, delete: 5, insert: String::new() }]).is_err());
    }

    #[cfg(feature = "xlsx")]
    #[test]
    fn a_workbook_keeps_formulas_and_their_values() {
        use calamine::{Reader, Xlsx};
        let view = SheetView {
            rows: vec!["r0".into(), "r1".into()],
            columns: vec!["c0".into()],
            raw: vec![vec!["2".into()], vec!["=A1*3".into()]],
            values: vec![vec!["2".into()], vec!["6".into()]],
        };
        let bytes = xlsx(&view, "Tracker").expect("a workbook");
        let mut book: Xlsx<_> = calamine::open_workbook_from_rs(std::io::Cursor::new(bytes)).expect("opens");
        let formulas = book.worksheet_formula("Tracker").expect("formulas");
        assert_eq!(formulas.get_value((1, 0)).map(String::as_str), Some("A1*3"));
        let values = book.worksheet_range("Tracker").expect("values");
        assert_eq!(values.get_value((0, 0)).map(ToString::to_string).as_deref(), Some("2"));
        assert_eq!(csv(&view), "2\n6");
    }

    #[test]
    fn edits_and_requests_read_as_json() {
        let edit: DocEdit = serde_json::from_value(serde_json::json!({"old": "a", "new": "b"})).expect("replace");
        assert_eq!(edit, DocEdit::Replace { old: "a".into(), new: "b".into() });
        let set: SheetRequest = serde_json::from_value(serde_json::json!({"op": "set", "cells": {"A1": "3"}})).expect("set");
        assert!(matches!(set, SheetRequest::Set { sheet: 0, .. }));
    }
}
