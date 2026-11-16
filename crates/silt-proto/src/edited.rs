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
    /// `base`, the version the edits' positions were counted against, as
    /// a read answered it; they merge with what others wrote since. None
    /// counts them against the text now.
    Edit {
        edits: Vec<DocEdit>,
        #[serde(default)]
        base: Option<String>,
    },
}

/// The most edits one request of a doc makes.
pub const DOC_EDITS: usize = 200;
/// The most cells one request of a sheet sets.
pub const SET_CELLS: usize = 1000;

/// The text operations that make `edits` to `text`, in order, each against
/// the text the ones before it left; all or none.
pub fn doc_ops(text: &str, edits: &[DocEdit]) -> Result<Vec<silt_crdt::TextOp>, String> {
    edited(text, edits).map(|(ops, _)| ops)
}

/// `edits`, counted against `seen`, the text at a version the person read
/// with each character's id, as edits of `now`, the text with what others
/// wrote since (ADR 0077). The change from what they read to what they
/// meant is found by character: what they deleted that's still there goes,
/// and what they typed goes after the last character before it that still
/// stands, so others' edits stay and theirs lands where they meant it.
pub fn rebased(
    seen: &[(char, silt_crdt::CharId)],
    now: &[(char, silt_crdt::CharId)],
    edits: &[DocEdit],
) -> Result<Vec<DocEdit>, String> {
    let read: String = seen.iter().map(|(char, _)| char).collect();
    let (_, meant) = edited(&read, edits)?;
    let read: Vec<char> = seen.iter().map(|(char, _)| *char).collect();
    let same_start = read.iter().zip(&meant).take_while(|(a, b)| a == b).count();
    let same_end = read[same_start..].iter().rev().zip(meant[same_start..].iter().rev()).take_while(|(a, b)| a == b).count();
    let (from, deleted_to) = (same_start, read.len() - same_end);
    let typed: String = meant[same_start..meant.len() - same_end].iter().collect();
    let at: std::collections::HashMap<silt_crdt::CharId, usize> =
        now.iter().enumerate().map(|(index, (_, id))| (*id, index)).collect();
    let mut gone: Vec<usize> = seen[from..deleted_to].iter().filter_map(|(_, id)| at.get(id).copied()).collect();
    gone.sort_unstable();
    let mut rebased = Vec::new();
    // Runs of what's still there, last first, so each position holds.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for index in gone {
        match runs.last_mut() {
            Some((start, len)) if *start + *len == index => *len += 1,
            _ => runs.push((index, 1)),
        }
    }
    for (pos, delete) in runs.into_iter().rev() {
        rebased.push(DocEdit::Splice { pos, delete, insert: String::new() });
    }
    if !typed.is_empty() {
        let before = seen[..from].iter().rev().find_map(|(_, id)| at.get(id));
        let pos = before.map_or(0, |index| index + 1);
        rebased.push(DocEdit::Splice { pos, delete: 0, insert: typed });
    }
    Ok(rebased)
}

/// [`doc_ops`], and the text they leave.
fn edited(text: &str, edits: &[DocEdit]) -> Result<(Vec<silt_crdt::TextOp>, Vec<char>), String> {
    use silt_crdt::TextOp;
    if edits.len() > DOC_EDITS {
        return Err(format!("a request makes at most {DOC_EDITS} edits"));
    }
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
    Ok((ops, current))
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
        /// The row's and column's ids each address had where the person
        /// typed it, so a row put in above meanwhile doesn't move it.
        #[serde(default)]
        ids: BTreeMap<String, (String, String)>,
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
    /// The sheet's name, and its columns' widths and number formats by
    /// column letter; `null`, or an empty name or format, clears one.
    Format {
        #[serde(default)]
        sheet: u16,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        widths: BTreeMap<String, Option<u16>>,
        #[serde(default)]
        formats: BTreeMap<String, Option<String>>,
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
/// holds (formulas with addresses) and its value, in its column's number
/// format, with each sheet's name and its columns' widths (ADR 0083).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct SheetView {
    pub rows: Vec<String>,
    pub columns: Vec<String>,
    /// By row, then column; `""` for an empty cell.
    pub raw: Vec<Vec<String>>,
    pub values: Vec<Vec<String>>,
    /// How many sheets the artifact has, for its tabs; a read fills it in.
    #[serde(default = "one_sheet")]
    pub sheets: u16,
    /// Each sheet's name for its tab, `Sheet 2` where it has none.
    #[serde(default)]
    pub names: Vec<String>,
    /// Each column's width in pixels, where someone set one.
    #[serde(default)]
    pub widths: Vec<Option<u16>>,
    /// Each column's number format, where someone set one.
    #[serde(default)]
    pub formats: Vec<Option<String>>,
}

/// The longest name a sheet takes.
pub const SHEET_NAME_CHARS: usize = 64;
/// The narrowest and widest a column is set, in pixels.
pub const COLUMN_WIDTHS: std::ops::RangeInclusive<u16> = 24..=800;

/// Where a sheet's settings are kept among its artifact's page documents:
/// written only through the sheet's own requests, at its edit level.
pub fn meta_path(sheet: u16) -> String {
    format!("_sheets/s{sheet}")
}

/// A sheet's name, and its columns' widths and number formats by column id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SheetMeta {
    pub name: Option<String>,
    pub widths: BTreeMap<String, u16>,
    pub formats: BTreeMap<String, String>,
}

/// The settings `doc` keeps, what isn't one a sheet takes left out, as a
/// peer may have written anything there.
pub fn sheet_meta(doc: Option<&serde_json::Value>) -> SheetMeta {
    let mut meta = SheetMeta::default();
    let Some(fields) = doc.and_then(serde_json::Value::as_object) else { return meta };
    for (field, value) in fields {
        if field == "name" {
            meta.name = value.as_str().filter(|name| name_ok(name)).map(str::to_owned);
        } else if let Some(column) = field.strip_prefix("w:") {
            if let Some(width) =
                value.as_u64().and_then(|width| u16::try_from(width).ok()).filter(|width| COLUMN_WIDTHS.contains(width))
            {
                meta.widths.insert(column.to_owned(), width);
            }
        } else if let Some(column) = field.strip_prefix("f:")
            && let Some(format) = value.as_str().filter(|format| silt_sheet::number_format_ok(format))
        {
            meta.formats.insert(column.to_owned(), format.to_owned());
        }
    }
    meta
}

fn name_ok(name: &str) -> bool {
    !name.trim().is_empty() && name.chars().count() <= SHEET_NAME_CHARS && !name.chars().any(char::is_control)
}

/// The settings of each of `count` sheets, from their artifact's documents.
pub fn metas_of(docs: &silt_log::page::Documents, count: usize) -> Vec<SheetMeta> {
    (0..count).map(|sheet| sheet_meta(u16::try_from(sheet).ok().and_then(|sheet| docs.get(&meta_path(sheet))))).collect()
}

/// The op that names sheet `sheet` and sets its columns' widths and
/// formats, by column letter as `cells` stands: `None`, or an empty name or
/// format, clears one.
pub fn format_op(
    artifact: ArtifactId,
    sheet: u16,
    cells: &SheetCells,
    name: Option<&str>,
    widths: &BTreeMap<String, Option<u16>>,
    formats: &BTreeMap<String, Option<String>>,
) -> Result<PageOp, String> {
    let mut fields = serde_json::Map::new();
    if let Some(name) = name {
        if !name.is_empty() && !name_ok(name) {
            return Err(format!("a sheet's name is 1 to {SHEET_NAME_CHARS} characters"));
        }
        fields.insert("name".into(), if name.is_empty() { serde_json::Value::Null } else { name.trim().into() });
    }
    let column_id = |letter: &str| {
        silt_sheet::parse_address(&format!("{letter}1"))
            .and_then(|(_, column)| cells.columns.get(column))
            .map(id_text)
            .ok_or_else(|| format!("{letter} isn't a column in the sheet"))
    };
    for (letter, width) in widths {
        let value = match width {
            Some(width) if COLUMN_WIDTHS.contains(width) => serde_json::Value::from(*width),
            Some(_) => return Err(format!("a column is {} to {} pixels wide", COLUMN_WIDTHS.start(), COLUMN_WIDTHS.end())),
            None => serde_json::Value::Null,
        };
        fields.insert(format!("w:{}", column_id(letter)?), value);
    }
    for (letter, format) in formats {
        let value = match format.as_deref() {
            None | Some("") => serde_json::Value::Null,
            Some(format) if silt_sheet::number_format_ok(format) => format.into(),
            Some(format) => {
                return Err(format!("{format} isn't a number format: use 0, 0.00, #,##0, 0%, or one after $, €, £ or ¥"));
            }
        };
        fields.insert(format!("f:{}", column_id(letter)?), value);
    }
    if fields.is_empty() {
        return Err("give a name, widths or formats".into());
    }
    Ok(PageOp::Patched { artifact, path: meta_path(sheet), fields: serde_json::Value::Object(fields).to_string() })
}

fn one_sheet() -> u16 {
    1
}

/// The most sheets an artifact has, as the log takes them.
pub const SHEETS: u16 = 16;

impl SheetRequest {
    /// Which sheet it's about.
    pub fn sheet(&self) -> u16 {
        match self {
            Self::Read { sheet, .. }
            | Self::Set { sheet, .. }
            | Self::Insert { sheet, .. }
            | Self::Remove { sheet, .. }
            | Self::Format { sheet, .. } => *sheet,
        }
    }
}

/// A sheet artifact's content from its sheets' rows: the first as `rows`,
/// as a one-sheet artifact gives it, and all of them under `sheets`.
pub fn sheet_content(sheets: Vec<serde_json::Value>) -> String {
    let first = sheets.first().map(|sheet| sheet["rows"].clone()).unwrap_or_else(|| serde_json::json!([]));
    if sheets.len() > 1 {
        serde_json::json!({ "rows": first, "sheets": sheets }).to_string()
    } else {
        serde_json::json!({ "rows": first }).to_string()
    }
}

/// The cells each sheet of a new sheet artifact starts with, by address,
/// from its content: `{"sheets": [{"rows": [...]}, ...]}`, or one sheet's
/// `{"rows": [...]}`. Each has a grid to type in, even empty, and there are
/// at most [`SHEETS`].
pub fn seeded_sheets(content: &str) -> Vec<BTreeMap<String, String>> {
    let seeded: serde_json::Value = serde_json::from_str(content).unwrap_or_default();
    let sheets: Vec<serde_json::Value> = match seeded["sheets"].as_array() {
        Some(sheets) if !sheets.is_empty() => sheets.iter().take(usize::from(SHEETS)).cloned().collect(),
        _ => vec![serde_json::json!({ "rows": seeded["rows"] })],
    };
    sheets
        .iter()
        .map(|sheet| {
            let rows: Vec<Vec<serde_json::Value>> = serde_json::from_value(sheet["rows"].clone()).unwrap_or_default();
            let mut cells = BTreeMap::new();
            for (row, values) in rows.iter().enumerate() {
                for (column, value) in values.iter().enumerate() {
                    let raw = match value {
                        serde_json::Value::String(text) => text.clone(),
                        serde_json::Value::Null => continue,
                        other => other.to_string(),
                    };
                    if !raw.is_empty() {
                        cells.insert(silt_sheet::address(row, column), raw);
                    }
                }
            }
            cells.entry(silt_sheet::address(9, 4)).or_default();
            cells
        })
        .collect()
}

fn id_text(id: &silt_crdt::CharId) -> String {
    match id {
        silt_crdt::CharId::Delta(delta) => delta.to_string(),
        silt_crdt::CharId::Base { source, offset } => format!("{source}+{offset}"),
    }
}

/// The sheet's view, values computed as of `today`, a date serial.
pub fn sheet_view(cells: &SheetCells, today: f64) -> SheetView {
    book_view(&[cells], 0, today)
}

/// Each sheet's row and column ids, as text, for formulas kept by them.
fn axes_of(book: &[&SheetCells]) -> Vec<(Vec<String>, Vec<String>)> {
    book.iter().map(|cells| (cells.rows.iter().map(id_text).collect(), cells.columns.iter().map(id_text).collect())).collect()
}

/// Sheet `sheet` of `book`, every sheet of an artifact, as its viewer shows
/// it: formulas may name another sheet's cells, `Sheet2!B3` (ADR 0077).
pub fn book_view(book: &[&SheetCells], sheet: usize, today: f64) -> SheetView {
    book_view_in(book, &[], sheet, today)
}

/// [`book_view`] with each sheet's settings, `metas`: its name, and its
/// columns' widths and number formats (ADR 0083).
pub fn book_view_in(book: &[&SheetCells], metas: &[SheetMeta], sheet: usize, today: f64) -> SheetView {
    let axes = axes_of(book);
    let borrowed: Vec<(&[String], &[String])> =
        axes.iter().map(|(rows, columns)| (rows.as_slice(), columns.as_slice())).collect();
    let raws: Vec<Vec<Vec<String>>> = book
        .iter()
        .enumerate()
        .map(|(at, cells)| {
            let positions = silt_sheet::Positions::in_book(&borrowed, at);
            cells
                .rows
                .iter()
                .map(|row| {
                    cells
                        .columns
                        .iter()
                        .map(|column| cells.cells.get(&(*row, *column)).map_or_else(String::new, |raw| positions.from_ids(raw)))
                        .collect()
                })
                .collect()
        })
        .collect();
    let sheets: Vec<&[Vec<String>]> = raws.iter().map(Vec::as_slice).collect();
    let (rows, columns) = axes.get(sheet).cloned().unwrap_or_default();
    let meta = metas.get(sheet).cloned().unwrap_or_default();
    let formats: Vec<Option<String>> = columns.iter().map(|column| meta.formats.get(column).cloned()).collect();
    let values = silt_sheet::evaluate_in(&silt_sheet::Book { sheets: &sheets, today }, sheet)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(column, value)| match (&value, formats.get(column).and_then(Option::as_deref)) {
                    (silt_sheet::Value::Number(number), Some(format)) => silt_sheet::format_number(*number, format),
                    _ => value.to_string(),
                })
                .collect()
        })
        .collect();
    let raw = raws.get(sheet).cloned().unwrap_or_default();
    let names = (0..book.len().max(1))
        .map(|at| metas.get(at).and_then(|meta| meta.name.clone()).unwrap_or_else(|| format!("Sheet {}", at + 1)))
        .collect();
    let widths = columns.iter().map(|column| meta.widths.get(column).copied()).collect();
    SheetView { rows, columns, raw, values, sheets: u16::try_from(book.len()).unwrap_or(SHEETS).max(1), names, widths, formats }
}

/// What a `Set` needs first: how many rows and columns to add at the end so
/// every address it names exists.
pub fn sheet_growth(cells: &SheetCells, set: &BTreeMap<String, String>) -> Result<(usize, usize), String> {
    if set.len() > SET_CELLS {
        return Err(format!("a request sets at most {SET_CELLS} cells"));
    }
    let mut needed = (0, 0);
    for (address, raw) in set {
        let (row, column) = silt_sheet::parse_address(address).ok_or_else(|| format!("{address} isn't a cell's address"))?;
        // The cells a formula names exist too, so it keeps meaning them
        // rather than becoming #REF! (spreadsheets count rows added later).
        for (row, column) in std::iter::once((row, column)).chain(silt_sheet::addresses_in(raw)) {
            if row >= silt_sheet::ROWS || column >= silt_sheet::COLUMNS {
                return Err(format!(
                    "{address} names a cell past a sheet's {} rows or {} columns",
                    silt_sheet::ROWS,
                    silt_sheet::COLUMNS
                ));
            }
            needed = (needed.0.max(row + 1), needed.1.max(column + 1));
        }
    }
    let (rows, columns) = (needed.0.max(cells.rows.len()), needed.1.max(cells.columns.len()));
    if rows.saturating_mul(columns) > SHEET_CELLS {
        return Err(format!("a sheet holds at most {SHEET_CELLS} cells; that would make it {rows} by {columns}"));
    }
    Ok((needed.0.saturating_sub(cells.rows.len()), needed.1.saturating_sub(cells.columns.len())))
}

/// The most cells a sheet's grid holds, rows times columns (ADR 0077).
pub const SHEET_CELLS: usize = silt_log::page::SHEET_CELLS;

/// The cell writes a `Set` makes once its rows and columns exist, each
/// formula kept with its references by ids.
pub fn cell_ops(
    artifact: ArtifactId,
    sheet: u16,
    cells: &SheetCells,
    set: &BTreeMap<String, String>,
) -> Result<Vec<PageOp>, String> {
    cell_ops_in(artifact, sheet, &[cells], 0, set, &BTreeMap::new())
}

/// [`cell_ops`] for sheet `sheet` with `book`, every sheet of the artifact,
/// at `at` in it: a formula naming another sheet keeps that sheet's ids.
pub fn cell_ops_in(
    artifact: ArtifactId,
    sheet: u16,
    book: &[&SheetCells],
    at: usize,
    set: &BTreeMap<String, String>,
    ids: &BTreeMap<String, (String, String)>,
) -> Result<Vec<PageOp>, String> {
    let Some(cells) = book.get(at) else { return Err("no such sheet".into()) };
    let axes = axes_of(book);
    let borrowed: Vec<(&[String], &[String])> =
        axes.iter().map(|(rows, columns)| (rows.as_slice(), columns.as_slice())).collect();
    set.iter()
        .map(|(address, raw)| {
            let (row, column) = match ids.get(address) {
                // The cell the person saw, wherever it stands now.
                Some((row_id, column_id)) => {
                    let row = cells.rows.iter().position(|id| id_text(id) == *row_id);
                    let column = cells.columns.iter().position(|id| id_text(id) == *column_id);
                    let (Some(row), Some(column)) = (row, column) else {
                        return Err(format!("conflict: {address}'s row or column was taken out"));
                    };
                    (row, column)
                }
                None => silt_sheet::parse_address(address).ok_or_else(|| format!("{address} isn't a cell's address"))?,
            };
            let (Some(row), Some(column)) = (cells.rows.get(row), cells.columns.get(column)) else {
                return Err(format!("{address} isn't in the sheet"));
            };
            if raw.len() > silt_log::page::CELL_BYTES {
                return Err(format!("{address} is over {} KiB", silt_log::page::CELL_BYTES >> 10));
            }
            Ok(PageOp::Cell { artifact, sheet, row: *row, column: *column, raw: silt_sheet::to_ids_in(raw, &borrowed, at) })
        })
        .collect()
}

/// The values in `range` of `view`, such as `A1:D20`, by row.
pub fn read_range(view: &SheetView, range: &str) -> Result<Vec<Vec<String>>, String> {
    let (start, end) = range.split_once(':').unwrap_or((range, range));
    let (Some((top, left)), Some((bottom, right))) = (silt_sheet::parse_address(start), silt_sheet::parse_address(end)) else {
        return Err(format!("{range} isn't a range"));
    };
    // Only what the sheet holds: past it every cell is empty.
    let rows = view.values.len();
    let columns = view.columns.len();
    let (top, bottom) = (top.min(bottom), top.max(bottom).min(rows.saturating_sub(1)));
    let (left, right) = (left.min(right), left.max(right).min(columns.saturating_sub(1)));
    if rows == 0 || columns == 0 || top > bottom || left > right {
        return Ok(Vec::new());
    }
    Ok((top..=bottom)
        .map(|row| {
            (left..=right)
                .map(|column| view.values.get(row).and_then(|cells| cells.get(column)).cloned().unwrap_or_default())
                .collect()
        })
        .collect())
}

/// A sheet as CSV: what each cell shows. A value a spreadsheet app would run
/// as a formula on opening starts with `'`, as OWASP advises.
pub fn csv(view: &SheetView) -> String {
    let cell = |text: &str| {
        let runs = text.starts_with(['=', '+', '-', '@', '\t', '\r']) && text.trim().parse::<f64>().is_err();
        let text = if runs { format!("'{text}") } else { text.to_owned() };
        if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text }
    };
    view.values.iter().map(|row| row.iter().map(|value| cell(value)).collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join("\n")
}

/// A sheet as an `.xlsx` workbook, formulas kept as formulas with their
/// values cached, so it opens alike in a spreadsheet app.
#[cfg(feature = "xlsx")]
pub fn xlsx(view: &SheetView, name: &str) -> Result<Vec<u8>, String> {
    xlsx_book(&[(name.to_owned(), view.clone())])
}

/// A workbook of `sheets`, each a worksheet by its name, in order.
#[cfg(feature = "xlsx")]
pub fn xlsx_book(sheets: &[(String, SheetView)]) -> Result<Vec<u8>, String> {
    use rust_xlsxwriter::Workbook;
    let titles = xlsx_titles(sheets.iter().map(|(name, _)| name.as_str()));
    let mut book = Workbook::new();
    for ((_, view), title) in sheets.iter().zip(&titles) {
        let sheet = book.add_worksheet();
        sheet.set_name(title).map_err(|error| error.to_string())?;
        write_sheet(sheet, view, &titles)?;
    }
    book.save_to_buffer().map_err(|error| error.to_string())
}

/// Worksheet names from sheets' names: without what a workbook refuses, at
/// most 31 characters, and each its own.
#[cfg(feature = "xlsx")]
fn xlsx_titles<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut titles: Vec<String> = Vec::new();
    for (at, name) in names.enumerate() {
        let cleaned: String = name.chars().filter(|c| !"[]:*?/\\'".contains(*c)).take(28).collect();
        let base = if cleaned.trim().is_empty() { format!("Sheet{}", at + 1) } else { cleaned.trim().to_owned() };
        let mut title = base.clone();
        let mut again = 2;
        while titles.iter().any(|taken| taken.eq_ignore_ascii_case(&title)) {
            title = format!("{base} {again}");
            again += 1;
        }
        titles.push(title);
    }
    titles
}

/// `raw` with each `SheetN!` outside quotes naming that sheet's worksheet,
/// as Silt numbers sheets in formulas and a workbook names them.
#[cfg(feature = "xlsx")]
fn named_sheets(raw: &str, titles: &[String]) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut quoted = false;
    let mut rest = raw;
    while let Some(c) = rest.chars().next() {
        if c == '"' {
            quoted = !quoted;
        }
        let before = out.chars().last();
        if !quoted
            && before.is_none_or(|before| !before.is_ascii_alphanumeric() && before != '_')
            && rest.get(..5).is_some_and(|head| head.eq_ignore_ascii_case("sheet"))
        {
            let digits: String = rest[5..].chars().take_while(char::is_ascii_digit).collect();
            if !digits.is_empty()
                && rest[5 + digits.len()..].starts_with('!')
                && let Some(title) = digits.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|n| titles.get(n))
            {
                out.push_str(&format!("'{title}'!"));
                rest = &rest[5 + digits.len() + 1..];
                continue;
            }
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(feature = "xlsx")]
fn write_sheet(sheet: &mut rust_xlsxwriter::Worksheet, view: &SheetView, titles: &[String]) -> Result<(), String> {
    use rust_xlsxwriter::{Format, Formula};
    let formats: Vec<Option<Format>> =
        view.formats.iter().map(|format| format.as_deref().map(|format| Format::new().set_num_format(format))).collect();
    for (column, width) in view.widths.iter().enumerate() {
        if let (Some(width), Ok(column)) = (width, u16::try_from(column)) {
            sheet.set_column_width_pixels(column, u32::from(*width)).map_err(|error| error.to_string())?;
        }
    }
    for (row, cells) in view.raw.iter().enumerate() {
        for (column, raw) in cells.iter().enumerate() {
            let (r, c) =
                (u32::try_from(row).map_err(|_| "too many rows")?, u16::try_from(column).map_err(|_| "too many columns")?);
            let value = view.values.get(row).and_then(|values| values.get(column)).cloned().unwrap_or_default();
            let format = formats.get(column).and_then(Option::as_ref);
            // Only a formula Silt computes is kept live, so a workbook never
            // runs what a peer wrote, such as WEBSERVICE or a DDE call.
            let written = if raw.starts_with('=') && silt_sheet::formula_ok(raw) {
                let formula = Formula::new(named_sheets(raw, titles)).set_result(value);
                match format {
                    Some(format) => sheet.write_formula_with_format(r, c, formula, format),
                    None => sheet.write_formula(r, c, formula),
                }
                .map(|_| ())
            } else if raw.is_empty() {
                Ok(())
            } else if let Ok(number) = raw.trim().parse::<f64>() {
                match format {
                    Some(format) => sheet.write_number_with_format(r, c, number, format),
                    None => sheet.write_number(r, c, number),
                }
                .map(|_| ())
            } else {
                sheet.write_string(r, c, raw.as_str()).map(|_| ())
            };
            written.map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// A comment's target on characters `start..end` of a doc as it stands:
/// anchored before the first and after the last, so it follows its text
/// through edits, as a comment on code does (ADR 0066).
pub fn doc_comment_target(
    doc: &silt_crdt::TextDoc,
    artifact: ArtifactId,
    start: usize,
    end: usize,
) -> Result<silt_log::CommentTarget, String> {
    use silt_crdt::Bias;
    if start >= end {
        return Err("choose some of the doc's text to comment on".into());
    }
    let anchor = |pos, bias| doc.anchor(pos, bias).map_err(|_| format!("the doc has {} characters", doc.len()));
    Ok(silt_log::CommentTarget::Page { artifact, start: anchor(start, Bias::Right)?, end: anchor(end, Bias::Left)? })
}

/// Where a comment on a doc's text is now.
pub fn doc_comment_location(
    doc: &silt_crdt::TextDoc,
    artifact: ArtifactId,
    start: &silt_crdt::Anchor,
    end: &silt_crdt::Anchor,
) -> crate::view::CommentLocation {
    let (Ok(from), Ok(to)) = (doc.resolve(start), doc.resolve(end)) else {
        return crate::view::CommentLocation::Unknown;
    };
    let (start, end) = (from.pos, to.pos.max(from.pos));
    let excerpt = doc.text().chars().skip(start).take((end - start).min(120)).collect();
    crate::view::CommentLocation::Doc {
        artifact: artifact.to_string(),
        start,
        end,
        excerpt,
        outdated: (from.outdated && to.outdated) || start == end,
    }
}

/// How many words a doc's or a sheet's search hit shows, as the index's
/// snippets do.
const SNIPPET_WORDS: usize = 12;

/// A doc's or a sheet's search hit: when every word of `query` begins a
/// word of `text`, as the index matches them, a snippet around the first
/// match with the matches in brackets.
pub fn text_hit(text: &str, query: &str) -> Option<String> {
    let terms: Vec<String> =
        query.split_whitespace().map(|word| word.replace('"', "").to_lowercase()).filter(|word| !word.is_empty()).collect();
    if terms.is_empty() {
        return None;
    }
    let words = words_of(text);
    let lowered: Vec<String> = words.iter().map(|&(start, end)| text[start..end].to_lowercase()).collect();
    if !terms.iter().all(|term| lowered.iter().any(|word| word.starts_with(term.as_str()))) {
        return None;
    }
    let matched = |word: &str| terms.iter().any(|term| word.starts_with(term.as_str()));
    let first = lowered.iter().position(|word| matched(word))?;
    let from = first.saturating_sub(SNIPPET_WORDS / 3);
    let to = (from + SNIPPET_WORDS).min(words.len());
    let mut snippet = String::new();
    if from > 0 {
        snippet.push('…');
    }
    for at in from..to {
        let (start, end) = words[at];
        if at > from {
            snippet.push_str(&text[words[at - 1].1..start].split_whitespace().collect::<Vec<_>>().join(" "));
            if text[words[at - 1].1..start].ends_with(char::is_whitespace) {
                snippet.push(' ');
            }
        }
        if matched(&lowered[at]) {
            snippet.push('[');
            snippet.push_str(&text[start..end]);
            snippet.push(']');
        } else {
            snippet.push_str(&text[start..end]);
        }
    }
    if to < words.len() {
        snippet.push('…');
    } else if let Some(&(_, end)) = words.last() {
        snippet.push_str(text[end..].trim());
    }
    Some(snippet)
}

/// The byte ranges of `text`'s words: runs of letters and digits.
fn words_of(text: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut start = None;
    for (at, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                words.push((from, at));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        words.push((from, text.len()));
    }
    words
}

/// What a sheet says, for search: its cells as typed, row by row.
pub fn sheet_text(cells: &SheetCells) -> String {
    let mut text = String::new();
    for row in &cells.rows {
        for column in &cells.columns {
            if let Some(raw) = cells.cells.get(&(*row, *column)).filter(|raw| !raw.starts_with('=')) {
                text.push_str(raw);
                text.push('\t');
            }
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text with an id for each character, `from` on.
    fn ids(text: &str, from: u64) -> Vec<(char, silt_crdt::CharId)> {
        let actor = silt_types::ActorId(7);
        text.chars()
            .zip(from..)
            .map(|(char, at)| (char, silt_crdt::CharId::Delta(silt_types::DeltaId::new(actor, silt_types::Lamport(at)))))
            .collect()
    }

    fn applied(now: &[(char, silt_crdt::CharId)], edits: &[DocEdit]) -> String {
        let text: String = now.iter().map(|(char, _)| char).collect();
        let (_, chars) = edited(&text, edits).expect("applies");
        chars.into_iter().collect()
    }

    #[test]
    fn a_sheet_starts_with_each_sheet_it_was_given() {
        let one = seeded_sheets(r#"{"rows": [["Task", "Hours"], ["Design", 3]]}"#);
        assert_eq!(one.len(), 1);
        assert_eq!((one[0].get("A1").map(String::as_str), one[0].get("B2").map(String::as_str)), (Some("Task"), Some("3")));
        let two = seeded_sheets(r#"{"sheets": [{"rows": [["a"]]}, {"rows": [["b", null, "c"]]}]}"#);
        assert_eq!(two.len(), 2);
        assert_eq!(two[1].get("C1").map(String::as_str), Some("c"));
        assert!(two.iter().all(|cells| cells.contains_key("E10")), "a grid to type in");
        let many: Vec<_> = (0..40).map(|_| serde_json::json!({"rows": [["x"]]})).collect();
        assert_eq!(seeded_sheets(&serde_json::json!({"sheets": many}).to_string()).len(), usize::from(SHEETS));
        let content: serde_json::Value = serde_json::from_str(&sheet_content(vec![
            serde_json::json!({"rows": [["a"]]}),
            serde_json::json!({"rows": [["b"]]}),
        ]))
        .expect("json");
        assert_eq!((&content["rows"][0][0], &content["sheets"][1]["rows"][0][0]), (&"a".into(), &"b".into()));
    }

    #[test]
    fn an_edit_counted_against_an_older_read_lands_where_it_was_meant() {
        let seen = ids("# Plan\n- ship\n", 0);
        // The agent put a line at the top since.
        let mut now = ids("Goals\n", 100);
        now.extend(seen.iter().copied());
        let append = [DocEdit::Splice { pos: seen.len(), delete: 0, insert: "- demo\n".into() }];
        assert_eq!(applied(&now, &rebased(&seen, &now, &append).expect("rebased")), "Goals\n# Plan\n- ship\n- demo\n");
        // Deleting "ship" deletes only it, though the agent typed inside it.
        let mut split = seen.clone();
        split.splice(10..10, ids("XX", 200));
        let delete = [DocEdit::Splice { pos: 9, delete: 4, insert: "go".into() }];
        assert_eq!(applied(&split, &rebased(&seen, &split, &delete).expect("rebased")), "# Plan\n- goXX\n");
        // What the agent deleted stays deleted; the person's text goes after what's left before it.
        let mut cut = seen.clone();
        cut.drain(7..9);
        let typed = [DocEdit::Splice { pos: 9, delete: 0, insert: "now ".into() }];
        assert_eq!(applied(&cut, &rebased(&seen, &cut, &typed).expect("rebased")), "# Plan\nnow ship\n");
    }

    #[test]
    fn the_log_folds_a_sheet_within_what_the_evaluator_computes() {
        assert_eq!((silt_log::page::SHEET_ROWS, silt_log::page::SHEET_COLUMNS), (silt_sheet::ROWS, silt_sheet::COLUMNS));
    }

    #[test]
    fn a_formula_naming_cells_past_the_sheet_grows_it_and_a_range_reads_only_the_sheet() {
        let cells = SheetCells::default();
        let set = BTreeMap::from([("A1".to_owned(), "=SUM(B2:B100)".to_owned())]);
        assert_eq!(sheet_growth(&cells, &set), Ok((100, 2)));
        let huge = BTreeMap::from([("A1".to_owned(), "=SUM(A1:GR10000)".to_owned())]);
        assert!(sheet_growth(&cells, &huge).is_err(), "past the cells a sheet holds");
        let view = SheetView {
            rows: vec!["r".into()],
            columns: vec!["c".into()],
            raw: vec![vec!["1".into()]],
            values: vec![vec!["1".into()]],
            sheets: 1,
            ..Default::default()
        };
        assert_eq!(read_range(&view, "A1:ZZZ99999999"), Ok(vec![vec!["1".to_owned()]]));
    }

    #[test]
    fn a_sheet_is_named_and_its_columns_sized_and_formatted() {
        let artifact: ArtifactId = "0123456789ab".parse().expect("an id");
        let id = |n: u64| silt_crdt::CharId::Delta(silt_types::DeltaId::new(silt_types::ActorId(1), silt_types::Lamport(n)));
        let cells = SheetCells {
            rows: vec![id(1), id(2)],
            columns: vec![id(3), id(4)],
            cells: BTreeMap::from([((id(1), id(4)), "0.25".to_owned())]),
        };
        let op = format_op(
            artifact,
            0,
            &cells,
            Some("Budget"),
            &BTreeMap::from([("B".to_owned(), Some(120))]),
            &BTreeMap::from([("B".to_owned(), Some("0%".to_owned()))]),
        )
        .expect("an op");
        let PageOp::Patched { path, fields, .. } = &op else { panic!("{op:?}") };
        assert_eq!(path, "_sheets/s0");
        let doc: serde_json::Value = serde_json::from_str(fields).expect("json");
        let meta = sheet_meta(Some(&doc));
        assert_eq!(meta.name.as_deref(), Some("Budget"));
        let view = book_view_in(&[&cells], &[meta], 0, 0.0);
        assert_eq!(view.values[0][1], "25%");
        assert_eq!((view.widths.clone(), view.names.clone()), (vec![None, Some(120)], vec!["Budget".to_owned()]));
        assert!(
            format_op(artifact, 0, &cells, None, &BTreeMap::new(), &BTreeMap::from([("Z".to_owned(), Some("0".to_owned()))]))
                .is_err()
        );
        assert!(
            format_op(artifact, 0, &cells, None, &BTreeMap::new(), &BTreeMap::from([("A".to_owned(), Some("yyyy".to_owned()))]))
                .is_err()
        );
        let peer = serde_json::json!({"name": "\u{7}", "w:x": 5000, "f:y": "=HYPERLINK()"});
        assert_eq!(sheet_meta(Some(&peer)), SheetMeta::default(), "what a peer wrote that a sheet doesn't take is left out");
    }

    #[test]
    fn exports_never_run_what_a_cell_holds() {
        let view = SheetView {
            rows: vec!["r".into()],
            columns: vec!["a".into(), "b".into(), "c".into()],
            raw: vec![vec!["=cmd|' /C calc'!A0".into(), "-5".into(), "@SUM(1)".into()]],
            values: vec![vec!["=cmd|' /C calc'!A0".into(), "-5".into(), "@SUM(1)".into()]],
            sheets: 1,
            ..Default::default()
        };
        assert_eq!(csv(&view), "'=cmd|' /C calc'!A0,-5,'@SUM(1)");
        assert!(silt_sheet::formula_ok("=SUM(A1:B2)*2") && !silt_sheet::formula_ok("=WEBSERVICE(\"https://x\")"));
    }
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
            sheets: 1,
            ..Default::default()
        };
        let bytes = xlsx(&view, "Tracker").expect("a workbook");
        assert_eq!(
            named_sheets("=Sheet2!A1+SHEET1!B2&\"Sheet2!\"", &["Tracker".into(), "It's".into()]),
            "='It's'!A1+'Tracker'!B2&\"Sheet2!\""
        );
        assert_eq!(xlsx_titles(["Q1", "q1", "", "a/b"].into_iter()), ["Q1", "q1 2", "Sheet3", "ab"]);
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

    #[test]
    fn a_text_hit_needs_every_word_and_brackets_each_match() {
        let text = "Launch plan\n\nWe ship the beta on Friday, after the review of the pricing page.";
        assert_eq!(text_hit(text, "pric beta").as_deref(), Some("…plan We ship the [beta] on Friday, after the review of the…"));
        assert_eq!(text_hit(text, "beta tuesday"), None);
        assert_eq!(text_hit(text, "  "), None);
        assert_eq!(text_hit(text, "PRICING").as_deref(), Some("…the review of the [pricing] page."));
    }
}
