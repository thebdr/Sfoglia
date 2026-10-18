//! The main window's content: open files as tabs in a dock (C-004), the application
//! name when nothing is open (C-001), the highlighting rules panel on the right (C-006)
//! and the hex view any tab can switch to (C-007).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode, WindowExt as _};
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockPlacement, DockSkin, PaneNode, PaneRef, Panel as KitPanel,
    PanelEvent, PanelId, panel_handle,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable, IntoElement,
    KeyBinding, PathPromptOptions, Render, Role, SharedString, Subscription, TestSupportExt,
    Window, actions, div, px,
};

use crate::detect::{Content, Detection, Route, SNIFF_BYTES, detect};
use crate::generated::families::Family;
use crate::hex::HexFile;
use crate::rules;
use crate::shell::{APP_NAME, title};
use crate::tools::ToolSet;
use crate::viewers::data_view::{self, Collapse, CopyPath, Expand, PrettyPrint, SelectDown, SelectUp, ToggleNode, ToggleRaw};
use crate::viewers::hex_view::{self, FocusGoto, FocusSearch, ToggleSearchMode};
use crate::viewers::image_view::{self, RotateQuarter, ZoomActual, ZoomFit, ZoomIn, ZoomOut};
use crate::viewers::markdown_view::{self, OpenLinkedFile};
use crate::viewers::pdf_view::{self, PdfView};
use crate::viewers::sheet_view;
use crate::viewers::{DataView, HexView, ImageView, MarkdownView, RulesPanel, RulesStore, SheetView, TextView};

actions!(sfoglia, [OpenFiles, ToggleTheme, ToggleHighlightRules, ToggleHexView, SaveFile, SaveImageAs]);

/// Key bindings and other once-per-application setup. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    // GPUI ships no HTTP client: without one every remote image in a Markdown file fails at once (C-011).
    cx.set_http_client(crate::net::UreqClient::shared());
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenFiles, Some("Workspace")),
        KeyBinding::new("ctrl-shift-t", ToggleTheme, Some("Workspace")),
        KeyBinding::new("ctrl-shift-h", ToggleHighlightRules, Some("Workspace")),
        // The kit's inputs bind ctrl-f, ctrl-g and ctrl-h themselves, so the hex chords
        // take the shift and stay reachable from inside its fields.
        KeyBinding::new("ctrl-shift-b", ToggleHexView, Some("Workspace")),
        KeyBinding::new("ctrl-s", SaveFile, Some("Workspace")),
        KeyBinding::new("ctrl-shift-s", SaveImageAs, Some("Workspace")),
        KeyBinding::new("+", ZoomIn, Some(image_view::KEY_CONTEXT)),
        KeyBinding::new("=", ZoomIn, Some(image_view::KEY_CONTEXT)),
        KeyBinding::new("-", ZoomOut, Some(image_view::KEY_CONTEXT)),
        KeyBinding::new("1", ZoomActual, Some(image_view::KEY_CONTEXT)),
        KeyBinding::new("0", ZoomFit, Some(image_view::KEY_CONTEXT)),
        KeyBinding::new("r", RotateQuarter, Some(image_view::KEY_CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(data_view::TREE_CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(data_view::TREE_CONTEXT)),
        KeyBinding::new("left", Collapse, Some(data_view::TREE_CONTEXT)),
        KeyBinding::new("right", Expand, Some(data_view::TREE_CONTEXT)),
        KeyBinding::new("enter", ToggleNode, Some(data_view::TREE_CONTEXT)),
        KeyBinding::new("space", ToggleNode, Some(data_view::TREE_CONTEXT)),
        KeyBinding::new("ctrl-shift-f", data_view::FocusSearch, Some(data_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-c", CopyPath, Some(data_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-p", PrettyPrint, Some(data_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-d", ToggleRaw, Some(data_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-d", markdown_view::ToggleRaw, Some(markdown_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-d", sheet_view::ToggleRaw, Some(sheet_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-f", sheet_view::FocusSearch, Some(sheet_view::KEY_CONTEXT)),
        KeyBinding::new("+", pdf_view::ZoomIn, Some(pdf_view::KEY_CONTEXT)),
        KeyBinding::new("=", pdf_view::ZoomIn, Some(pdf_view::KEY_CONTEXT)),
        KeyBinding::new("-", pdf_view::ZoomOut, Some(pdf_view::KEY_CONTEXT)),
        KeyBinding::new("0", pdf_view::ZoomFit, Some(pdf_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-g", FocusGoto, Some(hex_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-f", FocusSearch, Some(hex_view::KEY_CONTEXT)),
        KeyBinding::new("ctrl-m", ToggleSearchMode, Some(hex_view::KEY_CONTEXT)),
    ]);
}

/// What is known about an open file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: Option<u64>,
    pub detection: Detection,
    /// Set when the file could not be read; the tab still opens and says so.
    pub error: Option<String>,
}

impl FileInfo {
    /// Read the leading bytes and the size of `path`, then identify it.
    pub fn probe(path: &Path) -> FileInfo {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut head = Vec::new();
        let mut error = None;
        let size = match std::fs::File::open(path) {
            Ok(mut file) => {
                if let Err(e) = (&mut file).take(SNIFF_BYTES as u64).read_to_end(&mut head) {
                    error = Some(e.to_string());
                }
                file.metadata().ok().map(|m| m.len())
            }
            Err(e) => {
                error = Some(e.to_string());
                None
            }
        };
        FileInfo {
            path: path.to_path_buf(),
            detection: detect(&name, &head),
            name,
            size,
            error,
        }
    }

    pub fn route(&self) -> Route {
        self.detection.route
    }
}

/// What a tab shows for its file when it is not showing the hex view.
enum PanelView {
    /// The facts about the file, until its family's viewer exists.
    Info,
    /// The text viewer (C-005).
    Text(Entity<TextView>),
    /// The image viewer (C-009).
    Image(Entity<ImageView>),
    /// The structured data viewer (C-010).
    Data(Entity<DataView>),
    /// The Markdown viewer (C-011).
    Markdown(Entity<MarkdownView>),
    /// The spreadsheet viewer (C-013).
    Sheet(Entity<SheetView>),
    /// A PDF or XPS through SumatraPDF or a page renderer (C-014).
    Pdf(Entity<PdfView>),
}

/// One open file: a tab in the centre of the dock.
pub struct FilePanel {
    focus_handle: FocusHandle,
    info: FileInfo,
    view: PanelView,
    /// The hex view (C-007), made the first time it is shown.
    hex: Option<Entity<HexView>>,
    hex_shown: bool,
    hex_error: Option<String>,
    /// Whether the dock shows this tab (its hosted window follows, C-014).
    active: bool,
    /// Keeps the tab in step with the text view's modified state (C-008).
    _observe_text: Option<Subscription>,
}

impl FilePanel {
    pub fn new(
        info: FileInfo,
        rules: &Entity<RulesStore>,
        tools: &Arc<ToolSet>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A family with a viewer of its own comes first (an SVG is text by content, yet an
        // image); text content falls back to the text viewer while its family has none.
        let view = if info.error.is_none() && info.route() == Route::Family(Family::Images) {
            match std::fs::read(&info.path) {
                Ok(bytes) => PanelView::Image(cx.new(|cx| ImageView::new(&info.path, &bytes, window, cx))),
                Err(_) => PanelView::Info,
            }
        } else if info.error.is_none() && info.route() == Route::Family(Family::DataTrees) {
            match std::fs::read(&info.path) {
                Ok(bytes) => PanelView::Data(cx.new(|cx| DataView::new(&info.path, &bytes, rules, window, cx))),
                Err(_) => PanelView::Info,
            }
        } else if info.error.is_none() && info.route() == Route::Family(Family::Markdown) {
            match std::fs::read(&info.path) {
                Ok(bytes) => PanelView::Markdown(cx.new(|cx| MarkdownView::new(&info.path, &bytes, rules, window, cx))),
                Err(_) => PanelView::Info,
            }
        } else if info.error.is_none() && info.route() == Route::Family(Family::Spreadsheets) {
            match std::fs::read(&info.path) {
                Ok(bytes) => PanelView::Sheet(cx.new(|cx| SheetView::new(&info.path, &bytes, rules, window, cx))),
                Err(_) => PanelView::Info,
            }
        } else if info.error.is_none() && info.route() == Route::Family(Family::PdfXps) {
            PanelView::Pdf(cx.new(|cx| {
                let kind = pdf_view::kind_for(&info.path, &info.detection.content);
                PdfView::new(&info.path, kind, tools.clone(), window, cx)
            }))
        } else if Self::opens_as_text(&info) {
            match std::fs::read(&info.path) {
                Ok(bytes) => PanelView::Text(
                    cx.new(|cx| TextView::new(&info.path, &bytes, rules, window, cx)),
                ),
                Err(_) => PanelView::Info,
            }
        } else {
            PanelView::Info
        };
        // The tab's name and its close permission follow the text view's modified state.
        let observe_text = match &view {
            PanelView::Text(text) => Some(cx.observe(text, |_, _, cx| {
                cx.emit(PanelEvent::LayoutChanged);
                cx.notify();
            })),
            PanelView::Data(data) => Some(cx.observe(data, |_, _, cx| {
                cx.emit(PanelEvent::LayoutChanged);
                cx.notify();
            })),
            PanelView::Markdown(view) => Some(cx.observe(view, |_, _, cx| {
                cx.emit(PanelEvent::LayoutChanged);
                cx.notify();
            })),
            PanelView::Sheet(view) => Some(cx.observe(view, |_, _, cx| {
                cx.emit(PanelEvent::LayoutChanged);
                cx.notify();
            })),
            PanelView::Image(_) | PanelView::Pdf(_) | PanelView::Info => None,
        };
        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            info,
            view,
            hex: None,
            hex_shown: false,
            hex_error: None,
            active: true,
            _observe_text: observe_text,
        };
        // A binary nobody claims opens as hex straight away.
        if panel.info.error.is_none() && panel.info.route() == Route::Hex {
            panel.show_hex(window, cx);
        }
        // An image view takes the focus as soon as its tab exists, so its keys work at once;
        // the dock itself focuses a panel only when a tab is selected later.
        match &panel.view {
            PanelView::Image(view) => {
                let focus = view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            PanelView::Data(view) => {
                let focus = view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            PanelView::Markdown(view) => {
                let focus = view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            PanelView::Sheet(view) => {
                let focus = view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            PanelView::Pdf(view) => {
                let focus = view.read(cx).focus_handle(cx);
                window.focus(&focus, cx);
            }
            PanelView::Text(_) | PanelView::Info => {}
        }
        panel
    }

    /// A readable file whose content is text, or that is routed to the text family, opens
    /// in the text viewer; a family with a viewer of its own will take over later.
    fn opens_as_text(info: &FileInfo) -> bool {
        info.error.is_none()
            && (info.route() == Route::Family(Family::Text)
                || matches!(info.detection.content, Content::Text(_)))
    }

    pub fn info(&self) -> &FileInfo {
        &self.info
    }

    /// The text viewer, when this tab shows one.
    pub fn text_view(&self) -> Option<&Entity<TextView>> {
        match &self.view {
            PanelView::Text(view) if !self.hex_shown => Some(view),
            _ => None,
        }
    }

    /// The image viewer, when this tab shows one.
    pub fn image_view(&self) -> Option<&Entity<ImageView>> {
        match &self.view {
            PanelView::Image(view) if !self.hex_shown => Some(view),
            _ => None,
        }
    }

    /// The data viewer, when this tab shows one.
    pub fn data_view(&self) -> Option<&Entity<DataView>> {
        match &self.view {
            PanelView::Data(view) if !self.hex_shown => Some(view),
            _ => None,
        }
    }

    /// The Markdown viewer, when this tab shows one.
    pub fn markdown_view(&self) -> Option<&Entity<MarkdownView>> {
        match &self.view {
            PanelView::Markdown(view) if !self.hex_shown => Some(view),
            _ => None,
        }
    }

    /// The spreadsheet viewer, while the grid shows (C-013).
    pub fn sheet_view(&self) -> Option<&Entity<SheetView>> {
        match &self.view {
            PanelView::Sheet(view) if !self.hex_shown => Some(view),
            _ => None,
        }
    }

    /// The PDF viewer, while it shows (C-014).
    pub fn pdf_view(&self) -> Option<&Entity<PdfView>> {
        match &self.view {
            PanelView::Pdf(view) if !self.hex_shown => Some(view),
            _ => None,
        }
    }

    /// Whether the dock shows this tab (the active tab of its group).
    pub fn shown_by_dock(&self) -> bool {
        self.active
    }

    /// The workspace says on every render whether the dock shows this tab; the kit's dock calls no
    /// `set_active` of its own, so this is how a hosted window learns its tab went behind another.
    pub fn set_shown_by_dock(&mut self, shown: bool, cx: &mut Context<Self>) {
        if self.active != shown {
            self.active = shown;
            self.sync_hosted(cx);
        }
    }

    /// What a tab hosts outside GPUI (the SumatraPDF frame, C-014) shows only while the tab is active and
    /// the hex view is off.
    fn sync_hosted(&self, cx: &mut App) {
        if let PanelView::Pdf(view) = &self.view {
            let shown = self.active && !self.hex_shown;
            view.update(cx, |view, cx| view.set_shown(shown, cx));
        }
    }

    /// The text that Ctrl+S saves: the text viewer, or the data viewer's raw text once it exists.
    pub fn editable_text(&self, cx: &App) -> Option<Entity<TextView>> {
        match &self.view {
            PanelView::Text(view) => Some(view.clone()),
            PanelView::Data(view) => view.read(cx).text_view().cloned(),
            PanelView::Markdown(view) => Some(view.read(cx).text_view().clone()),
            PanelView::Sheet(view) => view.read(cx).text_view().cloned(),
            PanelView::Image(_) | PanelView::Pdf(_) | PanelView::Info => None,
        }
    }

    /// The hex view, when this tab shows one.
    pub fn hex_view(&self) -> Option<&Entity<HexView>> {
        if self.hex_shown { self.hex.as_ref() } else { None }
    }

    /// Why the hex view could not be opened, if it could not.
    pub fn hex_error(&self) -> Option<&str> {
        self.hex_error.as_deref()
    }

    /// Show the hex view, mapping the file the first time.
    pub fn show_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hex.is_none() {
            match HexFile::open(&self.info.path) {
                Ok(file) => self.hex = Some(cx.new(|cx| HexView::new(file, window, cx))),
                Err(error) => {
                    self.hex_error = Some(format!("Cannot map this file: {error}"));
                    cx.notify();
                    return;
                }
            }
        }
        self.hex_shown = true;
        self.sync_hosted(cx);
        if let Some(hex) = &self.hex {
            let focus = hex.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    /// Switch between the hex view and the file's own view.
    pub fn toggle_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hex_shown {
            self.hex_shown = false;
            self.sync_hosted(cx);
            let focus = self.focus_handle(cx);
            window.focus(&focus, cx);
            cx.notify();
        } else {
            self.show_hex(window, cx);
        }
    }
}

impl FilePanel {
    /// Whether the file has edits not yet saved (C-008).
    pub fn is_dirty(&self, cx: &App) -> bool {
        match &self.view {
            PanelView::Text(text) => text.read(cx).is_dirty(),
            PanelView::Data(data) => data.read(cx).is_dirty(cx),
            PanelView::Markdown(view) => view.read(cx).is_dirty(cx),
            PanelView::Sheet(view) => view.read(cx).is_dirty(cx),
            PanelView::Image(_) | PanelView::Pdf(_) | PanelView::Info => false,
        }
    }

    /// The tab's name: the file name, with a dot while it has unsaved edits.
    fn tab_label(&self, cx: &App) -> String {
        if self.is_dirty(cx) {
            format!("{} •", self.info.name)
        } else {
            self.info.name.clone()
        }
    }
}

impl BasePanel for FilePanel {
    fn panel_name(&self) -> &'static str {
        "sfoglia-file"
    }

    /// The dock says when the tab shows; a hosted window follows.
    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        self.set_shown_by_dock(active, cx);
    }

    /// A modified tab stays open until it is saved or reverted.
    fn closable(&self, cx: &App) -> bool {
        !self.is_dirty(cx)
    }
}

impl KitPanel for FilePanel {
    fn tab_name(&self, cx: &App) -> Option<SharedString> {
        Some(self.tab_label(cx).into())
    }

    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        SharedString::from(self.tab_label(cx))
    }
}

/// Whether two paths name the same file on disk, however they are spelled.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
        || match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
}

impl EventEmitter<PanelEvent> for FilePanel {}

impl Focusable for FilePanel {
    /// The dock focuses the active panel through this: an image tab hands the focus to its
    /// view, so the zoom and rotation keys work as soon as the tab shows.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            PanelView::Image(view) if !self.hex_shown => view.read(cx).focus_handle(cx),
            PanelView::Data(view) if !self.hex_shown => view.read(cx).focus_handle(cx),
            PanelView::Markdown(view) if !self.hex_shown => view.read(cx).focus_handle(cx),
            PanelView::Sheet(view) if !self.hex_shown => view.read(cx).focus_handle(cx),
            PanelView::Pdf(view) if !self.hex_shown => view.read(cx).focus_handle(cx),
            _ => self.focus_handle.clone(),
        }
    }
}

fn row(id: &'static str, label: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    let value = value.into();
    div()
        .flex()
        .gap_3()
        .child(div().w_32().flex_shrink_0().opacity(0.7).child(label))
        .child(
            div()
                .id(id)
                .role(Role::Label)
                .test_support()
                .aria_label(value.clone())
                .child(value),
        )
}

impl Render for FilePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(hex) = self.hex_view().cloned() {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(hex)
                .into_any_element();
        }
        if let PanelView::Text(view) = &self.view {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(view.clone())
                .into_any_element();
        }
        if let PanelView::Image(view) = &self.view {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(view.clone())
                .into_any_element();
        }
        if let PanelView::Data(view) = &self.view {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(view.clone())
                .into_any_element();
        }
        if let PanelView::Markdown(view) = &self.view {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(view.clone())
                .into_any_element();
        }
        if let PanelView::Sheet(view) = &self.view {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(view.clone())
                .into_any_element();
        }
        if let PanelView::Pdf(view) = &self.view {
            return div()
                .id("file-panel")
                .track_focus(&self.focus_handle)
                .size_full()
                .child(view.clone())
                .into_any_element();
        }
        let info = &self.info;
        let route = info.route();
        let family = info
            .detection
            .family
            .map_or_else(|| "none".to_string(), |f| format!("{} ({})", f.title(), f.slug()));
        let size = info
            .size
            .map_or_else(|| "unknown".to_string(), |s| format!("{s} bytes"));
        let status = match (&info.error, &self.hex_error) {
            (Some(error), _) => format!("Cannot read this file: {error}"),
            (None, Some(error)) => error.clone(),
            (None, None) => format!(
                "The {} viewer arrives with clause {}. Until then this tab shows what Sfoglia knows; Ctrl+Shift+B shows the bytes.",
                route.id(),
                route.clause()
            ),
        };
        div()
            .id("file-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(row("file-name", "File", info.name.clone()))
            .child(row("file-path", "Path", info.path.display().to_string()))
            .child(row("file-size", "Size", size))
            .child(row("file-family", "Family", family))
            .child(row("file-viewer", "Viewer", route.id()))
            .child(row("file-content", "Content", format!("{:?}", info.detection.content)))
            .child(div().id("file-status").role(Role::Status).test_support().aria_label(status.clone()).child(status))
            .into_any_element()
    }
}

/// The width the rules panel opens at.
const RULES_PANEL_WIDTH: f32 = 460.;

/// The main window's root view.
pub struct Workspace {
    focus_handle: FocusHandle,
    dock_area: Entity<DockArea>,
    _skin: Rc<DockSkin>,
    open: Vec<OpenFile>,
    rules: Entity<RulesStore>,
    rules_panel: Entity<RulesPanel>,
    /// The tools outside Sfoglia, as found on this machine or pinned by a test (C-014).
    tools: Arc<ToolSet>,
    /// The close request being answered (C-012), while the dialog shows.
    closing: Option<Closing>,
}

struct OpenFile {
    key: PathBuf,
    panel: Entity<FilePanel>,
}

impl Workspace {
    /// The workspace with the rule file from the user's configuration directory.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_rules_path(rules::default_path(), window, cx)
    }

    /// The workspace with the rule file at `rules_path` (`None`: rules are not saved).
    pub fn with_rules_path(
        rules_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_tools(rules_path, Arc::new(ToolSet::from_env()), window, cx)
    }

    /// The workspace with the rule file at `rules_path` and the tools outside Sfoglia as `tools` finds them.
    pub fn with_tools(
        rules_path: Option<PathBuf>,
        tools: Arc<ToolSet>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (dock_area, skin) = DockSkin::dock_area("sfoglia-main", Some(1), window, cx);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        // The window asks about modified tabs before it closes (C-012); the handler outlives nothing it
        // should not, holding the workspace weakly.
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |workspace, cx| workspace.may_close(window, cx)).unwrap_or(true)
        });
        let rules = cx.new(|_| RulesStore::load(rules_path));
        let rules_panel = cx.new(|cx| RulesPanel::new(&rules, window, cx));
        // The rules panel lives in the right dock, closed until Ctrl+Shift+H.
        dock_area.update(cx, |dock, cx| {
            dock.add_panel_view(
                panel_handle(rules_panel.clone()),
                DockPlacement::Right,
                Some(px(RULES_PANEL_WIDTH)),
                window,
                cx,
            );
            if dock.is_dock_open(DockPlacement::Right) {
                dock.toggle_dock(DockPlacement::Right, window, cx);
            }
        });
        Self {
            focus_handle,
            dock_area,
            _skin: skin,
            open: Vec::new(),
            rules,
            rules_panel,
            tools,
            closing: None,
        }
    }

    pub fn rules(&self) -> &Entity<RulesStore> {
        &self.rules
    }

    pub fn rules_panel(&self) -> &Entity<RulesPanel> {
        &self.rules_panel
    }

    pub fn tools(&self) -> &Arc<ToolSet> {
        &self.tools
    }

    /// The dock the tabs live in (tests move tabs through it).
    /// The workspace's own focus handle, the window's root focus (tests put the focus there to stand for
    /// "somewhere outside a view").
    pub fn own_focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Whether the rules panel is on screen.
    pub fn rules_panel_open(&self, cx: &App) -> bool {
        self.dock_area.read(cx).is_dock_open(DockPlacement::Right)
    }

    /// Whether the dock still holds this file's tab (the user may have closed it).
    fn still_open(&self, file: &OpenFile, cx: &App) -> bool {
        self.dock_area
            .read(cx)
            .panel(PanelId::from(file.panel.entity_id()))
            .is_some()
    }

    /// The panels of the files that are open, in the order they were opened.
    pub fn panels(&self, cx: &App) -> Vec<Entity<FilePanel>> {
        self.open
            .iter()
            .filter(|f| self.still_open(f, cx))
            .map(|f| f.panel.clone())
            .collect()
    }

    /// The open files, in the order they were opened. A file whose tab was closed is
    /// not open.
    pub fn open_files(&self, cx: &App) -> Vec<FileInfo> {
        self.panels(cx)
            .iter()
            .map(|panel| panel.read(cx).info.clone())
            .collect()
    }

    /// The panel whose tab is active in the centre, if any.
    pub fn active_panel(&self, cx: &App) -> Option<Entity<FilePanel>> {
        fn active_in(node: &PaneNode) -> Option<PanelId> {
            match node.kind() {
                PaneRef::Tabs { panels, active_ix } => panels.get(active_ix).copied(),
                PaneRef::Split { children, .. } => children.iter().find_map(active_in),
            }
        }
        let dock = self.dock_area.read(cx);
        let active = active_in(dock.layout(DockPlacement::Center)?.root())?;
        self.open
            .iter()
            .find(|f| PanelId::from(f.panel.entity_id()) == active)
            .map(|f| f.panel.clone())
    }

    /// The panels the centre shows: the active tab of every group.
    pub fn visible_panels(&self, cx: &App) -> Vec<PanelId> {
        fn shown_in(node: &PaneNode, out: &mut Vec<PanelId>) {
            match node.kind() {
                PaneRef::Tabs { panels, active_ix } => out.extend(panels.get(active_ix).copied()),
                PaneRef::Split { children, .. } => children.iter().for_each(|child| shown_in(child, out)),
            }
        }
        let mut out = Vec::new();
        if let Some(layout) = self.dock_area.read(cx).layout(DockPlacement::Center) {
            shown_in(layout.root(), &mut out);
        }
        out
    }

    /// The file whose tab is active in the centre, if any.
    pub fn active_file(&self, cx: &App) -> Option<FileInfo> {
        self.active_panel(cx).map(|panel| panel.read(cx).info.clone())
    }

    /// Open every path: a new tab each, or the existing tab activated. A file whose
    /// tab was closed since is opened afresh.
    pub fn open_paths(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        let dock = self.dock_area.clone();
        self.open
            .retain(|f| dock.read(cx).panel(PanelId::from(f.panel.entity_id())).is_some());
        for path in paths {
            let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());
            if let Some(existing) = self.open.iter().find(|f| f.key == key) {
                let id = PanelId::from(existing.panel.entity_id());
                self.dock_area
                    .update(cx, |dock, cx| dock.select_panel(id, window, cx));
                continue;
            }
            let info = FileInfo::probe(path);
            let rules = self.rules.clone();
            let tools = self.tools.clone();
            let panel = cx.new(|cx| FilePanel::new(info, &rules, &tools, window, cx));
            // The kit's handle carries the panel's title; a bare entity would show its panel_name.
            let handle = panel_handle(panel.clone());
            self.dock_area.update(cx, |dock, cx| {
                dock.add_panel_view(handle, DockPlacement::Center, None, window, cx)
            });
            self.open.push(OpenFile { key, panel });
        }
        cx.notify();
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
        Theme::change(mode, Some(window), cx);
        cx.notify();
    }

    fn on_toggle_rules(
        &mut self,
        _: &ToggleHighlightRules,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock_area
            .update(cx, |dock, cx| dock.toggle_dock(DockPlacement::Right, window, cx));
        cx.notify();
    }

    fn on_toggle_hex(&mut self, _: &ToggleHexView, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.active_panel(cx) {
            panel.update(cx, |panel, cx| panel.toggle_hex(window, cx));
        }
    }

    /// Save the active tab's text (C-008). The outcome is shown in the view's status row.
    fn on_save(&mut self, _: &SaveFile, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.active_panel(cx).and_then(|panel| panel.read(cx).editable_text(cx)) {
            let _ = view.update(cx, |view, cx| view.save(cx));
        }
    }

    /// Save the active tab's image as another raster format (C-009): a Save dialog picks
    /// the new name, whose extension chooses the format; the outcome shows in the facts.
    /// A Markdown link to a file: open it, and scroll to the heading its fragment names (C-011).
    fn on_open_linked_file(&mut self, action: &OpenLinkedFile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_paths(std::slice::from_ref(&action.path), window, cx);
        // The file may already be open under another spelling of its path (a case, a `.` or `..`):
        // the tab is found as `open_paths` finds it, by the file on disk, and takes the focus, so the
        // chords act on the tab that came up.
        let target = self.panels(cx).iter().find_map(|panel| {
            let view = panel.read(cx).markdown_view()?.clone();
            same_file(view.read(cx).path(), &action.path).then_some(view)
        });
        let Some(view) = target else { return };
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        if let Some(fragment) = &action.fragment {
            view.update(cx, |view, cx| view.reveal_heading(fragment, window, cx));
        }
    }

    fn on_save_image_as(&mut self, _: &SaveImageAs, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.active_panel(cx).and_then(|panel| panel.read(cx).image_view().cloned()) else {
            return;
        };
        let directory = view.read(cx).path().parent().map(Path::to_path_buf).unwrap_or_default();
        let receiver = cx.prompt_for_new_path(&directory, None);
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                view.update(cx, |view, cx| {
                    let _ = view.save_as(&path, cx);
                });
            }
        })
        .detach();
    }

    fn on_open_files(&mut self, _: &OpenFiles, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = receiver.await {
                this.update_in(cx, |this, window, cx| this.open_paths(&paths, window, cx))
                    .ok();
            }
        })
        .detach();
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A tab the dock closed is forgotten here too, so its panel and what it hosts (a SumatraPDF process,
        // a folder of drawings, C-014) go with it rather than at the next open.
        let dock = self.dock_area.clone();
        self.open.retain(|f| dock.read(cx).panel(PanelId::from(f.panel.entity_id())).is_some());
        // Each panel hears whether the dock shows it (the active tab of its group), so a hosted window hides
        // behind another tab even when it appears only after the tab went behind.
        let visible = self.visible_panels(cx);
        for file in &self.open {
            let shown = visible.contains(&PanelId::from(file.panel.entity_id()));
            file.panel.update(cx, |panel, cx| panel.set_shown_by_dock(shown, cx));
        }
        let nothing_open = self.open.is_empty();
        div()
            .id("workspace")
            .track_focus(&self.focus_handle)
            .key_context("Workspace")
            .on_action(cx.listener(Self::on_open_files))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_toggle_rules))
            .on_action(cx.listener(Self::on_toggle_hex))
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_save_image_as))
            .on_action(cx.listener(Self::on_open_linked_file))
            .can_drop(|value, _, _| value.downcast_ref::<ExternalPaths>().is_some())
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                let paths = paths.paths().to_vec();
                this.open_paths(&paths, window, cx);
            }))
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(div().size_full().child(self.dock_area.clone()))
            .when(nothing_open, |this| {
                this.child(
                    div()
                        .id("welcome")
                        .test_support()
                        .absolute()
                        .inset_0()
                        .flex()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .child(title(APP_NAME)),
                )
                .child(
                    div()
                        .id("welcome-hint")
                        .absolute()
                        .bottom_4()
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .opacity(0.7)
                        .child("Ctrl+O opens files. Dropping files here opens them too. Ctrl+Shift+H edits the highlighting rules. Ctrl+Shift+B shows a file's bytes."),
                )
            })
    }
}

/// The path as it stands on disk: resolved by the system when it is there (a link's `..` climbs from its
/// target, Windows' `\\?\` prefix taken off); when a file or a folder is gone meanwhile, the nearest ancestor
/// still on disk resolved the same way and the gone tail joined with its `.` and `..` resolved by spelling;
/// with nothing of it on disk, the absolute spelling with `.` and `..` resolved.
fn plain_absolute(path: &Path) -> PathBuf {
    use std::path::Component;
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(canonical) = std::fs::canonicalize(&absolute) {
        return strip_verbatim(canonical);
    }
    let join_tail = |mut base: PathBuf, tail: &[Component]| {
        for component in tail {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    base.pop();
                }
                other => base.push(other.as_os_str()),
            }
        }
        base
    };
    let components: Vec<Component> = absolute.components().collect();
    for kept in (1..components.len()).rev() {
        let ancestor: PathBuf = components[..kept].iter().map(|c| c.as_os_str()).collect();
        if let Ok(canonical) = std::fs::canonicalize(&ancestor) {
            return join_tail(strip_verbatim(canonical), &components[kept..]);
        }
    }
    join_tail(PathBuf::new(), &components)
}

/// A canonical path without the `\\?\` (or `\\?\UNC\`) prefix Windows gives it.
fn strip_verbatim(canonical: PathBuf) -> PathBuf {
    let text = canonical.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        canonical
    }
}

/// The tabs still to settle before the window may close (C-012), the first being the one the
/// dialog asks about, and the text viewer's message when it refused the last save.
struct Closing {
    queue: Vec<Entity<FilePanel>>,
    message: Option<String>,
    /// The tab whose restore could not read the file back: a second `Discard changes` drops its edit.
    refused_restore: Option<gpui_kit::EntityId>,
}

/// What a button of the closing dialog decides.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Decision {
    Save,
    Discard,
    Cancel,
    SaveAll,
    DiscardAll,
}

impl Workspace {
    /// The open panels as their tabs stand in the centre, group by group and left to right (a dragged tab
    /// counts where it was dropped); a panel the centre does not hold follows in opening order.
    pub fn panels_in_tab_order(&self, cx: &App) -> Vec<Entity<FilePanel>> {
        fn collect(node: &PaneNode, out: &mut Vec<PanelId>) {
            match node.kind() {
                PaneRef::Tabs { panels, .. } => out.extend(panels.iter().copied()),
                PaneRef::Split { children, .. } => children.iter().for_each(|child| collect(child, out)),
            }
        }
        let mut ids = Vec::new();
        if let Some(layout) = self.dock_area.read(cx).layout(DockPlacement::Center) {
            collect(layout.root(), &mut ids);
        }
        let mut ordered: Vec<Entity<FilePanel>> = ids
            .iter()
            .filter_map(|id| self.open.iter().find(|f| PanelId::from(f.panel.entity_id()) == *id).map(|f| f.panel.clone()))
            .collect();
        for panel in self.panels(cx) {
            if !ordered.iter().any(|p| p.entity_id() == panel.entity_id()) {
                ordered.push(panel);
            }
        }
        ordered
    }

    /// The tabs with unsaved edits, in tab order.
    fn modified_panels(&self, cx: &App) -> Vec<Entity<FilePanel>> {
        self.panels_in_tab_order(cx).into_iter().filter(|panel| panel.read(cx).is_dirty(cx)).collect()
    }

    /// The platform asks whether the window may close (C-012): yes when every tab is saved. Otherwise
    /// the dialog asks about the first modified tab, and the window stays until the last one is saved or
    /// discarded; a second request while the dialog shows changes nothing.
    fn may_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let queue = self.modified_panels(cx);
        if queue.is_empty() {
            return true;
        }
        if self.closing.is_none() {
            self.closing = Some(Closing { queue, message: None, refused_restore: None });
            self.open_closing_dialog(window, cx);
        }
        cx.notify();
        false
    }

    /// The facts the closing dialog shows: the tab's name and folder, how many modified tabs follow,
    /// and the message of a refused save.
    fn closing_facts(&self, cx: &App) -> Option<(String, String, usize, Option<String>)> {
        let closing = self.closing.as_ref()?;
        let first = closing.queue.first()?.read(cx);
        // Where the file as opened sits: its folder resolved, the file itself (a link, say) not followed.
        let absolute = std::path::absolute(&first.info.path).unwrap_or_else(|_| first.info.path.clone());
        let folder = absolute.parent().map(|p| plain_absolute(p).display().to_string()).unwrap_or_default();
        Some((first.info.name.clone(), folder, closing.queue.len().saturating_sub(1), closing.message.clone()))
    }

    /// The alert dialog of C-012, read afresh from the closing state at every frame: the title names the
    /// tab, the body its folder, how many modified tabs follow and a refused save's message, and the
    /// buttons decide for this tab or for all of them. Closing the dialog any other way (Escape) is Cancel.
    fn open_closing_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let facts = this.upgrade().and_then(|w| w.read(cx).closing_facts(cx));
            let Some((name, folder, more, message)) = facts else {
                return alert.title("Closing");
            };
            let danger = cx.theme().danger;
            let button = |id: &'static str, label: &'static str, decision: Decision| {
                let this = this.clone();
                Button::new(id).label(label).on_click(move |_, window, cx| {
                    this.update(cx, |workspace, cx| workspace.decide(decision, window, cx)).ok();
                })
            };
            let more_text = match more {
                1 => "1 more file has unsaved changes".to_string(),
                n => format!("{n} more files have unsaved changes"),
            };
            let on_close = this.clone();
            let on_enter = this.clone();
            alert
                .title({
                    let title = format!("Save changes to “{name}”?");
                    div().id("close-title").test_support().aria_label(title.clone()).child(title)
                })
                .description(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().id("close-folder").test_support().aria_label(folder.clone()).child(folder))
                        .when(more > 0, |body| body.child(div().id("close-more").test_support().aria_label(more_text.clone()).child(more_text)))
                        .when_some(message, |body, message| {
                            body.child(div().id("close-message").test_support().aria_label(message.clone()).text_color(danger).child(message))
                        }),
                )
                .footer(
                    DialogFooter::new()
                        .child(button("close-cancel", "Cancel", Decision::Cancel))
                        .when(more > 0, |footer| {
                            footer
                                .child(button("close-discard-all", "Discard all", Decision::DiscardAll).danger())
                                .child(button("close-save-all", "Save all", Decision::SaveAll))
                        })
                        .child(button("close-discard", "Discard changes", Decision::Discard).danger())
                        .child(button("close-save", "Save", Decision::Save).primary()),
                )
                .on_close(move |_, _, cx| {
                    on_close.update(cx, |workspace, cx| workspace.closing_dialog_closed(cx)).ok();
                })
                // Enter is the primary button, Save; the dialog's own closing is ours (`decide` closes it).
                .on_ok(move |_, window, cx| {
                    on_enter.update(cx, |workspace, cx| workspace.decide(Decision::Save, window, cx)).ok();
                    false
                })
        });
    }

    /// The dialog went away without a decision that closes the window: every tab stays as it is.
    fn closing_dialog_closed(&mut self, cx: &mut Context<Self>) {
        self.closing = None;
        cx.notify();
    }

    /// A button of the closing dialog: Cancel keeps everything as it is; the others save or restore this
    /// tab, or every remaining one, and the window closes once no modified tab remains. A refused save
    /// keeps the dialog on that tab with the viewer's message.
    fn decide(&mut self, decision: Decision, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut closing) = self.closing.take() else {
            return;
        };
        closing.message = None;
        let (count, save) = match decision {
            Decision::Cancel => {
                window.close_dialog(cx);
                cx.notify();
                return;
            }
            Decision::Save => (1, true),
            Decision::Discard => (1, false),
            Decision::SaveAll => (usize::MAX, true),
            Decision::DiscardAll => (usize::MAX, false),
        };
        self.settle(&mut closing, count, save, window, cx);
        if closing.queue.is_empty() {
            window.close_dialog(cx);
            window.remove_window();
        } else {
            self.closing = Some(closing);
        }
        cx.notify();
    }

    /// Save or restore the first `count` tabs of the queue (a tab saved meanwhile is skipped); a refused
    /// save, or a restore that could not read the file back, stops on its tab with the viewer's message.
    fn settle(&mut self, closing: &mut Closing, count: usize, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut done = 0;
        while done < count && !closing.queue.is_empty() {
            let panel = closing.queue[0].clone();
            if panel.read(cx).is_dirty(cx)
                && let Some(text) = panel.read(cx).editable_text(cx)
            {
                if save {
                    if let Err(message) = text.update(cx, |text, cx| text.save(cx)) {
                        closing.message = Some(message);
                        return;
                    }
                } else if closing.refused_restore == Some(panel.entity_id()) {
                    // Asked a second time with the message in view: the edit is dropped as asked, file or no
                    // file, and with it the tab, which has nothing left to show.
                    let dock = self.dock_area.clone();
                    dock.update(cx, |dock, cx| dock.remove_panel(panel.clone(), window, cx));
                } else {
                    text.update(cx, |text, cx| text.reload(window, cx));
                    if text.read(cx).is_dirty() {
                        // The saved text could not be read back (the file is gone): nothing was restored, so
                        // the question stays on this tab with the viewer's message, as after a refused save;
                        // the next Discard changes drops the edit.
                        closing.message =
                            text.read(cx).message().map(|m| format!("{m}; Discard changes again drops the edit"));
                        closing.refused_restore = Some(panel.entity_id());
                        return;
                    }
                }
            }
            closing.queue.remove(0);
            done += 1;
        }
    }
}
