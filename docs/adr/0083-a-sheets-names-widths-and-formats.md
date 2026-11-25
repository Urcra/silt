# 0083: A sheet's names, column widths and number formats

- Status: accepted
- Date: 2026-11-16
- Amends: [ADR 0077](0077-docs-and-sheets-edited-together.md) (sheets)

## Context
A sheet artifact's cells, rows and columns are page ops folded along the thread (ADR 0077). Its sheets were "Sheet 1" to "Sheet 16", every column was as wide as its text, and a share showed as `0.25` unless a formula wrapped it in `TEXT`, which turns the number into text that other formulas can't add up.

## Decision
**One settings document per sheet.** Sheet `n`'s name, and its columns' widths and number formats, are fields of the page document `_sheets/s<n>`: `name`, `w:<column id>` and `f:<column id>`. They're written as `PageOp::Patched`, so two people changing different columns at once both keep their change, and they follow the column by its id when columns go in or out. No new op and no new wire format: they fold, fork, rewind and sync as page data does.

**Written only through the sheet.** `SheetRequest::Format` writes them, from the web view and `silt_artifact_sheet`'s `format`, at the sheet's edit level. A page's `db` can't, as paths starting `_` are Silt's, and the server checks a pushed `_sheets/` patch at the sheet's edit level, as it checks a cell. The fold keeps only what a sheet takes: a name of 1 to 64 characters, widths of 24 to 800 pixels, and the number formats below.

**Number formats Silt computes.** `0`, `0.0` and more decimals, `#,##0` with decimals, `0%` with decimals, any of those after `$`, `€`, `£` or `¥`, and for date serials `yyyy-mm-dd`, `d mmm yyyy`, `mmm yyyy`, `dd/mm/yyyy` and `mm/dd/yyyy`. A cell keeps its number; the view shows it formatted, and formulas read the number. The `.xlsx` carries the same format strings, the widths and the names.

**Formulas name sheets by number.** `Sheet2!B3` keeps meaning the second sheet whatever it's called, so renaming one never breaks a formula. The `.xlsx` writes such references by the worksheet's name.

## Consequences
- A tab shows its name and renames on a double-click; a column's header edge resizes it; a menu sets the selected column's format.
- A saved version keeps the cells, not the settings, as before.
- **Not done:** per-cell formats, and references to a sheet by its name.
