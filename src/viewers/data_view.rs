//! The structured data viewer (C-010): a JSON, JSON Lines, XML, YAML, TOML or INI file as a
//! collapsible tree with key, type and value columns, a search field, the selected node's
//! path, and the raw text in the text viewer one chord away.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState, Position};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, IntoElement, MouseButton, Render, Role,
    ScrollStrategy, Subscription, TestSupportExt, UniformListScrollHandle, Window, actions,
    div, px, uniform_list,
};

use crate::data::{self, Format, ParseError, Tree};
use crate::viewers::rules_panel::RulesStore;
use crate::viewers::text_view::TextView;

actions!(
    data_view,
    [SelectUp, SelectDown, Collapse, Expand, ToggleNode, FocusSearch, CopyPath, PrettyPrint, ToggleRaw]
);

/// The key context of the whole view: the chords work from the search field and the raw text too.
pub const KEY_CONTEXT: &str = "DataView";
/// The key context of the rows: the arrows, Enter and Space act on the tree only while it has the focus.
pub const TREE_CONTEXT: &str = "DataTree";

/// How many characters of a value a row shows.
const VALUE_PREVIEW: usize = 200;

pub struct DataView {
    focus_handle: FocusHandle,
    /// The rows' own focus: the arrow keys move the selection only while it is held.
    rows_focus: FocusHandle,
    path: PathBuf,
    format: Format,
    rules: Entity<RulesStore>,
    /// The file's bytes, kept until the text viewer takes them.
    bytes: Vec<u8>,
    tree: Result<Tree, ParseError>,
    /// Per node, whether its children show.
    expanded: Vec<bool>,
    /// The nodes shown, one per row.
    visible: Vec<usize>,
    /// The selected row, an index into `visible`.
    selected: Option<usize>,
    search: Entity<InputState>,
    /// The nodes the last search matched.
    matches: Vec<usize>,
    text: Option<Entity<TextView>>,
    raw_shown: bool,
    status: String,
    scroll: UniformListScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl DataView {
    /// Parse `bytes` as the file at `path`. A file that does not parse opens on its raw
    /// text with the error in the status row and the cursor on the error's line.
    pub fn new(path: &Path, bytes: &[u8], rules: &Entity<RulesStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let format = Format::for_name(&name).unwrap_or(Format::Json);
        let decoded = crate::text::decode(bytes);
        let tree = Tree::parse(format, &decoded.text);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("find a key or value"));
        let on_search = cx.subscribe_in(&search, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let text = state.read(cx).value().to_string();
                this.find_next(&text, cx);
            }
        });
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            rows_focus: cx.focus_handle(),
            path: path.to_path_buf(),
            format,
            rules: rules.clone(),
            bytes: bytes.to_vec(),
            tree,
            expanded: Vec::new(),
            visible: Vec::new(),
            selected: None,
            search,
            matches: Vec::new(),
            text: None,
            raw_shown: false,
            status: String::new(),
            scroll: UniformListScrollHandle::new(),
            _subscriptions: vec![on_search],
        };
        view.reset_rows();
        if let Err(error) = &view.tree {
            let error = error.clone();
            view.status = error.to_string();
            view.show_raw(window, cx);
            view.place_cursor(&error, window, cx);
        }
        view
    }

    /// Open the roots, close everything else, and select the first row.
    fn reset_rows(&mut self) {
        match &self.tree {
            Ok(tree) => {
                self.expanded = tree.nodes.iter().map(|node| node.depth == 0 && node.kind.is_container()).collect();
                self.rebuild_visible();
                self.selected = (!self.visible.is_empty()).then_some(0);
            }
            Err(_) => {
                self.expanded.clear();
                self.visible.clear();
                self.selected = None;
            }
        }
        self.matches.clear();
    }

    /// The rows in pre-order, skipping the subtrees of closed nodes.
    fn rebuild_visible(&mut self) {
        self.visible.clear();
        let Ok(tree) = &self.tree else { return };
        let mut ix = 0;
        while ix < tree.len() {
            self.visible.push(ix);
            ix = if self.expanded[ix] { ix + 1 } else { tree.nodes[ix].end };
        }
    }

    fn row_of(&self, node: usize) -> Option<usize> {
        self.visible.iter().position(|&n| n == node)
    }

    /// The selected node, if any.
    pub fn selected_node(&self) -> Option<usize> {
        self.selected.and_then(|row| self.visible.get(row).copied())
    }

    /// The selected node's path in the file's notation.
    pub fn selected_path(&self) -> Option<String> {
        let tree = self.tree.as_ref().ok()?;
        self.selected_node().map(|node| tree.path(node))
    }

    /// The row at the top of the list: from the scroll offset and the row height the last
    /// layout measured (the handle's own top-index helper reads bounds a uniform list never records).
    pub fn top_row(&self) -> usize {
        let state = self.scroll.0.borrow();
        let rows = self.visible.len();
        let Some(size) = state.last_item_size else { return 0 };
        if rows == 0 {
            return 0;
        }
        let row_height = f32::from(size.contents.height) / rows as f32;
        let scrolled = -f32::from(state.base_handle.offset().y);
        ((scrolled / row_height).round().max(0.) as usize).min(rows - 1)
    }

    fn select_row(&mut self, row: usize, strategy: ScrollStrategy, cx: &mut Context<Self>) {
        if row < self.visible.len() {
            self.selected = Some(row);
            self.scroll.scroll_to_item(row, strategy);
            cx.notify();
        }
    }

    /// Open or close the container at `node`, keeping the selection on the same node.
    fn toggle(&mut self, node: usize, cx: &mut Context<Self>) {
        let Ok(tree) = &self.tree else { return };
        if !tree.nodes[node].kind.is_container() {
            return;
        }
        let selected_node = self.selected_node();
        self.expanded[node] = !self.expanded[node];
        self.rebuild_visible();
        self.selected = selected_node.and_then(|n| self.row_of(n)).or_else(|| self.row_of(node));
        cx.notify();
    }

    /// Open every ancestor of `node` so its row shows.
    fn reveal(&mut self, node: usize) {
        let Ok(tree) = &self.tree else { return };
        for ancestor in tree.ancestors(node) {
            self.expanded[ancestor] = true;
        }
        self.rebuild_visible();
    }

    /// Find the next node after the selection whose key or value contains `text`, wrapping
    /// around; the status row reports how many match.
    pub fn find_next(&mut self, text: &str, cx: &mut Context<Self>) {
        let Ok(tree) = &self.tree else { return };
        self.matches = tree.search(text);
        if self.matches.is_empty() {
            self.status = if text.is_empty() { "type something to find".to_string() } else { format!("no node matches \"{text}\"") };
            cx.notify();
            return;
        }
        let after = self.selected_node().unwrap_or(0);
        let position = self
            .matches
            .iter()
            .position(|&node| node > after)
            .unwrap_or(0);
        let node = self.matches[position];
        self.reveal(node);
        if let Some(row) = self.row_of(node) {
            self.select_row(row, ScrollStrategy::Center, cx);
        }
        self.status = format!("match {} of {}", position + 1, self.matches.len());
        cx.notify();
    }

    /// Build the text viewer on the file's bytes the first time it is needed.
    fn ensure_text(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TextView> {
        if let Some(text) = &self.text {
            return text.clone();
        }
        let text = cx.new(|cx| TextView::new(&self.path, &self.bytes, &self.rules, window, cx));
        self._subscriptions.push(cx.observe(&text, |_, _, cx| cx.notify()));
        self.text = Some(text.clone());
        text
    }

    fn show_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.ensure_text(window, cx);
        self.raw_shown = true;
        let focus = text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Put the editor's cursor on the error's line and column (1-based in the error).
    fn place_cursor(&mut self, error: &ParseError, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = &self.text else { return };
        let position = Position::new(error.line.saturating_sub(1) as u32, error.column.saturating_sub(1) as u32);
        let editor = text.read(cx).editor().clone();
        editor.update(cx, |editor, cx| editor.set_cursor_position(position, window, cx));
    }

    /// The current text: the editor's, once it exists, else the file's.
    fn current_text(&self, cx: &App) -> String {
        match &self.text {
            Some(text) => text.read(cx).text(cx).to_string(),
            None => crate::text::decode(&self.bytes).text,
        }
    }

    /// Parse the current text again and rebuild the rows.
    fn reparse(&mut self, cx: &App) {
        let text = self.current_text(cx);
        self.tree = Tree::parse(self.format, &text);
        self.reset_rows();
    }

    fn on_select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.selected.unwrap_or(0).saturating_sub(1);
        self.select_row(row, ScrollStrategy::Top, cx);
    }

    fn on_select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.selected.map_or(0, |r| r + 1).min(self.visible.len().saturating_sub(1));
        self.select_row(row, ScrollStrategy::Bottom, cx);
    }

    /// Left closes an open container, or moves to the parent of anything else.
    fn on_collapse(&mut self, _: &Collapse, _: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.selected_node() else { return };
        let Ok(tree) = &self.tree else { return };
        if tree.nodes[node].kind.is_container() && self.expanded[node] {
            self.toggle(node, cx);
        } else if let Some(parent) = tree.parent(node)
            && let Some(row) = self.row_of(parent)
        {
            self.select_row(row, ScrollStrategy::Top, cx);
        }
    }

    /// Right opens a closed container, or moves into an open one.
    fn on_expand(&mut self, _: &Expand, _: &mut Window, cx: &mut Context<Self>) {
        let Some(node) = self.selected_node() else { return };
        let Ok(tree) = &self.tree else { return };
        if !tree.nodes[node].kind.is_container() {
            return;
        }
        if !self.expanded[node] {
            self.toggle(node, cx);
        } else if let Some(row) = self.selected
            && row + 1 < self.visible.len()
            && tree.nodes[self.visible[row + 1]].depth > tree.nodes[node].depth
        {
            self.select_row(row + 1, ScrollStrategy::Bottom, cx);
        }
    }

    fn on_toggle_node(&mut self, _: &ToggleNode, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(node) = self.selected_node() {
            self.toggle(node, cx);
        }
    }

    fn on_focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_shown {
            self.show_tree(window, cx);
        }
        let handle = self.search.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    fn on_copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        match self.selected_path() {
            Some(path) => {
                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                self.status = format!("copied {path}");
            }
            None => self.status = "no node is selected".to_string(),
        }
        cx.notify();
    }

    /// Replace the raw text with its pretty form as one undoable edit, and rebuild the tree.
    /// A read-only text (units that did not decode, C-008) stays as it is.
    fn on_pretty_print(&mut self, _: &PrettyPrint, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.ensure_text(window, cx);
        if !view.read(cx).is_editable() {
            self.status = "the text is read-only (it has units that did not decode), so it stays as it is".to_string();
            cx.notify();
            return;
        }
        let text = self.current_text(cx);
        // The editor may hold the file's carriage returns; the pretty form is written with line feeds and
        // saved back with the file's endings (C-008), so the two are compared with the same endings.
        match data::pretty(self.format, &text) {
            Ok(pretty) if pretty == data::with_line_feeds(&text) => {
                self.status = "already in its pretty form".to_string();
            }
            Ok(pretty) => {
                let editor = view.read(cx).editor().clone();
                editor.update(cx, |editor, cx| editor.replace_all(pretty, window, cx));
                view.update(cx, |view, cx| view.note_edit(cx));
                self.reparse(cx);
                self.status = match &self.tree {
                    Ok(tree) => format!("pretty-printed, {} nodes", tree.len()),
                    Err(error) => error.to_string(),
                };
            }
            Err(error) => self.status = error.to_string(),
        }
        cx.notify();
    }

    /// Back to the tree, rebuilt from the text as edited.
    fn show_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reparse(cx);
        self.raw_shown = false;
        self.status = match &self.tree {
            Ok(_) => String::new(),
            Err(error) => error.to_string(),
        };
        window.focus(&self.rows_focus, cx);
        cx.notify();
    }

    fn on_toggle_raw(&mut self, _: &ToggleRaw, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_shown {
            self.show_tree(window, cx);
        } else {
            self.show_raw(window, cx);
        }
    }

    // ---- what the workspace and the tests read ----

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn tree(&self) -> Option<&Tree> {
        self.tree.as_ref().ok()
    }

    pub fn error(&self) -> Option<&ParseError> {
        self.tree.as_ref().err()
    }

    /// The nodes shown, one per row.
    pub fn visible_nodes(&self) -> &[usize] {
        &self.visible
    }

    pub fn is_expanded(&self, node: usize) -> bool {
        self.expanded.get(node).copied().unwrap_or(false)
    }

    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search
    }

    /// The text viewer, once the raw text or a pretty-print has been asked for.
    pub fn text_view(&self) -> Option<&Entity<TextView>> {
        self.text.as_ref()
    }

    pub fn raw_shown(&self) -> bool {
        self.raw_shown
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    /// Whether the raw text has edits not yet saved (C-008).
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.text.as_ref().is_some_and(|text| text.read(cx).is_dirty())
    }

    /// The row's depth and three columns, and whether the search marked it, as the test support reads them.
    fn row_label(tree: &Tree, node: usize, marked: bool) -> String {
        let n = &tree.nodes[node];
        let found = if marked { "|found" } else { "" };
        format!("{}|{}|{}|{}{found}", n.depth, n.key, n.kind.label(), preview(&n.value))
    }
}

/// A value cut to what a row can show.
fn preview(value: &str) -> String {
    let mut chars = value.chars();
    let short: String = chars.by_ref().take(VALUE_PREVIEW).collect();
    if chars.next().is_some() { format!("{short}…") } else { short }
}

impl Focusable for DataView {
    /// The dock focuses the rows through this, so the arrow keys work as soon as the tab shows.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match (&self.raw_shown, &self.text) {
            (true, Some(text)) => text.read(cx).editor().read(cx).focus_handle(cx),
            _ => self.rows_focus.clone(),
        }
    }
}

impl Render for DataView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let selected_bg = theme.list_active;
        let marked_bg = theme.primary;
        let marked_fg = theme.primary_foreground;
        let summary = match &self.tree {
            Ok(tree) => tree.summary(),
            Err(_) => format!("{}, does not parse", self.format.label()),
        };
        let path = self.selected_path().unwrap_or_default();
        let status = if self.status.is_empty() {
            if self.raw_shown {
                "Ctrl+Shift+D shows the tree, Ctrl+Shift+P pretty-prints".to_string()
            } else {
                "Enter opens and closes, Ctrl+Shift+F finds, Ctrl+Shift+C copies the path, Ctrl+Shift+P pretty-prints, Ctrl+Shift+D shows the text".to_string()
            }
        } else {
            self.status.clone()
        };
        let status_row = h_flex()
            .id("data-status")
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
            .child(div().id("data-summary").test_support().aria_label(summary.clone()).child(summary))
            .child(div().id("data-path").test_support().aria_label(path.clone()).child(path))
            .child(div().id("data-message").test_support().aria_label(status.clone()).flex_1().overflow_hidden().child(status));

        let raw_shown = self.raw_shown;
        let body = if raw_shown
            && let Some(text) = &self.text
        {
            div().id("data-raw").size_full().min_h_0().child(text.clone()).into_any_element()
        } else {
            match &self.tree {
                Err(error) => div()
                    .id("data-error")
                    .test_support()
                    .role(Role::Status)
                    .aria_label(error.to_string())
                    .p_4()
                    .child(format!("Cannot build the tree: {error}"))
                    .into_any_element(),
                Ok(_) => {
                    let selected = self.selected;
                    let rows = self.visible.len();
                    uniform_list(
                        "data-rows",
                        rows,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            let Ok(tree) = &this.tree else { return Vec::new() };
                            range
                                .map(|row| {
                                    let node = this.visible[row];
                                    let n = &tree.nodes[node];
                                    let container = n.kind.is_container();
                                    let open = this.expanded[node];
                                    let marked = this.matches.contains(&node);
                                    let glyph = if !container {
                                        ""
                                    } else if open {
                                        "▾"
                                    } else {
                                        "▸"
                                    };
                                    h_flex()
                                        .id(("data-row", row))
                                        .test_support()
                                        .role(Role::Row)
                                        .aria_label(Self::row_label(tree, node, marked))
                                        .w_full()
                                        .h(px(24.))
                                        .px_3()
                                        .gap_2()
                                        .whitespace_nowrap()
                                        .when(selected == Some(row), |row| row.bg(selected_bg))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            window.focus(&this.rows_focus, cx);
                                            this.select_row(row, ScrollStrategy::Top, cx);
                                            this.toggle(node, cx);
                                        }))
                                        // The key column holds the indent, the glyph and the key, so the type
                                        // and value columns line up at every depth; a mark hugs its text.
                                        .child(
                                            h_flex()
                                                .w(px(300.))
                                                .flex_shrink_0()
                                                .overflow_hidden()
                                                .child(div().w(px(16. * n.depth as f32)).flex_shrink_0())
                                                .child(
                                                    div()
                                                        .id(("data-glyph", row))
                                                        .test_support()
                                                        .w(px(16.))
                                                        .flex_shrink_0()
                                                        .text_color(muted)
                                                        .child(glyph)
                                                        .when(container, |glyph| {
                                                            glyph.on_mouse_down(
                                                                MouseButton::Left,
                                                                cx.listener(move |this, _, _, cx| {
                                                                    cx.stop_propagation();
                                                                    this.toggle(node, cx);
                                                                }),
                                                            )
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .px_1()
                                                        .rounded_sm()
                                                        .overflow_hidden()
                                                        .when(marked, |key| key.bg(marked_bg).text_color(marked_fg))
                                                        .child(n.key.clone()),
                                                ),
                                        )
                                        .child(div().w(px(90.)).flex_shrink_0().text_color(muted).child(n.kind.label()))
                                        .child(
                                            h_flex().flex_1().overflow_hidden().child(
                                                div()
                                                    .px_1()
                                                    .rounded_sm()
                                                    .overflow_hidden()
                                                    .when(marked, |value| value.bg(marked_bg).text_color(marked_fg))
                                                    .child(preview(&n.value)),
                                            ),
                                        )
                                })
                                .collect()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .size_full()
                    .py_1()
                    .into_any_element()
                }
            }
        };

        let toolbar = h_flex()
            .flex_shrink_0()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(div().w(px(300.)).child(Input::new(&self.search)))
            .child(Button::new("data-next").label("Next").on_click(cx.listener(|this, _, _, cx| {
                let text = this.search.read(cx).value().to_string();
                this.find_next(&text, cx);
            })))
            .child(div().text_color(muted).text_sm().child("Key · Type · Value"));

        v_flex()
            .id("data-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_copy_path))
            .on_action(cx.listener(Self::on_pretty_print))
            .on_action(cx.listener(Self::on_toggle_raw))
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .when(!self.raw_shown, |view| view.child(toolbar))
            .map(|view| {
                if raw_shown {
                    // The raw text lives outside the rows' key context: Space and the arrows are
                    // the editor's there.
                    view.child(div().id("data-raw-area").flex_1().min_h_0().size_full().child(body))
                } else {
                    view.child(
                        div()
                            .id("data-tree")
                            .test_support()
                            .key_context(TREE_CONTEXT)
                            .track_focus(&self.rows_focus)
                            .on_action(cx.listener(Self::on_select_up))
                            .on_action(cx.listener(Self::on_select_down))
                            .on_action(cx.listener(Self::on_collapse))
                            .on_action(cx.listener(Self::on_expand))
                            .on_action(cx.listener(Self::on_toggle_node))
                            .flex_1()
                            .min_h_0()
                            .size_full()
                            .child(body),
                    )
                }
            })
            .child(status_row)
    }
}
