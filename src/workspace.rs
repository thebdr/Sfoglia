//! The main window's content: open files as tabs in a dock (C-004), the application
//! name when nothing is open (C-001), the highlighting rules panel on the right (C-006)
//! and the hex view any tab can switch to (C-007).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
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
use crate::viewers::hex_view::{self, FocusGoto, FocusSearch, ToggleSearchMode};
use crate::viewers::image_view::{self, RotateQuarter, ZoomActual, ZoomFit, ZoomIn, ZoomOut};
use crate::viewers::{HexView, ImageView, RulesPanel, RulesStore, TextView};

actions!(sfoglia, [OpenFiles, ToggleTheme, ToggleHighlightRules, ToggleHexView, SaveFile, SaveImageAs]);

/// Key bindings and other once-per-application setup. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
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
    /// Keeps the tab in step with the text view's modified state (C-008).
    _observe_text: Option<Subscription>,
}

impl FilePanel {
    pub fn new(
        info: FileInfo,
        rules: &Entity<RulesStore>,
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
            PanelView::Image(_) | PanelView::Info => None,
        };
        let mut panel = Self {
            focus_handle: cx.focus_handle(),
            info,
            view,
            hex: None,
            hex_shown: false,
            hex_error: None,
            _observe_text: observe_text,
        };
        // A binary nobody claims opens as hex straight away.
        if panel.info.error.is_none() && panel.info.route() == Route::Hex {
            panel.show_hex(window, cx);
        }
        // An image view takes the focus as soon as its tab exists, so its keys work at once;
        // the dock itself focuses a panel only when a tab is selected later.
        if let PanelView::Image(view) = &panel.view {
            let focus = view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
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
            PanelView::Image(_) | PanelView::Info => false,
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

impl EventEmitter<PanelEvent> for FilePanel {}

impl Focusable for FilePanel {
    /// The dock focuses the active panel through this: an image tab hands the focus to its
    /// view, so the zoom and rotation keys work as soon as the tab shows.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.view {
            PanelView::Image(view) if !self.hex_shown => view.read(cx).focus_handle(cx),
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
        let (dock_area, skin) = DockSkin::dock_area("sfoglia-main", Some(1), window, cx);
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
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
        }
    }

    pub fn rules(&self) -> &Entity<RulesStore> {
        &self.rules
    }

    pub fn rules_panel(&self) -> &Entity<RulesPanel> {
        &self.rules_panel
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
            let panel = cx.new(|cx| FilePanel::new(info, &rules, window, cx));
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
        if let Some(view) = self.active_panel(cx).and_then(|panel| panel.read(cx).text_view().cloned()) {
            let _ = view.update(cx, |view, cx| view.save(cx));
        }
    }

    /// Save the active tab's image as another raster format (C-009): a Save dialog picks
    /// the new name, whose extension chooses the format; the outcome shows in the facts.
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
