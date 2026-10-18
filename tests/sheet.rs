//! Headless tests of the spreadsheet viewer (C-013): a real gpui-kit window opens a csv as a grid with its
//! header, sorts by a column, finds cells, edits a cell back into its record, toggles to the raw text, and
//! opens an Excel workbook on its sheets; the workbook fixtures are read without a window too.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::component::table::ColumnSort;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use sfoglia::generated::families::Family;
use sfoglia::sheet::{self, CellKind, Merge};
use sfoglia::text::Encoding;
use sfoglia::viewers::SheetView;
use sfoglia::workspace::FilePanel;
use sfoglia::{Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-sheet-rules-{}-{n}.toml", std::process::id()));
    cx.update(gpui_kit::init);
    cx.update(sfoglia::init);
    cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |window, cx| {
            cx.new(|cx| Workspace::with_rules_path(Some(rules_path.clone()), window, cx))
        })
        .expect("open the main window headlessly")
    })
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap()
}

/// A fresh directory for this test's files, and a copy of a fixture in it: the tests write.
fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sfoglia-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_fixture(name: &str, dir: &std::path::Path) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, fixture_bytes(name)).unwrap();
    path
}

fn label(window: &gpui_kit::Window, id: impl Into<gpui_kit::ElementId>) -> String {
    window.find(id).label().unwrap_or_default().to_string()
}

/// Open `path` in a fresh workspace and hand back its panel and spreadsheet view.
fn open_sheet(cx: &mut TestAppContext, path: PathBuf) -> (AnyWindowHandle, Entity<FilePanel>, Entity<SheetView>) {
    let (handle, workspace) = open_workspace(cx);
    let (panel, view) = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[path], window, cx));
            window.render_frame(cx);
            let panel = workspace.read(cx).panels(cx)[0].clone();
            let view = panel.read(cx).sheet_view().expect("the file opens in the spreadsheet viewer").clone();
            (panel, view)
        })
        .unwrap();
    cx.run_until_parked();
    (handle, panel, view)
}

/// The grid's column names and cells as the table dumps them.
fn dump(view: &Entity<SheetView>, cx: &gpui_kit::App) -> (Vec<String>, Vec<Vec<String>>) {
    view.read(cx).table().read(cx).dump(cx)
}

fn rows(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| c.to_string()).collect()
}

#[gpui_kit::test]
fn the_spreadsheet_extensions_map_to_their_family(_: &mut TestAppContext) {
    let sheets: Vec<&str> = sfoglia::generated::families::EXTENSIONS
        .iter()
        .filter(|(_, families)| families.contains(&Family::Spreadsheets))
        .map(|(ext, _)| *ext)
        .collect();
    assert_eq!(sheets, vec!["csv", "tsv", "xls", "xlsm", "xlsx", "xlt", "xltm", "xltx"]);
    for (ext, families) in sfoglia::generated::families::EXTENSIONS {
        if families.contains(&Family::Spreadsheets) {
            assert_eq!(*families, &[Family::Spreadsheets], "{ext} belongs to the spreadsheets alone");
        }
    }
}

/// The delimited fixtures parse to the same fields, each record with its exact byte span.
#[test]
fn the_delimited_fixtures_parse_to_the_same_fields() {
    let expected = vec![
        rows(&["id", "name", "note", "amount", "when"]),
        rows(&["1", "Rossi, Mario", "first line\nsecond line", "7", "2026-01-05"]),
        rows(&["2", "Élodie", "plain", "12.50", "2026-02-28"]),
        rows(&["3", "quote \"inside\"", "", "-3.25", ""]),
    ];
    let csv = sfoglia::text::decode(&fixture_bytes("table.csv")).text;
    let grid = sheet::parse_delimited(&csv, sheet::detect_delimiter(&csv, "table.csv"));
    assert_eq!(grid.rows, expected);
    assert_eq!(grid.columns, 5);
    assert_eq!(grid.delimiter, b',');
    assert!(!grid.crlf);
    let spans: Vec<&str> = grid.spans.iter().map(|s| &csv[s.clone()]).collect();
    assert_eq!(spans[1], "1,\"Rossi, Mario\",\"first line\nsecond line\",7,2026-01-05");
    assert_eq!(spans[3], "3,\"quote \"\"inside\"\"\",,-3.25,");
    let tsv = sfoglia::text::decode(&fixture_bytes("table.tsv")).text;
    let grid = sheet::parse_delimited(&tsv, sheet::detect_delimiter(&tsv, "table.tsv"));
    assert_eq!(grid.rows, expected);
    assert_eq!(grid.delimiter, b'\t');
    let decoded = sfoglia::text::decode(&fixture_bytes("utf16.csv"));
    assert_eq!(decoded.encoding.label(), "UTF-16 LE");
    let grid = sheet::parse_delimited(&decoded.text, sheet::detect_delimiter(&decoded.text, "utf16.csv"));
    assert_eq!(grid.rows, expected);
}

/// The workbook fixtures read their typed cells: the generated xlsx and Excel's own 97-2003 save of it.
#[test]
fn the_workbook_fixtures_read_their_typed_cells() {
    for name in ["book.xlsx", "book.xls"] {
        let book = sheet::read_workbook(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let names: Vec<&str> = book.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Data", "Notes", "Empty"], "{name}");
        let data = &book.sheets[0];
        assert_eq!((data.rows.len(), data.columns), (7, 6), "{name}");
        let text = |r: usize, c: usize| data.cell(r, c).map(|cell| cell.text.clone()).unwrap_or_default();
        let texts: Vec<Vec<String>> = (0..7).map(|r| (0..6).map(|c| text(r, c)).collect()).collect();
        assert_eq!(texts[0], rows(&["id", "name", "amount", "when", "paid", "note"]), "{name}");
        assert_eq!(texts[1], rows(&["1", "Rossi, Mario", "12.5", "2026-01-05", "TRUE", "first line\nsecond line"]), "{name}");
        assert_eq!(texts[2], rows(&["2", "Élodie", "7", "2026-02-28 14:30:00", "FALSE", ""]), "{name}");
        assert_eq!(texts[3], rows(&["3", "quote \"inside\"", "-3.25", "", "", ""]), "{name}");
        let error = if name == "book.xlsx" { "#DIV/0!" } else { "" };
        assert_eq!(texts[4], rows(&["", "total", "16.25", "", "", error]), "{name}: the formula's cached value, an error as written");
        if name == "book.xlsx" {
            assert_eq!(data.cell(4, 5).map(|c| c.kind.clone()), Some(CellKind::Error));
        }
        assert_eq!(texts[5], rows(&["", "", "", "", "", ""]), "{name}");
        assert_eq!(texts[6], rows(&["merged across three", "", "", "", "", ""]), "{name}");
        assert_eq!(data.cell(1, 0).map(|c| c.kind.clone()), Some(CellKind::Number(1.0)), "{name}");
        assert_eq!(data.cell(1, 3).map(|c| c.kind.clone()), Some(CellKind::Date), "{name}");
        assert_eq!(data.cell(1, 4).map(|c| c.kind.clone()), Some(CellKind::Bool(true)), "{name}");
        if name == "book.xlsx" {
            assert_eq!(data.merges, vec![Merge { rows: 6..7, columns: 0..3 }]);
            assert!(data.covered_by(6, 1).is_some() && data.covered_by(6, 2).is_some() && data.covered_by(6, 0).is_none());
        }
        let notes = &book.sheets[1];
        assert_eq!((notes.rows.len(), notes.columns), (2, 2), "{name}");
        assert_eq!(notes.cell(0, 0).map(|c| c.text.as_str()), Some("a note"), "{name}");
        assert_eq!(notes.cell(1, 1).map(|c| c.text.as_str()), Some("42"), "{name}");
        assert_eq!(book.sheets[2].rows.len(), 0, "{name}");
    }
    // The same book saved by Excel with the 1904 date system: Excel keeps the serials, so every date shows
    // four years and a day later, as Excel itself shows them in that book.
    let book = sheet::read_workbook(&fixture("book1904.xls")).unwrap();
    let data = &book.sheets[0];
    assert_eq!(data.cell(1, 3).map(|c| c.text.as_str()), Some("2030-01-06"), "the 1904 epoch of a legacy xls");
    assert_eq!(data.cell(2, 3).map(|c| c.text.as_str()), Some("2030-03-01 14:30:00"));
    assert_eq!(data.cell(1, 1).map(|c| c.text.as_str()), Some("Rossi, Mario"));
    assert_eq!(data.cell(4, 2).map(|c| c.text.as_str()), Some("16.25"));
}

#[gpui_kit::test]
fn a_csv_opens_as_a_grid_with_its_summary_and_header_toggle(cx: &mut TestAppContext) {
    let (handle, _panel, view) = open_sheet(cx, fixture("table.csv"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 3 rows, 5 columns, comma, UTF-8");
        assert_eq!(label(window, "sheet-message"), "the arrow in a header sorts, Ctrl+Shift+F finds a cell, Ctrl+Shift+D shows the text");
        assert_eq!(label(window, "sheet-header"), "Header: on");
        let (names, cells) = dump(&view, cx);
        assert_eq!(names, rows(&["id", "name", "note", "amount", "when"]));
        assert_eq!(
            cells,
            vec![
                rows(&["1", "Rossi, Mario", "first line\nsecond line", "7", "2026-01-05"]),
                rows(&["2", "Élodie", "plain", "12.50", "2026-02-28"]),
                rows(&["3", "quote \"inside\"", "", "-3.25", ""]),
            ]
        );
        // A selected cell first: the toggle rebuilds the grid and drops it, so no stale record is named.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "B3");
        window.click("sheet-header-button", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "", "the toggle drops the selection");
        assert_eq!(view.read(cx).table().read(cx).selected_cell(), None);
        assert_eq!(label(window, "sheet-summary"), "CSV, 4 rows, 5 columns, comma, UTF-8");
        assert_eq!(label(window, "sheet-header"), "Header: off");
        let (names, cells) = dump(&view, cx);
        assert_eq!(names, rows(&["A", "B", "C", "D", "E"]));
        assert_eq!(cells[0], rows(&["id", "name", "note", "amount", "when"]));
        assert_eq!(cells.len(), 4);
        // With the header off, the first record is row 1.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(0, 0, cx));
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "A1");
    })
    .unwrap();
}

/// A `.csv` that holds tabs is read by tabs and labelled by its delimiter (C-013).
#[gpui_kit::test]
fn a_tab_delimited_csv_is_labelled_by_its_delimiter(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-tabs-csv");
    let path = dir.join("tabs.csv");
    std::fs::write(&path, "a\tb\n1\t2\n").unwrap();
    let (handle, _panel, view) = open_sheet(cx, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "TSV, 1 row, 2 columns, tab, UTF-8");
        assert_eq!(dump(&view, cx).1[0], rows(&["1", "2"]));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A ⏎ typed into a changed value goes back as a line break, in the record and in the saved bytes (C-013).
#[gpui_kit::test]
fn a_typed_break_glyph_goes_back_as_a_break(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-typed-break");
    let path = copy_fixture("table.csv", &dir);
    let (handle, _panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(0, 2, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("first line⏎second line!", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "C2 set; Ctrl+S saves");
        assert_eq!(dump(&view, cx).1[0][2], "first line\nsecond line!", "the glyph went back as a break");
        let text = view.read(cx).text_view().unwrap().read(cx).text(cx).to_string();
        assert!(text.contains("\"first line\nsecond line!\",7,"), "{text}");
        window.press("ctrl-s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let expected = String::from_utf8(fixture_bytes("table.csv")).unwrap().replace("second line\",7,", "second line!\",7,");
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_header_click_sorts_the_rows_and_a_third_restores_the_order(cx: &mut TestAppContext) {
    let (handle, _panel, view) = open_sheet(cx, fixture("table.csv"));
    let ids = |view: &Entity<SheetView>, cx: &gpui_kit::App| -> Vec<String> { dump(view, cx).1.iter().map(|r| r[0].clone()).collect() };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // A selected cell first, so the sort has something to drop.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "B3");
        // The amount column: 7, 12.50, -3.25 in the file.
        window.click(("icon-sort", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(ids(&view, cx), rows(&["2", "1", "3"]), "a click sorts descending");
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), Some((3, ColumnSort::Descending)));
        assert_eq!(label(window, "sheet-cell"), "", "a sort drops the selection, so no stale cell is named");
        window.click(("icon-sort", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(ids(&view, cx), rows(&["3", "1", "2"]), "a second click ascending");
        window.click(("icon-sort", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(ids(&view, cx), rows(&["1", "2", "3"]), "a third restores the file's order");
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), None);
        // Text sorts case-insensitively, empty cells last: the note column descending.
        window.click(("icon-sort", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(ids(&view, cx), rows(&["2", "1", "3"]), "plain, first line, then the empty note");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_search_field_finds_cells_and_counts_them(cx: &mut TestAppContext) {
    let (handle, _panel, view) = open_sheet(cx, fixture("table.csv"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        let focused = window.focused(cx);
        assert_eq!(focused, Some(view.read(cx).search_input().read(cx).focus_handle(cx)), "Ctrl+Shift+F focuses the search field");
        // Upper case, so a case-sensitive search would find nothing.
        window.input("LINE", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 1 of 1");
        assert_eq!(label(window, "sheet-cell"), "C2", "the note of the first record, named by the file's rows");
        assert_eq!(view.read(cx).table().read(cx).selected_cell(), Some((0, 2)));
        let input = view.read(cx).search_input().clone();
        input.update(cx, |input, cx| input.set_value("2026", window, cx));
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 1 of 2", "from the selected cell on, around the end");
        assert_eq!(label(window, "sheet-cell"), "E2");
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 2 of 2");
        assert_eq!(label(window, "sheet-cell"), "E3");
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 1 of 2", "around the end");
        assert_eq!(label(window, "sheet-cell"), "E2");
        let input = view.read(cx).search_input().clone();
        input.update(cx, |input, cx| input.set_value("zzz", window, cx));
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "no cell matches \"zzz\"");
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_edit_in_the_grid_rewrites_its_record_alone_and_saves(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-edit");
    let path = copy_fixture("table.csv", &dir);
    let (handle, panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = view.read(cx).table().clone();
        // C2 first: a two-line note shows its break as ⏎ in the one-line field, and Enter with nothing
        // changed writes nothing.
        table.update(cx, |table, cx| table.set_selected_cell(0, 2, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "C2");
        assert_eq!(view.read(cx).edit_input().read(cx).value().as_ref(), "first line⏎second line");
        let edit = view.read(cx).edit_input().clone();
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "C2 unchanged");
        assert!(!panel.read(cx).is_dirty(cx), "nothing written, the tab stays saved");
        let (_, cells) = dump(&view, cx);
        assert_eq!(cells[0][2], "first line\nsecond line", "the break is kept");
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "B3");
        assert_eq!(view.read(cx).edit_input().read(cx).value().as_ref(), "Élodie", "the edit field holds the cell");
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("Élodie Durand", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B3 set; Ctrl+S saves");
        let (_, cells) = dump(&view, cx);
        assert_eq!(cells[1][1], "Élodie Durand", "the grid follows the text");
        assert!(panel.read(cx).is_dirty(cx), "the tab is modified");
        let text = view.read(cx).text_view().expect("the text viewer holds the edit").read(cx).text(cx).to_string();
        assert_eq!(
            text,
            "id,name,note,amount,when\n1,\"Rossi, Mario\",\"first line\nsecond line\",7,2026-01-05\n2,Élodie Durand,plain,12.50,2026-02-28\n3,\"quote \"\"inside\"\"\",,-3.25,\n",
            "that record alone re-written, the others byte for byte"
        );
        window.press("ctrl-s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let expected = String::from_utf8(fixture_bytes("table.csv")).unwrap().replace("2,Élodie,plain", "2,Élodie Durand,plain");
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes(), "Ctrl+S writes the exact bytes");
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn the_raw_text_toggles_and_the_grid_follows_the_edit(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-raw");
    let path = copy_fixture("table.csv", &dir);
    let (handle, _panel, view) = open_sheet(cx, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        assert!(window.try_find("sheet-raw").is_some());
        assert_eq!(label(window, "sheet-message"), "Ctrl+Shift+D shows the grid");
        let text = view.read(cx).text_view().unwrap().clone();
        let focus = text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("X", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        let (names, _) = dump(&view, cx);
        assert_eq!(names[0], "Xid", "the grid is rebuilt from the text as edited");
        assert_eq!(label(window, "sheet-summary"), "CSV, 3 rows, 5 columns, comma, UTF-8");
        // Ctrl+Shift+F from the raw text comes back to the grid and focuses the field there.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown(), "the grid again");
        assert_eq!(window.focused(cx), Some(view.read(cx).search_input().read(cx).focus_handle(cx)));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_workbook_opens_on_its_sheets_with_merges_and_dates(cx: &mut TestAppContext) {
    let (handle, panel, view) = open_sheet(cx, fixture("book.xlsx"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "Excel, 3 sheets · Data: 7 rows, 6 columns");
        assert_eq!(label(window, "sheet-message"), "the arrow in a header sorts, Ctrl+Shift+F finds a cell");
        assert_eq!(view.read(cx).sheet_names(), rows(&["Data", "Notes", "Empty"]));
        assert!(window.try_find("sheet-header").is_none(), "a workbook has no header toggle");
        assert!(window.try_find("sheet-edit").is_none(), "nor an edit field");
        assert!(panel.read(cx).editable_text(cx).is_none(), "nor a text to save");
        let (names, cells) = dump(&view, cx);
        assert_eq!(names, rows(&["A", "B", "C", "D", "E", "F"]));
        assert_eq!(cells[1], rows(&["1", "Rossi, Mario", "12.5", "2026-01-05", "TRUE", "first line\nsecond line"]));
        assert_eq!(cells[2], rows(&["2", "Élodie", "7", "2026-02-28 14:30:00", "FALSE", ""]));
        assert_eq!(cells[4][2], "16.25", "the formula's cached value");
        assert_eq!(cells[6], rows(&["merged across three", "", "", "", "", ""]), "the merged value once, the covered cells blank");
        // Sorting, search and cell selection work on a workbook too: C's arrow (text first, then by value,
        // the empty cells last), then `Élodie` found at B3 (the sort dropped, the file's order again).
        window.click(("icon-sort", 2usize), cx);
        window.render_frame(cx);
        let amounts: Vec<String> = dump(&view, cx).1.iter().map(|r| r[2].clone()).collect();
        assert_eq!(amounts, rows(&["amount", "16.25", "12.5", "7", "-3.25", "", ""]));
        window.click(("icon-sort", 2usize), cx);
        window.render_frame(cx);
        window.click(("icon-sort", 2usize), cx);
        window.render_frame(cx);
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        window.input("ÉLODIE", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 1 of 1");
        assert_eq!(label(window, "sheet-cell"), "B3");
        window.click(("sheet-tab-button", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "", "a sheet switch drops the selection");
        assert_eq!(label(window, "sheet-summary"), "Excel, 3 sheets · Notes: 2 rows, 2 columns");
        assert_eq!(dump(&view, cx).1[1][1], "42");
        window.click(("sheet-tab-button", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "Excel, 3 sheets · Empty: 0 rows, 0 columns");
        assert!(dump(&view, cx).1.is_empty());
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        assert_eq!(label(window, "sheet-message"), "a workbook has no text to show; Ctrl+Shift+B shows its bytes");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_large_delimited_text_opens_on_its_text(cx: &mut TestAppContext) {
    // One fixed name, removed at the end: a leftover once filled the disk. A `.tsv`, so the label is checked.
    let big = std::env::temp_dir().join("sfoglia-sheet-big.tsv");
    {
        use std::io::Write as _;
        let mut file = std::io::BufWriter::new(std::fs::File::create(&big).unwrap());
        let line = format!("{},x\n", "a".repeat(1000));
        let mut written = 0usize;
        while written < 65 * 1024 * 1024 {
            file.write_all(line.as_bytes()).unwrap();
            written += line.len();
        }
    }
    let (handle, _panel, view) = open_sheet(cx, big.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "past the budget the text shows");
        assert!(label(window, "sheet-message").contains("past the grid's 64 MB"), "{}", label(window, "sheet-message"));
        assert_eq!(label(window, "sheet-summary"), "TSV, past the grid's 64 MB");
        // The toggle measures again: the text stays, with the message.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "still past the budget after the toggle");
        assert!(label(window, "sheet-message").contains("past the grid's 64 MB"));
    })
    .unwrap();
    std::fs::remove_file(&big).ok();
}

/// A file with CRLF endings: the grid's cells carry no carriage returns, an edit lands in its own record
/// before and after another edit, and Ctrl+S writes the file back with its endings (C-013).
#[gpui_kit::test]
fn an_edit_in_a_crlf_file_keeps_its_endings(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-crlf");
    let path = dir.join("table.csv");
    let crlf = String::from_utf8(fixture_bytes("table.csv")).unwrap().replace('\n', "\r\n");
    std::fs::write(&path, crlf.as_bytes()).unwrap();
    let (handle, panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (_, cells) = dump(&view, cx);
        assert_eq!(cells[0][2], "first line\nsecond line", "no carriage return in a cell");
        let table = view.read(cx).table().clone();
        // D2: the amount of the first record.
        table.update(cx, |table, cx| table.set_selected_cell(0, 3, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "D2");
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("9", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "D2 set; Ctrl+S saves");
        let (_, cells) = dump(&view, cx);
        assert_eq!(cells[0][3], "9");
        assert_eq!(cells[1][1], "Élodie", "the next record is untouched");
        // B3 next: the second edit lands in its own record too.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "B3");
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("Élodie Durand", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let expected_lf = String::from_utf8(fixture_bytes("table.csv"))
        .unwrap()
        .replace("second line\",7,", "second line\",9,")
        .replace("2,Élodie,plain", "2,Élodie Durand,plain");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (_, cells) = dump(&view, cx);
        assert_eq!(cells[1][1], "Élodie Durand");
        assert_eq!(cells[0][3], "9", "the first edit is still there");
        let text = view.read(cx).text_view().unwrap().read(cx).text(cx).to_string();
        assert_eq!(sfoglia::data::with_line_feeds(&text).as_ref(), expected_lf, "both records re-written, the rest as it was");
        assert!(panel.read(cx).is_dirty(cx));
        window.press("ctrl-s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), expected_lf.replace('\n', "\r\n").as_bytes(), "saved with the file's CRLF endings");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Enter on a value left as the field showed it writes nothing, whatever the record's own quoting: a literal ⏎
/// in a cell, a record quoted all over, and an empty cell past a ragged record's end (C-013).
#[gpui_kit::test]
fn an_unchanged_value_writes_nothing_whatever_the_quoting(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-unchanged");
    let path = dir.join("odd.csv");
    let source = "\"id\",\"n\"\n\"1\",\"x⏎y\"\n\"2\",\"7\"\n3\n";
    std::fs::write(&path, source).unwrap();
    let (handle, panel, view) = open_sheet(cx, path.clone());
    let press_enter = |window: &mut gpui_kit::Window, cx: &mut gpui_kit::App, view: &Entity<SheetView>| {
        let edit = view.read(cx).edit_input().clone();
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    };
    // B2: a literal ⏎ in the cell shows as itself and goes back untouched.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(0, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).edit_input().read(cx).value().as_ref(), "x⏎y");
        press_enter(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B2 unchanged");
        assert!(!panel.read(cx).is_dirty(cx));
        // B3: quoted all over in the file, shown as `7`, unchanged on Enter; `9` re-writes the record minimal.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).edit_input().read(cx).value().as_ref(), "7");
        press_enter(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B3 unchanged");
        assert!(!panel.read(cx).is_dirty(cx));
        // B4: past the ragged record's end, empty, unchanged on Enter.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(2, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).edit_input().read(cx).value().as_ref(), "");
        press_enter(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B4 unchanged");
        assert!(!panel.read(cx).is_dirty(cx), "no delimiter appended to the ragged record");
        assert!(view.read(cx).text_view().is_none(), "the text viewer was never needed");
        // A real change to B3 re-writes that record with minimal quoting, the others byte for byte.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("9", window, cx));
        press_enter(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B3 set; Ctrl+S saves");
        let text = view.read(cx).text_view().unwrap().read(cx).text(cx).to_string();
        assert_eq!(text, "\"id\",\"n\"\n\"1\",\"x⏎y\"\n2,9\n3\n", "that record alone, quoted only where needed");
        // A real value past the ragged record's end: the record grows to hold it.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(2, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("9", window, cx));
        press_enter(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B4 set; Ctrl+S saves");
        let text = view.read(cx).text_view().unwrap().read(cx).text(cx).to_string();
        assert_eq!(text, "\"id\",\"n\"\n\"1\",\"x⏎y\"\n2,9\n3,9\n", "the ragged record padded to the edited cell");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Another program changes the file: the bar shows on the grid side, Reload rebuilds the grid from the file
/// as read now (its encoding named afresh, the budget measured again), Keep mine marks the tab (C-013).
#[gpui_kit::test]
fn a_reload_follows_the_file_its_encoding_and_its_budget(cx: &mut TestAppContext) {
    // One fixed folder, cleared first and removed at the end: the file grows to 65 MB below, and a leftover
    // under a fresh name every run once filled the disk.
    let dir = std::env::temp_dir().join("sfoglia-sheet-reload");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("latin.csv");
    std::fs::write(&path, b"id,name\n1,caf\xe9\n").unwrap();
    let (handle, panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 1 row, 2 columns, comma, Latin-1");
        assert_eq!(dump(&view, cx).1[0][1], "café");
        assert!(window.try_find("sheet-changed-on-disk").is_none());
    })
    .unwrap();
    // Rewritten outside as UTF-8 with a third column: the bar comes up within two seconds.
    std::fs::write(&path, "id,name,more\n1,café,x\n".as_bytes()).unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_some(), "the bar shows on the grid side");
        window.click("sheet-reload", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 1 row, 3 columns, comma, UTF-8", "the file as read now");
        assert_eq!(label(window, "sheet-message"), "reloaded");
        assert_eq!(dump(&view, cx).1[0], rows(&["1", "café", "x"]));
        assert!(!panel.read(cx).is_dirty(cx));
        assert!(window.try_find("sheet-changed-on-disk").is_none());
    })
    .unwrap();
    // Changed again: Keep mine marks the tab, the next save overwrites.
    std::fs::write(&path, "id,name,more\n1,café,y\n".as_bytes()).unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_some());
        window.click("sheet-keep-mine", cx);
        window.render_frame(cx);
        assert!(panel.read(cx).is_dirty(cx), "kept yours: the tab is modified");
        assert_eq!(label(window, "sheet-message"), "kept yours; the next save overwrites the file");
        assert_eq!(dump(&view, cx).1[0], rows(&["1", "café", "x"]), "the grid keeps what was read");
    })
    .unwrap();
    cx.run_until_parked();
    // Grown past the budget outside: Reload shows the raw text and says so.
    {
        use std::io::Write as _;
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let line = format!("{},x\n", "a".repeat(1000));
        let mut written = 0usize;
        while written < 65 * 1024 * 1024 {
            file.write_all(line.as_bytes()).unwrap();
            written += line.len();
        }
    }
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_some());
        window.click("sheet-reload", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "past the budget after a reload: the text shows");
        assert_eq!(label(window, "sheet-summary"), "CSV, past the grid's 64 MB");
        assert!(label(window, "sheet-message").contains("past the grid's 64 MB"));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A workbook that does not open says why in the status row (C-013).
#[gpui_kit::test]
fn a_workbook_that_does_not_open_says_why(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-bad-book");
    let path = dir.join("bad.xlsx");
    std::fs::write(&path, b"not a workbook at all").unwrap();
    let (handle, panel, view) = open_sheet(cx, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "Excel, did not open");
        assert!(label(window, "sheet-message").starts_with("cannot open the workbook:"), "{}", label(window, "sheet-message"));
        assert!(dump(&view, cx).1.is_empty());
        assert!(panel.read(cx).editable_text(cx).is_none());
        // Still a workbook: no edit field, no header toggle, no raw text.
        assert!(window.try_find("sheet-edit").is_none());
        assert!(window.try_find("sheet-header").is_none());
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        assert_eq!(label(window, "sheet-message"), "a workbook has no text to show; Ctrl+Shift+B shows its bytes");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A save in another encoding renames it in the summary: the file holds UTF-8 now, so the grid says so (C-013).
#[gpui_kit::test]
fn a_save_in_another_encoding_renames_it_in_the_summary(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-save-encoding");
    let path = dir.join("latin.csv");
    std::fs::write(&path, b"id,name\n1,caf\xe9\n").unwrap();
    let (handle, _panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 1 row, 2 columns, comma, Latin-1");
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        let text = view.read(cx).text_view().unwrap().clone();
        text.update(cx, |text, cx| text.set_save_encoding(Encoding::Utf8, cx));
        let focus = text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("!", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-s", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&path).unwrap(), "!id,name\n1,café\n".as_bytes(), "written as UTF-8");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 1 row, 2 columns, comma, UTF-8", "as last written");
        assert_eq!(dump(&view, cx).0[0], "!id");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A workbook changed on disk raises the bar; a Reload that cannot read it keeps the sheets as read, says why
/// and takes the bar down (C-013).
#[gpui_kit::test]
fn a_workbook_reload_that_fails_keeps_the_sheets_and_says_why(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-book-reload");
    let path = copy_fixture("book.xlsx", &dir);
    let (handle, panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "Excel, 3 sheets · Data: 7 rows, 6 columns");
        window.click(("sheet-tab-button", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "Excel, 3 sheets · Notes: 2 rows, 2 columns");
    })
    .unwrap();
    // Rewritten outside as another book: Reload reads it afresh and opens on its first sheet.
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&path, fixture_bytes("book2.xlsx")).unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_some());
        window.click("sheet-reload", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "reloaded");
        assert_eq!(label(window, "sheet-summary"), "Excel, 1 sheet · Only: 1 row, 1 column", "the other book, on its first sheet");
        assert_eq!(dump(&view, cx).1[0][0], "changed", "read afresh");
        assert!(window.try_find("sheet-changed-on-disk").is_none());
    })
    .unwrap();
    // No further change on disk: the bar stays down through the next checks.
    cx.background_executor.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_none(), "the reload took the stamp");
    })
    .unwrap();
    // Changed again: Keep mine keeps the sheets as read, takes the bar down and marks nothing (nothing to save).
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&path, b"changed again").unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_some());
        window.click("sheet-keep-mine", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "kept the sheets as read");
        assert!(window.try_find("sheet-changed-on-disk").is_none());
        assert!(!panel.read(cx).is_dirty(cx), "a workbook has nothing to save");
        assert_eq!(dump(&view, cx).1[0][0], "changed");
    })
    .unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_none(), "keep mine took the stamp too");
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&path, b"no longer a workbook at all").unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("sheet-changed-on-disk").is_some(), "the bar shows for a workbook too");
        window.click("sheet-reload", cx);
        window.render_frame(cx);
        assert!(label(window, "sheet-message").starts_with("cannot open the workbook:"), "{}", label(window, "sheet-message"));
        assert_eq!(label(window, "sheet-summary"), "Excel, 1 sheet · Only: 1 row, 1 column", "the sheets as read stay");
        assert_eq!(dump(&view, cx).1[0][0], "changed");
        assert!(window.try_find("sheet-changed-on-disk").is_none(), "the bar is down");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A mixed column sorts numbers by value before text, text case-insensitively, empty cells last either way:
/// the first click is the reverse of that order, the second the order itself (C-013).
#[gpui_kit::test]
fn a_mixed_column_sorts_numbers_before_text_case_insensitively(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-mixed-sort");
    let path = dir.join("mixed.csv");
    // `b`, `A`, `C`: byte order would give A, C, b; case-insensitive order gives A, b, C.
    std::fs::write(&path, "k,v\n1,b\n2,10\n3,A\n4,2\n5,C\n6,\n").unwrap();
    let (handle, _panel, view) = open_sheet(cx, path);
    let values = |view: &Entity<SheetView>, cx: &gpui_kit::App| -> Vec<String> { dump(view, cx).1.iter().map(|r| r[1].clone()).collect() };
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(values(&view, cx), rows(&["C", "b", "A", "10", "2", ""]), "descending: text before numbers, the empty cell last");
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(values(&view, cx), rows(&["2", "10", "A", "b", "C", ""]), "ascending: numbers by value, then text regardless of case, the empty cell last");
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(values(&view, cx), rows(&["b", "10", "A", "2", "C", ""]), "the file's order again");
        // A sort left on, then the header toggle: the file's order again, the sort gone.
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(values(&view, cx)[0], "C");
        window.click("sheet-header-button", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), None, "the header toggle drops the sort");
        assert_eq!(values(&view, cx), rows(&["v", "b", "10", "A", "2", "C", ""]), "the file's order, the header record a row");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `.tsv` is read by tabs and says so (C-013).
#[gpui_kit::test]
fn a_tsv_opens_by_tabs_and_says_so(cx: &mut TestAppContext) {
    let (handle, _panel, view) = open_sheet(cx, fixture("table.tsv"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "TSV, 3 rows, 5 columns, tab, UTF-8");
        assert_eq!(dump(&view, cx).1[0][1], "Rossi, Mario", "a comma inside a field is no delimiter here");
    })
    .unwrap();
}

/// A UTF-16 file names its encoding before and after an edit saved back with its mark, that record alone
/// changed (C-013). One workspace per test: a second in the same context delivers Enter twice.
#[gpui_kit::test]
fn a_utf16_file_keeps_its_label_and_its_bytes(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-utf16");
    let path = copy_fixture("utf16.csv", &dir);
    let (handle, panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 3 rows, 5 columns, comma, UTF-16 LE");
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(1, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("Élodie Durand", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B3 set; Ctrl+S saves");
        assert_eq!(label(window, "sheet-summary"), "CSV, 3 rows, 5 columns, comma, UTF-16 LE", "the encoding as read, still");
        assert!(panel.read(cx).is_dirty(cx));
        window.press("ctrl-s", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let written = std::fs::read(&path).unwrap();
    assert_eq!(&written[..2], &[0xff, 0xfe], "the mark kept");
    let expected = String::from_utf8(fixture_bytes("table.csv")).unwrap().replace("2,Élodie,plain", "2,Élodie Durand,plain");
    let expected_bytes: Vec<u8> = [0xff, 0xfe].into_iter().chain(expected.encode_utf16().flat_map(u16::to_le_bytes)).collect();
    assert_eq!(written, expected_bytes, "that record alone, in UTF-16 LE");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 3 rows, 5 columns, comma, UTF-16 LE", "as last written");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A match far down a long file scrolls the grid to it (C-013).
#[gpui_kit::test]
fn a_match_far_down_scrolls_the_grid_to_it(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-scroll");
    let path = dir.join("long.csv");
    let mut text = String::from("n,word\n");
    for n in 1..=300 {
        text.push_str(&format!("{n},{}\n", if n == 290 { "needle" } else { "hay" }));
    }
    std::fs::write(&path, text).unwrap();
    let (handle, _panel, view) = open_sheet(cx, path);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).table().read(cx).visible_range().rows().start, 0, "at the top");
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        window.input("NEEDLE", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 1 of 1");
        assert_eq!(label(window, "sheet-cell"), "B291");
        let visible = view.read(cx).table().read(cx).visible_range().rows().clone();
        assert!(visible.start > 0 && visible.contains(&289), "scrolled to the match: rows {visible:?} on screen");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The other delimiters are named in the summary too (C-013).
#[gpui_kit::test]
fn a_semicolon_and_a_pipe_file_name_their_delimiter(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-delimiters");
    let semi = dir.join("semi.csv");
    std::fs::write(&semi, "a;b\n1;2\n").unwrap();
    let (handle, _panel, view) = open_sheet(cx, semi);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 1 row, 2 columns, semicolon, UTF-8");
        assert_eq!(dump(&view, cx).1[0], rows(&["1", "2"]));
    })
    .unwrap();
    let pipe = dir.join("pipe.csv");
    std::fs::write(&pipe, "a|b\n1|2\n").unwrap();
    let (handle, _panel, view) = open_sheet(cx, pipe);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "CSV, 1 row, 2 columns, pipe, UTF-8");
        assert_eq!(dump(&view, cx).1[0], rows(&["1", "2"]));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Under a sort, the selected cell is still named by the file's own records, the field holds that record's
/// text, the search selects by the file's rows, and Enter writes into that record (C-013).
#[gpui_kit::test]
fn a_cell_selected_under_a_sort_is_its_own_record(cx: &mut TestAppContext) {
    let dir = scratch_dir("sheet-sorted-edit");
    let path = copy_fixture("table.csv", &dir);
    let (handle, _panel, view) = open_sheet(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // amount descending: Élodie (record 2) comes first on screen.
        window.click(("icon-sort", 3usize), cx);
        window.render_frame(cx);
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(0, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "B3", "the first row on screen is record 2 of the file");
        assert_eq!(view.read(cx).edit_input().read(cx).value().as_ref(), "Élodie");
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        window.input("rossi", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "match 1 of 1");
        assert_eq!(label(window, "sheet-cell"), "B2", "Rossi is record 1 whatever the sort shows first");
        assert_eq!(view.read(cx).table().read(cx).selected_cell(), Some((1, 1)), "on the second row on screen");
        assert!(view.read(cx).table().read(cx).delegate().sort().is_some(), "the sort stays through a search");
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_cell(0, 1, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-cell"), "B3");
        let edit = view.read(cx).edit_input().clone();
        edit.update(cx, |edit, cx| edit.set_value("Élodie Durand", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-message"), "B3 set; Ctrl+S saves");
        let text = view.read(cx).text_view().unwrap().read(cx).text(cx).to_string();
        assert!(text.contains("\n2,Élodie Durand,plain,12.50,2026-02-28\n"), "{text}");
        assert!(text.contains("\n1,\"Rossi, Mario\",\"first line\nsecond line\",7,2026-01-05\n"), "record 1 untouched: {text}");
        let (_, cells) = dump(&view, cx);
        assert_eq!(cells[1][1], "Élodie Durand", "the grid in the file's order again");
        assert_eq!(view.read(cx).table().read(cx).selected_cell(), Some((1, 1)), "the edited cell comes up again where it stands now");
        assert_eq!(label(window, "sheet-cell"), "B3");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A legacy xls opens through the viewer as a workbook, on its sheets (C-013).
#[gpui_kit::test]
fn a_legacy_xls_opens_as_a_workbook(cx: &mut TestAppContext) {
    let (handle, panel, view) = open_sheet(cx, fixture("book.xls"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "sheet-summary"), "Excel, 3 sheets · Data: 7 rows, 6 columns");
        assert_eq!(view.read(cx).sheet_names(), rows(&["Data", "Notes", "Empty"]));
        assert_eq!(dump(&view, cx).1[1], rows(&["1", "Rossi, Mario", "12.5", "2026-01-05", "TRUE", "first line\nsecond line"]));
        assert!(panel.read(cx).editable_text(cx).is_none(), "read-only");
    })
    .unwrap();
}

#[test]
fn the_workbook_names_are_the_excel_family_in_any_case() {
    use sfoglia::viewers::sheet_view::is_workbook_name;
    for name in ["a.xls", "a.xlt", "a.xlsm", "a.xlsx", "a.xltx", "a.xltm", "A.XLSX", "A.XLS"] {
        assert!(is_workbook_name(name), "{name}");
    }
    for name in ["a.csv", "a.tsv", "a.txt", "xlsx", "a.xlsx.csv"] {
        assert!(!is_workbook_name(name), "{name}");
    }
}
