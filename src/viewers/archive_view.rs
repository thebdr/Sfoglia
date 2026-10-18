//! Archives, read side (C-015): the entries of a zip, a 7z, a tar (plain or through a codec), a lone
//! compressed file or, through an installed 7-Zip, a rar and the containers no library here reads, as a
//! sortable, searchable table. Enter or a double click opens an entry inside Sfoglia; `Extract all` and
//! `Extract selected` write entries into a chosen folder, never outside it; an encrypted archive asks for
//! its password in the tools row. With no 7-Zip for a format that needs one, the status row says what to
//! install and the bytes show meanwhile.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::{Column, ColumnSort, TableDelegate, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, Context, Entity, FocusHandle, Focusable, IntoElement, PathPromptOptions, Render, Role, Subscription,
    TestSupportExt, Window, actions, div, px,
};

use crate::archive::{self, ArchiveError, Container, Entry, Listing, SevenZipTool, TempFolder};
use crate::hex::HexFile;
use crate::tools::{self, Found, Tool, ToolSet};
use crate::viewers::hex_view::HexView;

actions!(archive_view, [FocusSearch, OpenEntry, ExtractAll, ExtractSelected, SelectUp, SelectDown, SelectFirst, SelectLast]);

/// Asks the workspace to open entry `index` of the archive at `archive` inside Sfoglia, in a tab of its own.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = sfoglia, no_json)]
pub struct OpenArchiveEntry {
    pub archive: PathBuf,
    pub index: usize,
}

/// The key context of the whole view: the chords work from the fields too.
pub const KEY_CONTEXT: &str = "ArchiveView";
/// The key context of the entry rows: Enter opens the selected entry there, and stays the fields' own key
/// elsewhere.
pub const ROWS_CONTEXT: &str = "ArchiveRows";

const COLUMNS: [&str; 6] = ["Name", "Size", "Packed", "Modified", "Method", "CRC"];

/// The entries the table shows, in their display order under the filter and the sort.
pub struct ArchiveDelegate {
    entries: Vec<Entry>,
    /// Display row → entry index.
    order: Vec<usize>,
    sort: Option<(usize, ColumnSort)>,
    filter: String,
    widths: [f32; 6],
}

impl ArchiveDelegate {
    fn new(entries: Vec<Entry>) -> Self {
        let longest = entries.iter().take(2000).map(|e| e.name.chars().count()).max().unwrap_or(4);
        let widths = [(longest as f32 * 8. + 24.).clamp(160., 520.), 100., 100., 150., 110., 90.];
        let mut delegate = Self { entries, order: Vec::new(), sort: None, filter: String::new(), widths };
        delegate.rebuild();
        delegate
    }

    /// The display order: the entries the filter keeps (a substring of the name, any case), in the sort's
    /// order.
    fn rebuild(&mut self) {
        let needle = self.filter.to_lowercase();
        let mut order: Vec<usize> = (0..self.entries.len())
            .filter(|&i| needle.is_empty() || self.entries[i].name.to_lowercase().contains(&needle))
            .collect();
        if let Some((column, sort)) = self.sort
            && sort != ColumnSort::Default
        {
            order.sort_by(|&a, &b| {
                let ordering = compare(&self.entries[a], &self.entries[b], column);
                if sort == ColumnSort::Descending { ordering.reverse() } else { ordering }
            });
        }
        self.order = order;
    }

    /// The entry a display row shows.
    pub fn entry_index(&self, display_row: usize) -> Option<usize> {
        self.order.get(display_row).copied()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn sort(&self) -> Option<(usize, ColumnSort)> {
        self.sort
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// What a cell says: the name with its path, the sizes in bytes, the date, the method, the CRC in hex.
    pub fn cell(entry: &Entry, column: usize) -> String {
        match column {
            0 => entry.name.clone(),
            1 => {
                if entry.is_dir {
                    String::new()
                } else {
                    entry.size.to_string()
                }
            }
            2 => entry.packed.filter(|_| !entry.is_dir).map(|p| p.to_string()).unwrap_or_default(),
            3 => entry.modified.clone().unwrap_or_default(),
            4 => entry.method.clone(),
            _ => entry.crc.map(|c| format!("{c:08X}")).unwrap_or_default(),
        }
    }
}

/// Entries compared on a column, ties broken by the name so the order is the same every time.
fn compare(a: &Entry, b: &Entry, column: usize) -> Ordering {
    let by_column = match column {
        0 => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        1 => a.size.cmp(&b.size),
        2 => a.packed.cmp(&b.packed),
        3 => a.modified.cmp(&b.modified),
        4 => a.method.cmp(&b.method),
        _ => a.crc.cmp(&b.crc),
    };
    by_column.then_with(|| a.name.cmp(&b.name))
}

impl TableDelegate for ArchiveDelegate {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.order.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let mut column = Column::new(format!("c{col_ix}"), COLUMNS[col_ix]).sortable().width(px(self.widths[col_ix]));
        if let Some((c, sort)) = self.sort
            && c == col_ix
        {
            column = column.sort(sort);
        }
        column
    }

    fn perform_sort(&mut self, col_ix: usize, sort: ColumnSort, _: &mut Window, cx: &mut Context<TableState<Self>>) {
        self.sort = if sort == ColumnSort::Default { None } else { Some((col_ix, sort)) };
        self.rebuild();
        cx.notify();
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        self.entry_index(row_ix).map(|i| Self::cell(&self.entries[i], col_ix)).unwrap_or_default()
    }

    fn render_td(&mut self, row_ix: usize, col_ix: usize, _: &mut Window, _: &mut Context<TableState<Self>>) -> impl IntoElement {
        let text = self.entry_index(row_ix).map(|i| Self::cell(&self.entries[i], col_ix)).unwrap_or_default();
        let numeric = matches!(col_ix, 1 | 2 | 5);
        div()
            .id(("entry", row_ix * 8 + col_ix))
            .test_support()
            .aria_label(text.clone())
            .size_full()
            .px_2()
            .whitespace_nowrap()
            .overflow_hidden()
            .when(numeric, |d| d.text_right())
            .child(text)
    }
}

/// What the view holds.
enum State {
    /// The entries are listed.
    Listed(Listing),
    /// Encrypted headers: nothing lists until the password is right.
    Locked,
    /// The format needs 7-Zip, and none is installed: the bytes show meanwhile.
    NoTool,
    /// The file did not open as its container: the bytes show meanwhile.
    Failed(String),
}

pub struct ArchiveView {
    focus_handle: FocusHandle,
    /// Tracked by the rows' wrapper, so its key context and Enter binding sit on the focus path above the table.
    rows_focus: FocusHandle,
    path: PathBuf,
    name: String,
    size: Option<u64>,
    container: Option<Container>,
    tools: Arc<ToolSet>,
    /// 7-Zip as found, when the container goes through it.
    tool: Option<Found>,
    state: State,
    password: Option<String>,
    /// Wrong passwords so far, so each refusal reads as new.
    wrong: u32,
    table: Entity<TableState<ArchiveDelegate>>,
    search: Entity<InputState>,
    password_input: Entity<InputState>,
    /// What the last action said.
    message: Option<String>,
    hex: Option<Entity<HexView>>,
    hex_error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ArchiveView {
    /// Open the archive at `path`, listed through the library for its container or through the 7-Zip
    /// `tools` finds.
    pub fn new(path: &Path, tools: Arc<ToolSet>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("find an entry"));
        let password_input = cx.new(|cx| InputState::new(window, cx).placeholder("password"));
        let on_search = cx.subscribe_in(&search, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                this.set_filter(text, cx);
            }
        });
        let on_password = cx.subscribe_in(&password_input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.apply_password(window, cx);
            }
        });
        let table = cx.new(|cx| {
            TableState::new(ArchiveDelegate::new(Vec::new()), window, cx).sortable(true).col_resizable(true).row_selectable(true)
        });
        let on_table = cx.subscribe_in(&table, window, |this, table, event, window, cx| match event {
            TableEvent::DoubleClickedRow(row) => {
                if let Some(index) = table.read(cx).delegate().entry_index(*row) {
                    this.open_entry(index, window, cx);
                }
            }
            // A click on a row keeps the keyboard on the rows' wrapper, where Enter and the arrows are bound.
            TableEvent::SelectRow(_) => window.focus(&this.rows_focus, cx),
            _ => {}
        });
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            rows_focus: cx.focus_handle(),
            path: path.to_path_buf(),
            name,
            size: std::fs::metadata(path).ok().map(|m| m.len()),
            container: None,
            tools,
            tool: None,
            state: State::Locked,
            password: None,
            wrong: 0,
            table,
            search,
            password_input,
            message: None,
            hex: None,
            hex_error: None,
            _subscriptions: vec![on_search, on_password, on_table],
        };
        view.load(window, cx);
        view
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn container(&self) -> Option<Container> {
        self.container
    }

    /// The listing, while the entries are listed.
    pub fn listing(&self) -> Option<&Listing> {
        match &self.state {
            State::Listed(listing) => Some(listing),
            _ => None,
        }
    }

    /// `listed`, `locked`, `missing` (no 7-Zip) or `failed`.
    pub fn state_name(&self) -> &'static str {
        match self.state {
            State::Listed(_) => "listed",
            State::Locked => "locked",
            State::NoTool => "missing",
            State::Failed(_) => "failed",
        }
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Why the file did not open as its container, when it did not.
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            State::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    /// The 7-Zip in use, when the container goes through it.
    pub fn tool(&self) -> Option<&Found> {
        self.tool.as_ref()
    }

    pub fn table(&self) -> &Entity<TableState<ArchiveDelegate>> {
        &self.table
    }

    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    pub fn password_input(&self) -> &Entity<InputState> {
        &self.password_input
    }

    /// The hex view, while the bytes show.
    pub fn hex_view(&self) -> Option<&Entity<HexView>> {
        self.hex.as_ref()
    }

    /// The name of entry `index` inside the archive.
    pub fn entry_name(&self, index: usize) -> Option<String> {
        self.listing().and_then(|l| l.entries.get(index)).map(|e| e.name.clone())
    }

    /// The entry the selected row shows.
    pub fn selected_entry(&self, cx: &App) -> Option<usize> {
        let table = self.table.read(cx);
        table.selected_row().and_then(|row| table.delegate().entry_index(row))
    }

    /// The names the table shows, top to bottom.
    pub fn shown_names(&self, cx: &App) -> Vec<String> {
        let table = self.table.read(cx);
        let delegate = table.delegate();
        (0..delegate.rows_count(cx)).map(|row| delegate.cell_text(row, 0, cx)).collect()
    }

    /// `ZIP, 5 entries, 789 B packed from 5.4 KB`; the container alone while nothing is listed.
    pub fn summary(&self) -> String {
        match (&self.state, self.container) {
            (State::Listed(listing), _) => listing.describe(&self.name, self.size),
            (State::Locked, Some(container)) => format!("{}, encrypted", container.label(&self.name)),
            (_, Some(container)) => container.label(&self.name),
            (_, None) => String::new(),
        }
    }

    /// `7-Zip 24.08`, or `no 7-Zip`, when the container goes through it.
    pub fn tool_label(&self) -> Option<String> {
        if self.container != Some(Container::External) {
            return None;
        }
        Some(match &self.tool {
            Some(found) => found.describe(Tool::SevenZip),
            None => "no 7-Zip".to_string(),
        })
    }

    /// Whether `Look again` is offered: the container needs 7-Zip and none is found.
    pub fn wants_look_again(&self) -> bool {
        matches!(self.state, State::NoTool)
    }

    /// Whether the password field shows: the archive is encrypted, in its headers or its entries.
    pub fn asks_password(&self) -> bool {
        match &self.state {
            State::Locked => true,
            State::Listed(listing) => listing.encrypted,
            _ => false,
        }
    }

    /// The status row's text: the last action's message, or the hint.
    pub fn message_text(&self) -> String {
        match &self.message {
            Some(message) => message.clone(),
            None => match self.state {
                State::Listed(_) => "Enter opens an entry inside Sfoglia, Ctrl+Shift+F finds one, the arrow in a header sorts".to_string(),
                _ => String::new(),
            },
        }
    }

    fn seven_zip(&self) -> Option<SevenZipTool> {
        self.tool.as_ref().map(|found| SevenZipTool(found.path.clone()))
    }

    /// List the archive through what its container calls for.
    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let head = std::fs::File::open(&self.path)
            .and_then(|mut file| {
                use std::io::Read as _;
                let mut head = Vec::new();
                (&mut file).take(512).read_to_end(&mut head)?;
                Ok(head)
            })
            .unwrap_or_default();
        self.container = archive::container_of(&self.name, &head);
        self.tool = (self.container == Some(Container::External)).then(|| self.tools.find(Tool::SevenZip)).flatten();
        let list_with = |password: Option<&str>| match self.container {
            None => Err(ArchiveError::Bad(format!("{} is no archive", self.name))),
            Some(Container::External) if self.tool.is_none() => Err(ArchiveError::NoTool),
            Some(_) => archive::list(&self.path, password, self.seven_zip().as_ref()),
        };
        let mut result = list_with(self.password.as_deref());
        // A wrong password: said, and the archive listed without one where its headers allow it.
        let wrong = matches!(result, Err(ArchiveError::WrongPassword));
        if wrong {
            result = list_with(None);
        }
        match result {
            Ok(listing) => {
                self.set_entries(listing.entries.clone(), cx);
                self.state = State::Listed(listing);
                self.hex = None;
                self.message = wrong.then(|| self.wrong_message());
            }
            Err(ArchiveError::NeedsPassword) => {
                self.set_entries(Vec::new(), cx);
                self.state = State::Locked;
                self.message = Some(if wrong { self.wrong_message() } else { ArchiveError::NeedsPassword.to_string() });
            }
            Err(ArchiveError::WrongPassword) => {
                self.set_entries(Vec::new(), cx);
                self.state = State::Locked;
                self.message = Some(self.wrong_message());
            }
            Err(ArchiveError::NoTool) => {
                self.set_entries(Vec::new(), cx);
                self.state = State::NoTool;
                let what = format!("A {}", self.container.map(|c| c.label(&self.name)).unwrap_or_default());
                self.message = Some(format!("{} The bytes show meanwhile.", tools::missing_message(&what, &[Tool::SevenZip])));
                self.show_bytes(window, cx);
            }
            Err(ArchiveError::Bad(reason)) => {
                self.set_entries(Vec::new(), cx);
                self.state = State::Failed(reason.clone());
                self.message = Some(format!("cannot open {}: {reason}. The bytes show meanwhile.", self.name));
                self.show_bytes(window, cx);
            }
        }
        cx.notify();
    }

    /// The table's entries replaced, the filter and the sort kept.
    fn set_entries(&mut self, entries: Vec<Entry>, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            let filter = table.delegate().filter().to_string();
            let sort = table.delegate().sort();
            let mut delegate = ArchiveDelegate::new(entries);
            delegate.filter = filter;
            delegate.sort = sort;
            delegate.rebuild();
            *table.delegate_mut() = delegate;
            table.refresh(cx);
        });
    }

    /// The rows filtered to the names holding `text`.
    pub fn set_filter(&mut self, text: String, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            if delegate.filter != text {
                delegate.filter = text;
                delegate.rebuild();
                table.refresh(cx);
            }
        });
        cx.notify();
    }

    /// The password field's text taken as the password, the archive listed again with it.
    pub fn apply_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.password_input.read(cx).value().to_string();
        self.password = (!value.is_empty()).then_some(value);
        self.load(window, cx);
    }

    /// `Look again`: 7-Zip asked for afresh, the archive listed through what is found now.
    pub fn look_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tools.refresh(Tool::SevenZip);
        self.load(window, cx);
    }

    /// The bytes, under the message, while no listing shows.
    fn show_bytes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hex.is_none() && self.hex_error.is_none() {
            match HexFile::open(&self.path) {
                Ok(file) => self.hex = Some(cx.new(|cx| HexView::new(file, window, cx))),
                Err(error) => self.hex_error = Some(format!("Cannot map this file: {error}")),
            }
        }
    }

    /// Ask the workspace to open entry `index` in a tab of its own (a folder has nothing to open).
    pub fn open_entry(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.listing().and_then(|l| l.entries.get(index)).cloned() else { return };
        if entry.is_dir {
            self.message = Some(format!("{} is a folder", entry.name));
            cx.notify();
            return;
        }
        self.message = Some(format!("opening {}", entry.name));
        let action = OpenArchiveEntry { archive: self.path.clone(), index };
        // After this update ends: the workspace then asks this view for the entry's bytes.
        let focus = self.focus_handle.clone();
        window.defer(cx, move |window, cx| focus.dispatch_action(&action, window, cx));
        cx.notify();
    }

    /// Enter: the selected entry opens.
    pub fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_entry(cx) {
            Some(index) => self.open_entry(index, window, cx),
            None => {
                self.message = Some("select an entry first".to_string());
                cx.notify();
            }
        }
    }

    /// Entry `index` written into a folder of its own for a tab to open: the folder, the file and the name
    /// inside the archive; or the message now in the status row.
    pub fn materialize(&mut self, index: usize, cx: &mut Context<Self>) -> Result<(TempFolder, PathBuf, String), String> {
        let Some(listing) = self.listing() else { return Err("nothing is listed".to_string()) };
        let name = listing.entries.get(index).map(|e| e.name.clone()).ok_or_else(|| format!("no entry {index}"))?;
        match archive::materialize(&self.path, listing, index, self.password.as_deref(), self.seven_zip().as_ref()) {
            Ok((folder, file)) => Ok((folder, file, name)),
            Err(error) => {
                let message = self.explain(error);
                self.message = Some(message.clone());
                cx.notify();
                Err(message)
            }
        }
    }

    /// An error as the status row says it: a wrong password counted, the rest as it reads.
    fn explain(&mut self, error: ArchiveError) -> String {
        match error {
            ArchiveError::WrongPassword => self.wrong_message(),
            other => other.to_string(),
        }
    }

    /// `wrong password (try 2)`: one more wrong password, so each refusal reads as new.
    fn wrong_message(&mut self) -> String {
        self.wrong += 1;
        format!("wrong password (try {})", self.wrong)
    }

    /// `Extract all`: every entry into a folder the user picks.
    pub fn extract_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_folder(None, window, cx);
    }

    /// `Extract selected`: the selected entry into a folder the user picks.
    pub fn extract_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_entry(cx) {
            Some(index) => self.ask_folder(Some(vec![index]), window, cx),
            None => {
                self.message = Some("select an entry first".to_string());
                cx.notify();
            }
        }
    }

    fn ask_folder(&mut self, indices: Option<Vec<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        if self.listing().is_none() {
            self.message = Some("nothing is listed to extract".to_string());
            cx.notify();
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Extract".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(folder) = paths.first()
            {
                let folder = folder.clone();
                this.update(cx, |this, cx| this.extract_into(&folder, indices.as_deref(), cx)).ok();
            }
        })
        .detach();
    }

    /// Write the entries `indices` (all when `None`) into `folder`; the status row says what was written
    /// and what was skipped as climbing out.
    pub fn extract_into(&mut self, folder: &Path, indices: Option<&[usize]>, cx: &mut Context<Self>) {
        let Some(listing) = self.listing() else { return };
        let result = archive::extract(&self.path, listing, indices, folder, self.password.as_deref(), self.seven_zip().as_ref());
        self.message = Some(match result {
            Ok(done) => {
                let n = done.written.len();
                let noun = if n == 1 { "entry" } else { "entries" };
                let mut text = format!("{n} {noun} extracted into {}", folder.display());
                if !done.skipped.is_empty() {
                    text.push_str(&format!("; skipped as outside it: {}", done.skipped.join(", ")));
                }
                text
            }
            Err(error) => format!("extraction failed: {}", self.explain(error)),
        });
        cx.notify();
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn on_open_entry(&mut self, _: &OpenEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.open_selected(window, cx);
    }

    /// The selected row moved by `delta` (from the top or the bottom when none is selected), kept in view.
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        self.table.update(cx, |table, cx| {
            let rows = table.delegate().order.len();
            if rows == 0 {
                return;
            }
            let last = rows as isize - 1;
            let next = match table.selected_row() {
                None if delta < 0 => last,
                None => 0,
                Some(row) => (row as isize + delta).clamp(0, last),
            } as usize;
            table.set_selected_row(next, cx);
            table.scroll_to_row(next, cx);
        });
        cx.notify();
    }

    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn on_select_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MIN / 2, cx);
    }

    fn on_select_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(isize::MAX / 2, cx);
    }

    fn on_extract_all(&mut self, _: &ExtractAll, window: &mut Window, cx: &mut Context<Self>) {
        self.extract_all(window, cx);
    }

    fn on_extract_selected(&mut self, _: &ExtractSelected, window: &mut Window, cx: &mut Context<Self>) {
        self.extract_selected(window, cx);
    }
}

impl Focusable for ArchiveView {
    /// The rows take the keyboard while entries are listed (Enter opens, the arrows move), the password
    /// field while the archive is locked.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.state {
            State::Listed(_) => self.rows_focus.clone(),
            State::Locked => self.password_input.read(cx).focus_handle(cx),
            _ => self.focus_handle.clone(),
        }
    }
}

impl Render for ArchiveView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let summary = self.summary();
        let tool_label = self.tool_label();
        let message = self.message_text();
        let look_again = self.wants_look_again();
        let encrypted = self.asks_password();
        let listed = matches!(self.state, State::Listed(_));
        let status_row = h_flex()
            .id("archive-status")
            .test_support()
            .role(Role::Status)
            .aria_label(message.clone())
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
            .child(div().id("archive-summary").test_support().aria_label(summary.clone()).flex_shrink_0().child(summary))
            .when_some(tool_label, |row, label| {
                row.child(div().id("archive-tool").test_support().aria_label(label.clone()).flex_shrink_0().child(label))
            })
            .when(look_again, |row| {
                row.child(
                    div().id("archive-look-again").test_support().aria_label("Look again").flex_shrink_0().child(
                        Button::new("archive-look-again-button")
                            .label("Look again")
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| this.look_again(window, cx))),
                    ),
                )
            })
            .child(
                div()
                    .id("archive-message")
                    .test_support()
                    .aria_label(message.clone())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(message.clone()),
            );
        let tools_row = h_flex()
            .id("archive-tools")
            .flex_shrink_0()
            .gap_2()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(border)
            .child(div().w(px(260.)).child(Input::new(&self.search).id("archive-search")))
            .child(
                Button::new("archive-extract-all")
                    .label("Extract all")
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| this.extract_all(window, cx))),
            )
            .child(
                Button::new("archive-extract-selected")
                    .label("Extract selected")
                    .xsmall()
                    .on_click(cx.listener(|this, _, window, cx| this.extract_selected(window, cx))),
            )
            .when(encrypted, |row| {
                row.child(div().text_color(muted).text_sm().child("password:"))
                    .child(div().w(px(180.)).child(Input::new(&self.password_input).id("archive-password")))
                    .child(
                        Button::new("archive-unlock")
                            .label("Unlock")
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| this.apply_password(window, cx))),
                    )
            });
        let body = if listed {
            div()
                .id("archive-rows")
                .key_context(ROWS_CONTEXT)
                .track_focus(&self.rows_focus)
                .on_action(cx.listener(Self::on_open_entry))
                .on_action(cx.listener(Self::on_select_up))
                .on_action(cx.listener(Self::on_select_down))
                .on_action(cx.listener(Self::on_select_first))
                .on_action(cx.listener(Self::on_select_last))
                .size_full()
                .min_h_0()
                .child(self.table.clone())
                .into_any_element()
        } else {
            v_flex()
                .id("archive-empty")
                .size_full()
                .min_h_0()
                .child(div().id("archive-hint").test_support().aria_label(message.clone()).flex_shrink_0().px_3().py_2().text_sm().child(message))
                .when_some(self.hex_error.clone(), |col, error| col.child(div().px_3().text_sm().text_color(muted).child(error)))
                .when_some(self.hex.clone(), |col, hex| col.child(div().id("archive-hex").test_support().size_full().min_h_0().child(hex)))
                .into_any_element()
        };
        v_flex()
            .id("archive-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_extract_all))
            .on_action(cx.listener(Self::on_extract_selected))
            .size_full()
            .min_h_0()
            .child(tools_row)
            .child(body)
            .child(status_row)
    }
}
