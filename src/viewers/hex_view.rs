//! The hex view (C-007): a memory-mapped file as rows of offset, hex bytes and printable
//! ASCII, laid out only where visible, with go-to-offset and byte or text search.
//!
//! The list shows a window of at most [`WINDOW_ROWS`] rows around the current position.
//! Pixel arithmetic is 32-bit, so a list of every row of a multi-gigabyte file would
//! round its scroll positions by whole rows and could not reach its end; a window of a
//! million rows stays exact, and it slides when the view nears its edge.

use std::ops::Range;
use std::path::Path;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, Render, Role, ScrollStrategy,
    SharedString, Subscription, TestSupportExt, UniformListScrollHandle, Window, actions, div,
    px, uniform_list,
};

use crate::hex::{HexFile, SearchMode, parse_offset, row_of};

actions!(hex_view, [FocusGoto, FocusSearch, ToggleSearchMode]);

/// The key context the hex view's actions are bound in.
pub const KEY_CONTEXT: &str = "HexView";

/// The most rows the list holds at once.
pub const WINDOW_ROWS: usize = 1 << 20;

/// How close to the window's edge the top row may come before the window slides.
const WINDOW_EDGE: usize = 4096;

pub struct HexView {
    focus_handle: FocusHandle,
    file: HexFile,
    scroll: UniformListScrollHandle,
    /// The first row of the window the list shows.
    base_row: usize,
    goto: Entity<InputState>,
    search: Entity<InputState>,
    mode: SearchMode,
    mark: Option<Range<usize>>,
    last_hit: Option<usize>,
    status: String,
    _subscriptions: Vec<Subscription>,
}

impl HexView {
    /// Map the file at `path`.
    pub fn open(path: &Path, window: &mut Window, cx: &mut Context<Self>) -> std::io::Result<Self> {
        Ok(Self::new(HexFile::open(path)?, window, cx))
    }

    pub fn new(file: HexFile, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let goto = cx.new(|cx| InputState::new(window, cx).placeholder("offset, 0x for hex"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("find"));
        let on_goto = cx.subscribe_in(&goto, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = state.read(cx).value().to_string();
                this.go_to(&text, cx);
            }
        });
        let on_search = cx.subscribe_in(&search, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = state.read(cx).value().to_string();
                this.find_next(&text, cx);
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            file,
            scroll: UniformListScrollHandle::new(),
            base_row: 0,
            goto,
            search,
            mode: SearchMode::Text,
            mark: None,
            last_hit: None,
            status: String::new(),
            _subscriptions: vec![on_goto, on_search],
        }
    }

    pub fn file(&self) -> &HexFile {
        &self.file
    }

    /// The marked bytes: the byte gone to, or the last search hit.
    pub fn mark(&self) -> Option<&Range<usize>> {
        self.mark.as_ref()
    }

    /// The first row of the window the list holds.
    pub fn base_row(&self) -> usize {
        self.base_row
    }

    /// The rows the list holds: the whole file, or a window of it.
    fn window_rows(&self) -> usize {
        self.file.rows().saturating_sub(self.base_row).min(WINDOW_ROWS)
    }

    /// The row at the top of the list, in the file.
    pub fn top_row(&self) -> usize {
        self.base_row + self.top_row_in_window()
    }

    /// The row at the top of the list, in the window: from the scroll offset and the row
    /// height the last layout measured (the handle's own top-index helper reads child
    /// bounds that a uniform list never records).
    fn top_row_in_window(&self) -> usize {
        let state = self.scroll.0.borrow();
        let (Some(size), rows) = (state.last_item_size, self.window_rows()) else {
            return 0;
        };
        if rows == 0 {
            return 0;
        }
        let row_height = f32::from(size.contents.height) / rows as f32;
        let scrolled = -f32::from(state.base_handle.offset().y);
        ((scrolled / row_height).round().max(0.) as usize).min(rows - 1)
    }

    pub fn search_mode(&self) -> SearchMode {
        self.mode
    }

    /// What the status row says about the last go-to or search.
    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn goto_input(&self) -> &Entity<InputState> {
        &self.goto
    }

    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    /// Put the window around `row` when `row` is outside it or near its edge, and scroll
    /// `row` into place. A window that moved loses "nearest", so the row is centred then.
    fn scroll_to_row(&mut self, row: usize, mut strategy: ScrollStrategy) {
        let rows = self.file.rows();
        if rows > WINDOW_ROWS {
            // The row must sit inside the window, clear of its edges, or the window moves.
            let window = self.base_row..self.base_row + self.window_rows();
            let clear_of_edges = (window.start == 0 || row >= window.start + WINDOW_EDGE)
                && (window.end == rows || row + WINDOW_EDGE <= window.end);
            if !clear_of_edges {
                self.base_row = row.saturating_sub(WINDOW_ROWS / 2).min(rows - WINDOW_ROWS);
                if strategy == ScrollStrategy::Nearest {
                    strategy = ScrollStrategy::Center;
                }
            }
        }
        self.scroll.scroll_to_item(row - self.base_row, strategy);
    }

    /// Scroll `row` to the top of the view.
    pub fn show_row_at_top(&mut self, row: usize, cx: &mut Context<Self>) {
        let row = row.min(self.file.rows().saturating_sub(1));
        self.scroll_to_row(row, ScrollStrategy::Top);
        cx.notify();
    }

    /// Scroll by `delta` rows (down when positive), as the wheel does, within the window;
    /// the window slides at the next layout when that brings the top row near its edge.
    pub fn scroll_by_rows(&mut self, delta: isize, cx: &mut Context<Self>) {
        let state = self.scroll.0.borrow();
        let (Some(size), rows) = (state.last_item_size, self.window_rows()) else {
            return;
        };
        if rows == 0 {
            return;
        }
        let row_height = f32::from(size.contents.height) / rows as f32;
        let mut offset = state.base_handle.offset();
        offset.y -= px(row_height * delta as f32);
        let lowest = px(-(f32::from(size.contents.height) - f32::from(size.item.height)).max(0.));
        offset.y = offset.y.max(lowest).min(px(0.));
        state.base_handle.set_offset(offset);
        drop(state);
        cx.notify();
    }

    /// Slide the window when the top row has come within the edge of it, keeping the top
    /// row where it is on screen. Called before each layout.
    fn slide_window_if_near_edge(&mut self) {
        let rows = self.file.rows();
        // A scroll still to be applied makes the current offset stale; judge after it.
        if rows <= WINDOW_ROWS || self.scroll.0.borrow().deferred_scroll_to_item.is_some() {
            return;
        }
        let top = self.top_row();
        let near_start = self.base_row > 0 && top < self.base_row + WINDOW_EDGE;
        let near_end = self.base_row + WINDOW_ROWS < rows && top + WINDOW_EDGE > self.base_row + WINDOW_ROWS;
        if near_start || near_end {
            self.base_row = top.saturating_sub(WINDOW_ROWS / 2).min(rows - WINDOW_ROWS);
            self.scroll.scroll_to_item(top - self.base_row, ScrollStrategy::Top);
        }
    }

    /// Scroll the row holding the offset in `text` to the top and mark the byte; an offset
    /// past the end marks the last byte.
    pub fn go_to(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_offset(text) {
            Err(error) => self.status = error,
            Ok(_) if self.file.is_empty() => self.status = "the file is empty".into(),
            Ok(offset) => {
                let offset = offset.min(self.file.len() - 1);
                self.mark = Some(offset..offset + 1);
                self.scroll_to_row(row_of(offset), ScrollStrategy::Top);
                self.status = format!("at 0x{offset:x} ({offset})");
            }
        }
        cx.notify();
    }

    /// Find the next match of `text` (in the current mode) after the previous hit, from the
    /// start on the first search or after a miss; scroll it into view and mark it.
    pub fn find_next(&mut self, text: &str, cx: &mut Context<Self>) {
        let needle = match self.mode.needle(text) {
            Ok(needle) => needle,
            Err(error) => {
                self.status = error;
                cx.notify();
                return;
            }
        };
        let from = self.last_hit.map_or(0, |hit| hit + 1);
        match self.file.find(&needle, from) {
            Some(at) => {
                self.mark = Some(at..at + needle.len());
                self.last_hit = Some(at);
                self.scroll_to_row(row_of(at), ScrollStrategy::Nearest);
                self.status = format!("hit at 0x{at:x} ({at})");
            }
            None => {
                self.last_hit = None;
                self.status = if from == 0 {
                    "not found".to_string()
                } else {
                    format!("no more hits after 0x{:x}; Enter searches from the start", from - 1)
                };
            }
        }
        cx.notify();
    }

    pub fn set_search_mode(&mut self, mode: SearchMode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            self.last_hit = None;
            cx.notify();
        }
    }

    fn on_focus_goto(&mut self, _: &FocusGoto, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.goto.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.search.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn on_toggle_mode(&mut self, _: &ToggleSearchMode, _: &mut Window, cx: &mut Context<Self>) {
        self.set_search_mode(self.mode.other(), cx);
    }
}

impl Focusable for HexView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

fn status_item(id: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    let value = value.into();
    div()
        .id(id)
        .role(Role::Status)
        .test_support()
        .aria_label(value.clone())
        .child(value)
}

impl Render for HexView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.slide_window_if_near_edge();
        let theme = cx.theme();
        let mono = theme.mono_font_family.clone();
        let mono_size = theme.mono_font_size;
        let line_height = window.line_height();
        let marked_bg = theme.accent;
        let muted = theme.muted_foreground;
        let rows = self.file.rows();
        let base_row = self.base_row;
        let mark_label = self
            .mark
            .as_ref()
            .map_or_else(|| "no mark".to_string(), |m| format!("mark 0x{:x}..0x{:x}", m.start, m.end));
        let status = if self.status.is_empty() {
            "Ctrl+Shift+G goes to an offset, Ctrl+Shift+F finds, Ctrl+M switches text and bytes".to_string()
        } else {
            self.status.clone()
        };
        v_flex()
            .id("hex-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_focus_goto))
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_toggle_mode))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().w(px(200.)).child(Input::new(&self.goto)))
                    .child(
                        Button::new("hex-go")
                            .label("Go")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let text = this.goto.read(cx).value().to_string();
                                this.go_to(&text, cx);
                            })),
                    )
                    .child(div().w(px(260.)).child(Input::new(&self.search)))
                    .child(
                        Button::new("hex-mode")
                            .label(self.mode.label())
                            .on_click(cx.listener(|this, _, _, cx| this.set_search_mode(this.mode.other(), cx))),
                    )
                    .child(
                        Button::new("hex-next")
                            .label("Next")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let text = this.search.read(cx).value().to_string();
                                this.find_next(&text, cx);
                            })),
                    ),
            )
            .child(
                uniform_list(
                    "hex-rows",
                    self.window_rows(),
                    cx.processor(move |this, range: Range<usize>, _window, _cx| {
                        range
                            .map(|ix| {
                                let row_index = base_row + ix;
                                let row = this.file.row(row_index, this.mark.as_ref());
                                let line = row.line();
                                let [hex_before, hex_marked, hex_after] = row.hex;
                                let [ascii_before, ascii_marked, ascii_after] = row.ascii;
                                h_flex()
                                    .id(("hex-row", row_index))
                                    .test_support()
                                    .role(Role::Row)
                                    .aria_label(line)
                                    .h(line_height)
                                    .px_3()
                                    .font_family(mono.clone())
                                    .text_size(mono_size)
                                    .whitespace_nowrap()
                                    .child(div().text_color(muted).child(row.offset))
                                    .child(div().child("  "))
                                    .child(div().child(hex_before))
                                    .child(div().bg(marked_bg).child(hex_marked))
                                    .child(div().child(hex_after))
                                    .child(div().child("  |"))
                                    .child(div().child(ascii_before))
                                    .child(div().bg(marked_bg).child(ascii_marked))
                                    .child(div().child(ascii_after))
                                    .child(div().child("|"))
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.scroll)
                .size_full()
                .py_1(),
            )
            .child(
                h_flex()
                    .id("hex-status")
                    .test_support()
                    .flex_shrink_0()
                    .gap_4()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_sm()
                    .text_color(muted)
                    .child(status_item("hex-size", format!("{} bytes", self.file.len())))
                    .child(status_item("hex-rows", format!("{rows} rows")))
                    .child(status_item("hex-mark", mark_label))
                    .child(status_item("hex-search", format!("{} search", self.mode.label())))
                    .child(status_item("hex-status-text", status)),
            )
    }
}
