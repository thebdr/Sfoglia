//! Spreadsheets and delimited tables (C-013): a csv or tsv as a grid over the text viewer (the raw text
//! editable and saved as C-008 states, a cell edit written back into its record alone), and an Excel
//! workbook as read-only sheets, both on the kit's table with virtual rows, resizable, sortable columns
//! and cell selection.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Role, SharedString, Subscription, Task,
    TestSupportExt, Window, actions, div, px,
};

use crate::sheet::{self, Cell, CellKind, Grid, Merge, Workbook, cell_name, column_name, compare_cells};
use crate::viewers::rules_panel::RulesStore;
use crate::viewers::text_view::{DiskStamp, TextView};

actions!(sheet_view, [FocusSearch, ToggleRaw]);

/// The key context of the whole view: the chords work from the search field and the raw text too.
pub const KEY_CONTEXT: &str = "SheetView";

/// A delimited text past this size opens on its raw text: the grid holds every cell in memory.
pub const MAX_GRID_BYTES: usize = 64 * 1024 * 1024;

/// What the file is.
enum Source {
    /// A delimited text: its grid as last parsed, and how it was read.
    Delimited { grid: Grid, encoding: String, header: bool },
    /// An Excel workbook and the sheet on screen.
    Workbook { book: Workbook, sheet: usize },
    /// A delimited text past the budget (the raw text shows) or a workbook that did not open.
    None,
}

/// The cells the table shows: the current sheet's rows, their display order under the sort, and the
/// column names.
pub struct SheetDelegate {
    rows: Vec<Vec<Cell>>,
    columns: usize,
    /// The columns' names: the header record's fields, or letters.
    names: Vec<String>,
    /// Display row → data row.
    order: Vec<usize>,
    sort: Option<(usize, ColumnSort)>,
    merges: Vec<Merge>,
    /// Pixel widths per column, sized to the content.
    widths: Vec<f32>,
    /// The cells the last search matched, as (data row, column).
    matches: Vec<(usize, usize)>,
}

impl SheetDelegate {
    fn new(rows: Vec<Vec<Cell>>, columns: usize, names: Vec<String>, merges: Vec<Merge>) -> Self {
        let widths = (0..columns)
            .map(|c| {
                let longest = rows
                    .iter()
                    .take(500)
                    .filter_map(|r| r.get(c))
                    .map(|cell| cell.text.lines().map(|l| l.chars().count()).max().unwrap_or(0))
                    .max()
                    .unwrap_or(0)
                    .max(names.get(c).map(|n| n.chars().count()).unwrap_or(1));
                (longest as f32 * 8. + 24.).clamp(60., 400.)
            })
            .collect();
        let order = (0..rows.len()).collect();
        Self { rows, columns, names, order, sort: None, merges, widths, matches: Vec::new() }
    }

    fn resort(&mut self) {
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        if let Some((column, sort)) = self.sort
            && sort != ColumnSort::Default
        {
            let empty = Cell::empty();
            order.sort_by(|&a, &b| {
                let x = self.rows[a].get(column).unwrap_or(&empty);
                let y = self.rows[b].get(column).unwrap_or(&empty);
                // Empty cells stay last either way: the direction flips the rest.
                match (x.kind == CellKind::Empty, y.kind == CellKind::Empty) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    (false, false) => {
                        let ordering = compare_cells(x, y);
                        if sort == ColumnSort::Descending { ordering.reverse() } else { ordering }
                    }
                }
            });
        }
        self.order = order;
    }

    /// The display row of a data row.
    fn display_row(&self, data_row: usize) -> Option<usize> {
        self.order.iter().position(|&r| r == data_row)
    }

    pub fn data_row(&self, display_row: usize) -> Option<usize> {
        self.order.get(display_row).copied()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn sort(&self) -> Option<(usize, ColumnSort)> {
        self.sort
    }
}

impl TableDelegate for SheetDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns
    }

    fn rows_count(&self, _: &App) -> usize {
        self.order.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let name = self.names.get(col_ix).cloned().unwrap_or_else(|| column_name(col_ix));
        let mut column = Column::new(format!("c{col_ix}"), name).sortable().width(px(self.widths[col_ix]));
        if let Some((c, sort)) = self.sort
            && c == col_ix
        {
            column = column.sort(sort);
        }
        column
    }

    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, _: &mut Window, cx: &mut Context<TableState<Self>>) {
        self.sort = if sort == ColumnSort::Default { None } else { Some((col_ix, sort)) };
        self.resort();
        cx.notify();
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let data_row = self.order.get(row_ix).copied().unwrap_or(row_ix);
        self.rows.get(data_row).and_then(|r| r.get(col_ix)).map(|c| c.text.clone()).unwrap_or_default()
    }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, cx: &mut Context<TableState<Self>>) -> impl IntoElement {
        let empty = Cell::empty();
        let data_row = self.order.get(row_ix).copied().unwrap_or(row_ix);
        let cell = self.rows.get(data_row).and_then(|r| r.get(col_ix)).unwrap_or(&empty);
        let covered = self.merges.iter().any(|m| {
            m.rows.contains(&data_row) && m.columns.contains(&col_ix) && !(m.rows.start == data_row && m.columns.start == col_ix)
        });
        let marked = self.matches.binary_search(&(data_row, col_ix)).is_ok();
        let theme = cx.theme();
        let text: SharedString = if cell.text.contains('\n') { cell.text.replace('\n', " ⏎ ").into() } else { cell.text.clone().into() };
        div()
            .id(("cell", row_ix * 4096 + col_ix))
            .test_support()
            .aria_label(cell.text.clone())
            .size_full()
            .px_2()
            .whitespace_nowrap()
            .overflow_hidden()
            .when(matches!(cell.kind, CellKind::Number(_)), |d| d.text_right())
            .when(covered, |d| d.bg(theme.muted.opacity(0.4)))
            .when(marked, |d| d.bg(theme.primary).text_color(theme.primary_foreground))
            .child(text)
    }
}

pub struct SheetView {
    focus_handle: FocusHandle,
    path: PathBuf,
    name: String,
    rules: Entity<RulesStore>,
    /// The file's bytes, kept until the text viewer takes them, and the disk as it was when they were
    /// read, so the text viewer notices a writer that came in between (C-008).
    bytes: Vec<u8>,
    read_at: Option<DiskStamp>,
    source: Source,
    table: Entity<TableState<SheetDelegate>>,
    search: Entity<InputState>,
    /// The edit field beneath a delimited grid, holding the selected cell's text.
    edit: Entity<InputState>,
    text: Option<Entity<TextView>>,
    raw_shown: bool,
    status: String,
    /// Another writer noticed while the text viewer does not exist yet (C-008).
    disk_changed: bool,
    seen_operations: u64,
    seen_reloads: u64,
    _subscriptions: Vec<Subscription>,
    _watch: Task<()>,
}

impl SheetView {
    /// Open `bytes` as the file at `path`: a workbook read through calamine when the name says so, a
    /// delimited text otherwise (past the budget: on its raw text).
    pub fn new(path: &Path, bytes: &[u8], rules: &Entity<RulesStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("find a cell"));
        let edit = cx.new(|cx| InputState::new(window, cx).placeholder("select a cell to edit it"));
        let on_search = cx.subscribe_in(&search, window, |this, state, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = state.read(cx).value().to_string();
                this.find_next(&text, window, cx);
            }
        });
        let on_edit = cx.subscribe_in(&edit, window, |this, state, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = state.read(cx).value().to_string();
                this.apply_edit(&text, window, cx);
            }
        });
        let table = cx.new(|cx| {
            TableState::new(SheetDelegate::new(Vec::new(), 0, Vec::new(), Vec::new()), window, cx)
                .sortable(true)
                .col_resizable(true)
                .cell_selectable(true)
                .row_header(true)
        });
        let on_table = cx.subscribe_in(&table, window, |this, table, event, window, cx| {
            if let TableEvent::SelectCell(row, col) = event {
                let data_row = table.read(cx).delegate().data_row(*row);
                this.select_cell(data_row.map(|r| (r, *col)), window, cx);
            }
        });
        let watch = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                if this.update(cx, |this, cx| this.check_disk(cx)).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            path: path.to_path_buf(),
            name: name.clone(),
            rules: rules.clone(),
            bytes: bytes.to_vec(),
            read_at: DiskStamp::of(path),
            source: Source::None,
            table,
            search,
            edit,
            text: None,
            raw_shown: false,
            status: String::new(),
            disk_changed: false,
            seen_operations: 0,
            seen_reloads: 0,
            _subscriptions: vec![on_search, on_edit, on_table],
            _watch: watch,
        };
        if is_workbook_name(&name) {
            match sheet::read_workbook(path) {
                Ok(book) => {
                    view.source = Source::Workbook { book, sheet: 0 };
                    view.rebuild_table(cx);
                }
                Err(error) => view.status = format!("cannot open the workbook: {error}"),
            }
        } else {
            // The budget is measured on the decoded text, as it is again after a toggle or a reload.
            let decoded = crate::text::decode(bytes);
            if decoded.text.len() > MAX_GRID_BYTES {
                view.status = too_big_message(decoded.text.len());
                view.show_raw(window, cx);
                return view;
            }
            let encoding = decoded.encoding.label().to_string();
            // Parsed with line feeds alone, as the text an edit is written into (C-008's save puts the
            // file's endings back): the record spans and the cells agree before and after an edit.
            let text = crate::data::with_line_feeds(&decoded.text);
            view.parse_delimited(&text, encoding, true);
            view.rebuild_table(cx);
        }
        view
    }

    /// Parse a delimited text into the grid, the delimiter detected from the text and the name.
    fn parse_delimited(&mut self, text: &str, encoding: String, header: bool) {
        let delimiter = sheet::detect_delimiter(text, &self.name);
        let grid = sheet::parse_delimited(text, delimiter);
        self.source = Source::Delimited { grid, encoding, header };
    }

    /// Rebuild the table's delegate from the source: the sheet on screen, or the grid with or without
    /// its header record as column names. The sort and the selection start afresh.
    fn rebuild_table(&mut self, cx: &mut Context<Self>) {
        let delegate = match &self.source {
            Source::Delimited { grid, header, .. } => {
                let (names, data): (Vec<String>, &[Vec<String>]) = if *header && !grid.rows.is_empty() {
                    let first = &grid.rows[0];
                    let names = (0..grid.columns).map(|c| first.get(c).cloned().unwrap_or_else(|| column_name(c))).collect();
                    (names, &grid.rows[1..])
                } else {
                    ((0..grid.columns).map(column_name).collect(), &grid.rows[..])
                };
                let rows = data.iter().map(|r| r.iter().map(|f| sheet::classify(f)).collect()).collect();
                SheetDelegate::new(rows, grid.columns, names, Vec::new())
            }
            Source::Workbook { book, sheet } => match book.sheets.get(*sheet) {
                Some(s) => SheetDelegate::new(s.rows.clone(), s.columns, (0..s.columns).map(column_name).collect(), s.merges.clone()),
                None => SheetDelegate::new(Vec::new(), 0, Vec::new(), Vec::new()),
            },
            Source::None => SheetDelegate::new(Vec::new(), 0, Vec::new(), Vec::new()),
        };
        self.table.update(cx, |table, cx| {
            *table.delegate_mut() = delegate;
            table.clear_selection(cx);
            table.refresh(cx);
        });
        cx.notify();
    }

    /// The first record's index of the data rows: 1 with the header on, 0 without.
    fn record_offset(&self) -> usize {
        match &self.source {
            Source::Delimited { header: true, .. } => 1,
            _ => 0,
        }
    }

    /// The table's selected cell as (data row, column); a sort or a rebuild clears it with the table's own.
    fn selection(&self, cx: &App) -> Option<(usize, usize)> {
        let table = self.table.read(cx);
        table.selected_cell().and_then(|(display, col)| table.delegate().data_row(display).map(|row| (row, col)))
    }

    /// A cell came up in the table: the edit field takes its text, a line break shown as ⏎ (the field is
    /// one line) and written back as a break.
    fn select_cell(&mut self, cell: Option<(usize, usize)>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((row, col)) = cell
            && let Source::Delimited { grid, .. } = &self.source
        {
            let record = row + self.record_offset();
            let text = grid.rows.get(record).and_then(|r| r.get(col)).cloned().unwrap_or_default();
            self.edit.update(cx, |edit, cx| edit.set_value(text.replace('\n', "⏎"), window, cx));
        }
        cx.notify();
    }

    /// The selected cell's name as Excel writes it (`B3`), by the file's own row numbers.
    pub fn selected_cell_name(&self, cx: &App) -> Option<String> {
        self.selection(cx).map(|(row, col)| cell_name(row + self.record_offset(), col))
    }

    /// Write `text` into the selected cell's record, that record alone re-written in the file's dialect,
    /// so the raw text holds the edit and C-008's save writes it; the grid follows the text.
    fn apply_edit(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, col)) = self.selection(cx) else {
            self.status = "select a cell first".to_string();
            cx.notify();
            return;
        };
        let (record_ix, delimiter, span, fields) = match &self.source {
            Source::Delimited { grid, .. } => {
                let record_ix = row + self.record_offset();
                let Some(record) = grid.rows.get(record_ix) else { return };
                // Left as the field showed it (a break as ⏎, a cell past the record's end empty): nothing to
                // write, whatever the record's own quoting was.
                let original = record.get(col).cloned().unwrap_or_default();
                if text == original.replace('\n', "⏎") {
                    self.status = format!("{} unchanged", cell_name(record_ix, col));
                    cx.notify();
                    return;
                }
                // The field is one line: a ⏎ typed stands for a line break and goes back as one.
                let text = text.replace('⏎', "\n");
                let mut fields = record.clone();
                while fields.len() <= col {
                    fields.push(String::new());
                }
                fields[col] = text;
                (record_ix, grid.delimiter, grid.spans[record_ix].clone(), fields)
            }
            _ => return,
        };
        let view = self.ensure_text(window, cx);
        if !view.read(cx).is_editable() {
            self.status = "the text is read-only (it has units that did not decode), so it stays as it is".to_string();
            cx.notify();
            return;
        }
        let current = crate::data::with_line_feeds(&self.current_text(cx)).into_owned();
        let Some(span) = clamp_span(&current, span) else {
            self.status = "the text changed under the grid; Ctrl+Shift+D and back rebuilds it".to_string();
            cx.notify();
            return;
        };
        let record = sheet::format_record(&fields, delimiter);
        let mut next = String::with_capacity(current.len() + record.len());
        next.push_str(&current[..span.start]);
        next.push_str(&record);
        next.push_str(&current[span.end..]);
        let editor = view.read(cx).editor().clone();
        editor.update(cx, |editor, cx| editor.replace_all(next, window, cx));
        view.update(cx, |view, cx| view.note_edit(cx));
        self.reparse(cx);
        self.rebuild_table(cx);
        // The grid is rebuilt in the file's order: the edited cell comes up again where it stands now.
        let display = self.table.read(cx).delegate().display_row(row);
        if let Some(display) = display {
            self.table.update(cx, |table, cx| table.set_selected_cell(display, col, cx));
        }
        self.status = format!("{} set; Ctrl+S saves", cell_name(record_ix, col));
        cx.notify();
    }

    /// Find the next cell containing `text` (case-insensitively), from the selected cell on in display
    /// order, around the end; select it, scroll to it, and say how many cells match.
    pub fn find_next(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if text.is_empty() {
            self.status = "type something to find".to_string();
            cx.notify();
            return;
        }
        let needle = text.to_lowercase();
        let (matches, next) = {
            let table = self.table.read(cx);
            let delegate = table.delegate();
            let mut matches: Vec<(usize, usize)> = Vec::new();
            for (display, &data_row) in delegate.order.iter().enumerate() {
                for (col, cell) in delegate.rows[data_row].iter().enumerate() {
                    if cell.text.to_lowercase().contains(&needle) {
                        matches.push((display, col));
                    }
                }
            }
            let after = table.selected_cell();
            let next = matches.iter().position(|&m| after.is_none_or(|a| m > a)).unwrap_or(0);
            (matches, next)
        };
        if matches.is_empty() {
            self.status = format!("no cell matches \"{text}\"");
            self.table.update(cx, |table, cx| {
                table.delegate_mut().matches = Vec::new();
                cx.notify();
            });
            cx.notify();
            return;
        }
        let (display, col) = matches[next];
        let data_row = self.table.read(cx).delegate().data_row(display).unwrap_or(display);
        let mut marked: Vec<(usize, usize)> = matches
            .iter()
            .map(|&(d, c)| (self.table.read(cx).delegate().data_row(d).unwrap_or(d), c))
            .collect();
        marked.sort_unstable();
        self.table.update(cx, |table, cx| {
            table.delegate_mut().matches = marked;
            table.set_selected_cell(display, col, cx);
            table.scroll_to_row(display, cx);
        });
        self.select_cell(Some((data_row, col)), window, cx);
        self.status = format!("match {} of {}", next + 1, matches.len());
        cx.notify();
    }

    /// Until the text viewer exists and watches the disk itself: notice another writer.
    fn check_disk(&mut self, cx: &mut Context<Self>) {
        if self.text.is_some() || self.disk_changed {
            return;
        }
        if DiskStamp::of(&self.path) != self.read_at {
            self.disk_changed = true;
            cx.notify();
        }
    }

    /// Whether another program changed the file since it was read (C-008's bar, on the grid side too).
    pub fn changed_on_disk(&self, cx: &App) -> bool {
        match &self.text {
            Some(text) => text.read(cx).changed_on_disk(),
            None => self.disk_changed,
        }
    }

    fn on_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_workbook() {
            // The bar comes down either way: the sheets as read stay when the file cannot be read now.
            self.read_at = DiskStamp::of(&self.path);
            self.disk_changed = false;
            match sheet::read_workbook(&self.path) {
                Ok(book) => {
                    self.source = Source::Workbook { book, sheet: 0 };
                    self.rebuild_table(cx);
                    self.status = "reloaded".to_string();
                }
                Err(error) => self.status = format!("cannot open the workbook: {error}"),
            }
            cx.notify();
            return;
        }
        let text = self.ensure_text(window, cx);
        text.update(cx, |text, cx| text.reload(window, cx));
        self.disk_changed = false;
        self.follow_text("reloaded".to_string(), window, cx);
    }

    fn show_text_message(&mut self, cx: &App) {
        let Some(text) = &self.text else { return };
        let operations = text.read(cx).operations();
        if operations != self.seen_operations {
            self.seen_operations = operations;
            if let Some(message) = text.read(cx).message() {
                self.status = message.to_string();
            }
        }
    }

    /// The text viewer ran a save, a reload or a keep-mine: its message shows, and after a reload the
    /// grid follows the text it holds now.
    fn follow_text(&mut self, idle: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.text.clone() {
            self.show_text_message(cx);
            let reloads = text.read(cx).reloads();
            if reloads != self.seen_reloads {
                self.seen_reloads = reloads;
                // The budget and the encoding are measured again on the text as read now.
                let size = self.current_text(cx).len();
                if size > MAX_GRID_BYTES {
                    self.source = Source::None;
                    self.rebuild_table(cx);
                    self.status = too_big_message(size);
                    self.show_raw(window, cx);
                    return;
                }
                let encoding = text.read(cx).status().encoding.label().to_string();
                match &mut self.source {
                    Source::Delimited { encoding: read_as, .. } => *read_as = encoding,
                    _ => {
                        let text = crate::data::with_line_feeds(&self.current_text(cx)).into_owned();
                        self.parse_delimited(&text, encoding, true);
                    }
                }
                self.reparse(cx);
                self.rebuild_table(cx);
                self.status = idle;
            }
        }
        cx.notify();
    }

    fn on_keep_mine(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_workbook() {
            self.disk_changed = false;
            self.read_at = DiskStamp::of(&self.path);
            self.status = "kept the sheets as read".to_string();
            cx.notify();
            return;
        }
        let text = self.ensure_text(window, cx);
        text.update(cx, |text, cx| text.keep_mine(cx));
        self.disk_changed = false;
        self.seen_operations = text.read(cx).operations();
        self.status = "kept yours; the next save overwrites the file".to_string();
        cx.notify();
    }

    /// Build the text viewer on the file's bytes the first time it is needed (delimited files only).
    fn ensure_text(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextView> {
        if let Some(text) = &self.text {
            return text.clone();
        }
        let text = cx.new(|cx| TextView::new_as_read(&self.path, &self.bytes, self.read_at, &self.rules, window, cx));
        self._subscriptions.push(cx.observe_in(&text, window, |this, _, window, cx| this.follow_text(String::new(), window, cx)));
        self.text = Some(text.clone());
        self.bytes = Vec::new();
        text
    }

    fn show_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.ensure_text(window, cx);
        self.raw_shown = true;
        let focus = text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// The current text: the editor's, once it exists, else the file's.
    fn current_text(&self, cx: &App) -> String {
        match &self.text {
            Some(text) => text.read(cx).text(cx).to_string(),
            None => crate::text::decode(&self.bytes).text,
        }
    }

    /// Parse the current text again into the grid (the header choice kept).
    fn reparse(&mut self, cx: &App) {
        let (encoding, header) = match &self.source {
            Source::Delimited { encoding, header, .. } => (encoding.clone(), *header),
            _ => return,
        };
        let text = crate::data::with_line_feeds(&self.current_text(cx)).into_owned();
        self.parse_delimited(&text, encoding, header);
    }

    /// Back to the grid, rebuilt from the text as edited.
    fn show_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let size = self.current_text(cx).len();
        if size > MAX_GRID_BYTES {
            self.status = too_big_message(size);
            cx.notify();
            return;
        }
        if matches!(self.source, Source::None) {
            // The encoding the file was read in (not the one chosen for the next save).
            let decoded_encoding = self.text.as_ref().map(|t| t.read(cx).status().encoding.label().to_string()).unwrap_or_default();
            let text = crate::data::with_line_feeds(&self.current_text(cx)).into_owned();
            self.parse_delimited(&text, decoded_encoding, true);
        } else {
            self.reparse(cx);
        }
        self.rebuild_table(cx);
        self.raw_shown = false;
        self.status = String::new();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn on_toggle_raw(&mut self, _: &ToggleRaw, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_workbook() {
            self.status = "a workbook has no text to show; Ctrl+Shift+B shows its bytes".to_string();
            cx.notify();
            return;
        }
        if self.raw_shown {
            self.show_grid(window, cx);
        } else {
            self.show_raw(window, cx);
        }
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_shown {
            self.show_grid(window, cx);
        }
        let handle = self.search.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Switch the header record between column names and a row.
    pub fn toggle_header(&mut self, cx: &mut Context<Self>) {
        if let Source::Delimited { header, .. } = &mut self.source {
            *header = !*header;
            self.rebuild_table(cx);
        }
    }

    /// Show another sheet of the workbook.
    pub fn show_sheet(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Source::Workbook { sheet, book } = &mut self.source
            && index < book.sheets.len()
        {
            *sheet = index;
            self.rebuild_table(cx);
        }
    }

    // ---- what the workspace and the tests read ----

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_workbook(&self) -> bool {
        matches!(self.source, Source::Workbook { .. }) || (is_workbook_name(&self.name) && matches!(self.source, Source::None))
    }

    pub fn table(&self) -> &Entity<TableState<SheetDelegate>> {
        &self.table
    }

    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    pub fn edit_input(&self) -> &Entity<InputState> {
        &self.edit
    }

    /// The text viewer, once the raw text or an edit has been asked for (delimited files only).
    pub fn text_view(&self) -> Option<&Entity<TextView>> {
        self.text.as_ref()
    }

    pub fn raw_shown(&self) -> bool {
        self.raw_shown
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn header_on(&self) -> bool {
        matches!(self.source, Source::Delimited { header: true, .. })
    }

    pub fn sheet_names(&self) -> Vec<String> {
        match &self.source {
            Source::Workbook { book, .. } => book.sheets.iter().map(|s| s.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn current_sheet(&self) -> usize {
        match &self.source {
            Source::Workbook { sheet, .. } => *sheet,
            _ => 0,
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        self.text.as_ref().is_some_and(|text| text.read(cx).is_dirty())
    }

    /// `CSV, N rows, C columns, comma, UTF-8` or `Excel, S sheets · Name: N rows, C columns`; the encoding is
    /// the one the file holds as last read or written (the text viewer's, once it exists).
    pub fn summary(&self, cx: &App) -> String {
        let written_as = self.text.as_ref().map(|t| t.read(cx).status().encoding.label().to_string());
        match &self.source {
            Source::Delimited { grid, encoding, header } => {
                let encoding = written_as.unwrap_or_else(|| encoding.clone());
                let rows = grid.rows.len().saturating_sub(usize::from(*header && !grid.rows.is_empty()));
                let kind = if grid.delimiter == b'\t' { "TSV" } else { "CSV" };
                let delimiter = match grid.delimiter {
                    b',' => "comma",
                    b'\t' => "tab",
                    b';' => "semicolon",
                    b'|' => "pipe",
                    _ => "delimiter",
                };
                format!("{kind}, {rows} {}, {} {}, {delimiter}, {encoding}", plural(rows, "row"), grid.columns, plural(grid.columns, "column"))
            }
            Source::Workbook { book, sheet } => {
                let current = book.sheets.get(*sheet);
                let (name, rows, columns) = current.map(|s| (s.name.as_str(), s.rows.len(), s.columns)).unwrap_or(("", 0, 0));
                format!(
                    "Excel, {} {} · {name}: {rows} {}, {columns} {}",
                    book.sheets.len(),
                    plural(book.sheets.len(), "sheet"),
                    plural(rows, "row"),
                    plural(columns, "column")
                )
            }
            Source::None => {
                if is_workbook_name(&self.name) {
                    "Excel, did not open".to_string()
                } else {
                    let kind = if self.name.to_ascii_lowercase().ends_with(".tsv") { "TSV" } else { "CSV" };
                    format!("{kind}, past the grid's {} MB", MAX_GRID_BYTES / (1024 * 1024))
                }
            }
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 { word.to_string() } else { format!("{word}s") }
}

/// The file names calamine reads: the Excel family (the templates too), by extension.
pub fn is_workbook_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["xls", "xlt", "xlsm", "xlsx", "xltx", "xltm", "xlsb", "xlam"].iter().any(|ext| lower.ends_with(&format!(".{ext}")))
}

fn too_big_message(size: usize) -> String {
    format!(
        "{:.1} MB of text: past the grid's {} MB, so the text shows; Ctrl+Shift+F and the grid need a smaller file",
        size as f64 / (1024. * 1024.),
        MAX_GRID_BYTES / (1024 * 1024)
    )
}

/// A span kept inside the text and on character boundaries, or none when the text moved under it.
fn clamp_span(text: &str, span: Range<usize>) -> Option<Range<usize>> {
    if span.end > text.len() || !text.is_char_boundary(span.start) || !text.is_char_boundary(span.end) {
        return None;
    }
    Some(span)
}

impl Focusable for SheetView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match (&self.raw_shown, &self.text) {
            (true, Some(text)) => text.read(cx).editor().read(cx).focus_handle(cx),
            _ => self.focus_handle.clone(),
        }
    }
}

impl Render for SheetView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.show_text_message(cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let summary = self.summary(cx);
        let workbook = self.is_workbook();
        let hint = if self.raw_shown {
            "Ctrl+Shift+D shows the grid"
        } else if workbook {
            "the arrow in a header sorts, Ctrl+Shift+F finds a cell"
        } else {
            "the arrow in a header sorts, Ctrl+Shift+F finds a cell, Ctrl+Shift+D shows the text"
        };
        let status = if self.status.is_empty() { hint.to_string() } else { self.status.clone() };
        let cell = self.selected_cell_name(cx).unwrap_or_default();
        let header_label = if self.header_on() { "Header: on" } else { "Header: off" };
        let status_row = h_flex()
            .id("sheet-status")
            .test_support()
            .role(Role::Status)
            .aria_label(status.clone())
            .flex_shrink_0()
            .gap_4()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(border)
            .text_sm()
            .text_color(muted)
            .whitespace_nowrap()
            .overflow_hidden()
            .child(div().id("sheet-summary").test_support().aria_label(summary.clone()).child(summary))
            .when(!workbook && !self.raw_shown, |row| {
                row.child(
                    div().id("sheet-header").test_support().aria_label(header_label).child(
                        Button::new("sheet-header-button").label(header_label).xsmall().on_click(cx.listener(|this, _, _, cx| this.toggle_header(cx))),
                    ),
                )
            })
            .child(div().id("sheet-cell").test_support().aria_label(cell.clone()).child(cell))
            .child(div().id("sheet-message").test_support().aria_label(status.clone()).flex_1().overflow_hidden().child(status));

        let raw_shown = self.raw_shown;
        let changed = !raw_shown && self.changed_on_disk(cx);
        let warning = theme.warning.opacity(0.15);
        let body = if raw_shown
            && let Some(text) = &self.text
        {
            div().id("sheet-raw").test_support().size_full().min_h_0().child(text.clone()).into_any_element()
        } else {
            div().id("sheet-grid").test_support().size_full().min_h_0().child(self.table.clone()).into_any_element()
        };
        let names = self.sheet_names();
        let current = self.current_sheet();
        let sheet_bar = (!raw_shown && !names.is_empty()).then(|| {
            h_flex()
                .id("sheet-bar")
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(border)
                .children(names.into_iter().enumerate().map(|(ix, name)| {
                    let selected = ix == current;
                    div().id(("sheet-tab", ix)).test_support().aria_label(name.clone()).child(
                        Button::new(("sheet-tab-button", ix))
                            .label(name)
                            .xsmall()
                            .when(selected, |b| b.primary())
                            .on_click(cx.listener(move |this, _, _, cx| this.show_sheet(ix, cx))),
                    )
                }))
        });
        let tools = (!raw_shown).then(|| {
            h_flex()
                .id("sheet-tools")
                .flex_shrink_0()
                .gap_2()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(border)
                .child(div().w(px(260.)).child(Input::new(&self.search).id("sheet-search")))
                .child(Button::new("sheet-next").label("Next").xsmall().on_click(cx.listener(|this, _, window, cx| {
                    let text = this.search.read(cx).value().to_string();
                    this.find_next(&text, window, cx);
                })))
                .when(!workbook, |row| {
                    row.child(div().text_color(muted).text_sm().child("cell:"))
                        .child(div().flex_1().child(Input::new(&self.edit).id("sheet-edit")))
                })
        });

        v_flex()
            .id("sheet-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_toggle_raw))
            .on_action(cx.listener(Self::on_focus_search))
            .size_full()
            .min_h_0()
            .when(changed, |col| {
                col.child(
                    h_flex()
                        .id("sheet-changed-on-disk")
                        .test_support()
                        .role(Role::Status)
                        .aria_label("changed on disk")
                        .flex_shrink_0()
                        .gap_3()
                        .px_3()
                        .py_1()
                        .bg(warning)
                        .text_sm()
                        .child("The file changed on disk.")
                        .child(Button::new("sheet-reload").label("Reload").xsmall().on_click(cx.listener(|this, _, window, cx| this.on_reload(window, cx))))
                        .child(Button::new("sheet-keep-mine").label("Keep mine").xsmall().on_click(cx.listener(|this, _, window, cx| this.on_keep_mine(window, cx)))),
                )
            })
            .children(sheet_bar)
            .children(tools)
            .child(body)
            .child(status_row)
    }
}
