//! PDF and XPS (C-014): SumatraPDF hosted inside the panel on Windows, or the pages drawn by mutool or
//! pdftoppm into a scrollable column, or, with none of them installed, the hint that names each tool and the
//! bytes meanwhile. The tools come from the workspace's `ToolSet`; `Look again` asks it afresh.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Bounds, Context, Entity, FocusHandle, Focusable, ObjectFit, Pixels, Render, RenderImage, Role, ScrollHandle,
    SharedString, Size, Task, TestSupportExt, Window, actions, canvas, div, img, px,
};

use crate::hex::HexFile;
use crate::images;
use crate::pdf_pages::{Backend, PageSize, Renderer};
use crate::tools::{self, Found, Tool, ToolSet};
use crate::viewers::hex_view::HexView;
use crate::viewers::image_view::render_image;

actions!(pdf_view, [ZoomIn, ZoomOut, ZoomFit]);

pub const KEY_CONTEXT: &str = "PdfView";
/// A zoom step: a quarter up or down.
const ZOOM_STEP: f32 = 1.25;
/// Space around and between the pages, logical pixels.
const PAGE_GAP: f32 = 12.0;
/// Pages drawn beyond the visible ones, each way.
const PRELOAD: usize = 1;
/// The dots per inch a page is drawn at are kept within reason.
const DPI_RANGE: (f32, f32) = (24.0, 600.0);

/// `A PDF`, `An XPS`, `An OXPS`: the article the kind takes when read aloud.
pub fn article_for(kind: &str) -> &'static str {
    if kind.starts_with(['X', 'O', 'A', 'E', 'I', 'U']) { "An" } else { "A" }
}

/// The tools the family tries, in order, on this platform.
pub fn candidates() -> &'static [Tool] {
    if cfg!(windows) { &[Tool::SumatraPdf, Tool::MuTool, Tool::PdfToPpm] } else { &[Tool::MuTool, Tool::PdfToPpm] }
}

/// What became of the SumatraPDF host (Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostState {
    /// Another tool, or none, is in use.
    NotUsed,
    /// A window without a native handle (the headless tests): nothing to host in.
    NoWindow,
    /// The process runs; its frame is placed on each paint.
    Started,
    /// The process could not start, or has gone.
    Failed,
}

/// The pages drawn by a renderer.
struct Pages {
    renderer: Arc<Renderer>,
    sizes: Vec<PageSize>,
    listing: bool,
    failure: Option<String>,
    /// The page's latest drawing and the dpi it was drawn at.
    images: HashMap<usize, (u32, Arc<RenderImage>)>,
    pending: HashSet<(usize, u32)>,
    /// Drawings that failed, not asked for again until the zoom changes or the tools are looked at again.
    failed: HashSet<(usize, u32)>,
    /// `None` fits the width.
    zoom: Option<f32>,
    scroll: ScrollHandle,
    dir: PathBuf,
}

impl Drop for Pages {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

enum Mode {
    #[cfg(windows)]
    Host(Option<std::rc::Rc<std::cell::RefCell<crate::pdf_host::SumatraHost>>>),
    Pages(Box<Pages>),
    Missing,
}

pub struct PdfView {
    path: PathBuf,
    focus_handle: FocusHandle,
    tools: Arc<ToolSet>,
    tool: Option<(Tool, Found)>,
    mode: Mode,
    host_state: HostState,
    hex: Option<Entity<HexView>>,
    hex_error: Option<String>,
    viewport: Option<Size<Pixels>>,
    shown: bool,
    message: String,
    _tasks: Vec<Task<()>>,
}

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

impl PdfView {
    pub fn new(path: &Path, tools: Arc<ToolSet>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            path: path.to_path_buf(),
            focus_handle: cx.focus_handle(),
            tools,
            tool: None,
            mode: Mode::Missing,
            host_state: HostState::NotUsed,
            hex: None,
            hex_error: None,
            viewport: None,
            shown: true,
            message: String::new(),
            _tasks: Vec::new(),
        };
        view.detect(window, cx);
        view
    }

    /// Choose the tool and open the file through it.
    fn detect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tool = self.tools.first_found(candidates());
        self.host_state = HostState::NotUsed;
        self.message.clear();
        match self.tool.clone() {
            #[cfg(windows)]
            Some((Tool::SumatraPdf, found)) => self.start_host(&found.path, window, cx),
            #[cfg(not(windows))]
            Some((Tool::SumatraPdf, _)) => self.go_missing(window, cx),
            Some((tool @ (Tool::MuTool | Tool::PdfToPpm), found)) => self.start_pages(tool, found, cx),
            _ => self.go_missing(window, cx),
        }
        cx.notify();
    }

    #[cfg(windows)]
    fn start_host(&mut self, exe: &Path, window: &mut Window, cx: &mut Context<Self>) {
        use crate::pdf_host::{SumatraHost, handle_of};
        let Some(parent) = handle_of(window) else {
            self.host_state = HostState::NoWindow;
            self.mode = Mode::Host(None);
            return;
        };
        match SumatraHost::start(exe, &self.path, parent) {
            Ok(host) => {
                self.host_state = HostState::Started;
                self.mode = Mode::Host(Some(std::rc::Rc::new(std::cell::RefCell::new(host))));
                // SumatraPDF creates its frame a moment after it starts: a few repaints place it in time.
                self._tasks.push(cx.spawn(async move |this, cx| {
                    for ms in [100u64, 300, 700, 1500] {
                        cx.background_executor().timer(Duration::from_millis(ms)).await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
            Err(error) => {
                self.host_state = HostState::Failed;
                self.message = format!("SumatraPDF could not start: {error}.");
                self.go_missing(window, cx);
            }
        }
    }

    fn start_pages(&mut self, tool: Tool, found: Found, cx: &mut Context<Self>) {
        let backend = if tool == Tool::MuTool { Backend::MuTool } else { Backend::PdfToPpm };
        let info_exe = (backend == Backend::PdfToPpm).then(|| self.tools.find(Tool::PdfInfo).map(|f| f.path)).flatten();
        if backend == Backend::PdfToPpm && info_exe.is_none() {
            self.message = format!("pdftoppm needs pdfinfo from the same package to list the pages: {}.", Tool::PdfInfo.install_hint());
        }
        let renderer = Arc::new(Renderer { backend, exe: found.path, info_exe });
        let dir = std::env::temp_dir().join(format!("sfoglia-pdf-{}-{}", std::process::id(), NEXT_DIR.fetch_add(1, Ordering::Relaxed)));
        self.mode = Mode::Pages(Box::new(Pages {
            renderer: renderer.clone(),
            sizes: Vec::new(),
            listing: true,
            failure: None,
            images: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            zoom: None,
            scroll: ScrollHandle::new(),
            dir,
        }));
        let path = self.path.clone();
        self._tasks.push(cx.spawn(async move |this, cx| {
            let listed = cx.background_spawn(async move { renderer.pages(&path) }).await;
            let _ = this.update(cx, |view, cx| view.pages_listed(listed, cx));
        }));
    }

    fn pages_listed(&mut self, listed: Result<Vec<PageSize>, String>, cx: &mut Context<Self>) {
        if let Mode::Pages(pages) = &mut self.mode {
            pages.listing = false;
            match listed {
                Ok(sizes) => pages.sizes = sizes,
                Err(failure) => pages.failure = Some(failure),
            }
            cx.notify();
        }
    }

    /// No tool: the hint and the bytes.
    fn go_missing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Missing;
        if self.hex.is_none() && self.hex_error.is_none() {
            match HexFile::open(&self.path) {
                Ok(file) => self.hex = Some(cx.new(|cx| HexView::new(file, window, cx))),
                Err(error) => self.hex_error = Some(format!("Cannot map this file: {error}")),
            }
        }
    }

    /// `Look again`: the tools asked afresh, the file opened again through what is found now.
    pub fn look_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for tool in candidates().iter().chain([Tool::PdfInfo].iter()) {
            self.tools.refresh(*tool);
        }
        self._tasks.clear();
        self.detect(window, cx);
    }

    /// Hide or show what the panel hosts (the tab inactive, the hex view on).
    pub fn set_shown(&mut self, shown: bool, _cx: &mut Context<Self>) {
        self.shown = shown;
        #[cfg(windows)]
        if let Mode::Host(Some(host)) = &self.mode {
            host.borrow_mut().show(shown);
        }
    }

    pub fn tool(&self) -> Option<(Tool, &Found)> {
        self.tool.as_ref().map(|(tool, found)| (*tool, found))
    }

    pub fn host_state(&self) -> HostState {
        self.host_state
    }

    /// `host`, `pages` or `missing`.
    pub fn mode_name(&self) -> &'static str {
        match &self.mode {
            #[cfg(windows)]
            Mode::Host(_) => "host",
            Mode::Pages(_) => "pages",
            Mode::Missing => "missing",
        }
    }

    /// The hex view shown under the hint, when no tool is installed.
    pub fn hex_view(&self) -> Option<&Entity<HexView>> {
        if matches!(self.mode, Mode::Missing) { self.hex.as_ref() } else { None }
    }

    pub fn page_sizes(&self) -> Vec<PageSize> {
        match &self.mode {
            Mode::Pages(pages) => pages.sizes.clone(),
            _ => Vec::new(),
        }
    }

    /// The pages drawn so far with the dpi each was drawn at, in page order.
    pub fn drawn_pages(&self) -> Vec<(usize, u32)> {
        match &self.mode {
            Mode::Pages(pages) => {
                let mut drawn: Vec<(usize, u32)> = pages.images.iter().map(|(page, (dpi, _))| (*page, *dpi)).collect();
                drawn.sort();
                drawn
            }
            _ => Vec::new(),
        }
    }

    pub fn is_listing(&self) -> bool {
        matches!(&self.mode, Mode::Pages(pages) if pages.listing)
    }

    pub fn failure(&self) -> Option<&str> {
        match &self.mode {
            Mode::Pages(pages) => pages.failure.as_deref(),
            _ => None,
        }
    }

    /// The zoom chosen, or none for fit-width.
    pub fn zoom(&self) -> Option<f32> {
        match &self.mode {
            Mode::Pages(pages) => pages.zoom,
            _ => None,
        }
    }

    /// The zoom in effect: the chosen one, or the one that fits the widest page in the viewport.
    pub fn effective_zoom(&self) -> f32 {
        match &self.mode {
            Mode::Pages(pages) => pages.zoom.unwrap_or_else(|| self.fit_zoom(pages)),
            _ => 1.0,
        }
    }

    fn fit_zoom(&self, pages: &Pages) -> f32 {
        let widest = pages.sizes.iter().map(|s| s.width_pt).fold(0.0f32, f32::max);
        match self.viewport {
            Some(viewport) if widest > 0.0 => {
                let available = f32::from(viewport.width) - 2.0 * PAGE_GAP;
                (available / (widest * 96.0 / 72.0)).clamp(0.05, 16.0)
            }
            _ => 1.0,
        }
    }

    /// The folder the page drawings land in, removed with the view.
    pub fn scratch_dir(&self) -> Option<PathBuf> {
        match &self.mode {
            Mode::Pages(pages) => Some(pages.dir.clone()),
            _ => None,
        }
    }

    pub fn scroll_handle(&self) -> Option<&ScrollHandle> {
        match &self.mode {
            Mode::Pages(pages) => Some(&pages.scroll),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        self.message_text()
    }

    pub fn zoom_in(&mut self, cx: &mut Context<Self>) {
        let zoom = self.effective_zoom() * ZOOM_STEP;
        self.set_zoom(Some(zoom.min(16.0)), cx);
    }

    pub fn zoom_out(&mut self, cx: &mut Context<Self>) {
        let zoom = self.effective_zoom() / ZOOM_STEP;
        self.set_zoom(Some(zoom.max(0.05)), cx);
    }

    pub fn zoom_fit(&mut self, cx: &mut Context<Self>) {
        self.set_zoom(None, cx);
    }

    fn set_zoom(&mut self, zoom: Option<f32>, cx: &mut Context<Self>) {
        if let Mode::Pages(pages) = &mut self.mode {
            pages.zoom = zoom;
            pages.failed.clear();
            cx.notify();
        }
    }

    fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_in(cx);
    }

    fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_out(cx);
    }

    fn on_zoom_fit(&mut self, _: &ZoomFit, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_fit(cx);
    }

    /// `PDF`, `XPS` or `OXPS` by the extension.
    pub fn kind(&self) -> String {
        self.path.extension().map_or("PDF".to_string(), |e| e.to_string_lossy().to_ascii_uppercase())
    }

    /// The page at the top of the viewport, from 1.
    pub fn current_page(&self) -> Option<usize> {
        let Mode::Pages(pages) = &self.mode else { return None };
        if pages.sizes.is_empty() {
            return None;
        }
        let zoom = self.effective_zoom();
        let top = -f32::from(pages.scroll.offset().y);
        let mut y = PAGE_GAP;
        for (ix, size) in pages.sizes.iter().enumerate() {
            let height = size.height_pt * 96.0 / 72.0 * zoom;
            if y + height > top + 1.0 {
                return Some(ix + 1);
            }
            y += height + PAGE_GAP;
        }
        Some(pages.sizes.len())
    }

    /// `PDF, 3 pages · page 1 · 100 %` in the page column; the kind alone otherwise.
    pub fn summary(&self) -> String {
        let kind = self.kind();
        match &self.mode {
            Mode::Pages(pages) if pages.listing => format!("{kind}, listing the pages"),
            Mode::Pages(pages) if pages.failure.is_some() => format!("{kind}, {}", pages.failure.clone().unwrap_or_default()),
            Mode::Pages(pages) => {
                let n = pages.sizes.len();
                let noun = if n == 1 { "page" } else { "pages" };
                format!(
                    "{kind}, {n} {noun} · page {} · {} %",
                    self.current_page().unwrap_or(1),
                    (self.effective_zoom() * 100.0).round() as i64
                )
            }
            _ => kind,
        }
    }

    /// The tool named with its version, or `no tool`.
    pub fn tool_label(&self) -> String {
        match &self.tool {
            Some((tool, found)) => found.describe(*tool),
            None => "no tool".to_string(),
        }
    }

    fn message_text(&self) -> String {
        if !self.message.is_empty() {
            return self.message.clone();
        }
        match &self.mode {
            #[cfg(windows)]
            Mode::Host(_) => "the page, find, zoom, rotation and view controls are SumatraPDF's own".to_string(),
            Mode::Pages(pages) => {
                if pages.renderer.backend == Backend::PdfToPpm && !self.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                    format!("pdftoppm reads PDF alone: for {} {}", self.kind(), Tool::MuTool.install_hint())
                } else {
                    "+ and - zoom, 0 fits the width".to_string()
                }
            }
            Mode::Missing => {
                let kind = self.kind();
                let what = format!("{} {kind}", article_for(&kind));
                format!("{} The bytes show meanwhile.", tools::missing_message(&what, candidates()))
            }
        }
    }

    /// Ask for the drawings of the pages in and around the viewport that are not drawn at this dpi yet.
    fn request_visible(&mut self, window: &Window, cx: &mut Context<Self>) {
        let zoom = self.effective_zoom();
        let Some(viewport) = self.viewport else { return };
        let Mode::Pages(pages) = &mut self.mode else { return };
        if pages.sizes.is_empty() {
            return;
        }
        let dpi = (zoom * 96.0 * window.scale_factor()).round().clamp(DPI_RANGE.0, DPI_RANGE.1) as u32;
        let top = -f32::from(pages.scroll.offset().y);
        let bottom = top + f32::from(viewport.height);
        let mut y = PAGE_GAP;
        let mut visible: Vec<usize> = Vec::new();
        for (ix, size) in pages.sizes.iter().enumerate() {
            let height = size.height_pt * 96.0 / 72.0 * zoom;
            if y + height >= top && y <= bottom {
                visible.push(ix + 1);
            }
            y += height + PAGE_GAP;
        }
        let (Some(&first), Some(&last)) = (visible.first(), visible.last()) else { return };
        let wanted = first.saturating_sub(PRELOAD).max(1)..=(last + PRELOAD).min(pages.sizes.len());
        for page in wanted {
            let drawn = pages.images.get(&page).is_some_and(|(at, _)| *at == dpi);
            if drawn || pages.pending.contains(&(page, dpi)) || pages.failed.contains(&(page, dpi)) {
                continue;
            }
            pages.pending.insert((page, dpi));
            let renderer = pages.renderer.clone();
            let dir = pages.dir.clone();
            let path = self.path.clone();
            cx.spawn(async move |this, cx| {
                let drawn = cx
                    .background_spawn(async move {
                        let png = renderer.render(&path, page, dpi, &dir)?;
                        let bytes = std::fs::read(&png).map_err(|e| format!("cannot read {}: {e}", png.display()))?;
                        images::decode(&bytes, "png")
                    })
                    .await;
                let _ = this.update(cx, |view, cx| view.page_drawn(page, dpi, drawn, cx));
            })
            .detach();
        }
    }

    fn page_drawn(&mut self, page: usize, dpi: u32, drawn: Result<images::Picture, String>, cx: &mut Context<Self>) {
        if let Mode::Pages(pages) = &mut self.mode {
            pages.pending.remove(&(page, dpi));
            match drawn {
                Ok(picture) => {
                    pages.images.insert(page, (dpi, render_image(&picture, 0)));
                }
                Err(failure) => {
                    pages.failed.insert((page, dpi));
                    pages.failure = Some(failure);
                }
            }
            cx.notify();
        }
    }
}

impl Focusable for PdfView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PdfView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.request_visible(window, cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let background = theme.background;
        let foreground = theme.foreground;
        let summary = self.summary();
        let tool_label = self.tool_label();
        let message = self.message_text();
        let missing = matches!(self.mode, Mode::Missing);
        let status_row = h_flex()
            .id("pdf-status")
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
            .child(div().id("pdf-summary").test_support().aria_label(summary.clone()).child(summary))
            .child(div().id("pdf-tool").test_support().aria_label(tool_label.clone()).child(tool_label))
            .when(missing, |row| {
                row.child(
                    div().id("pdf-look-again").test_support().aria_label("Look again").child(
                        Button::new("pdf-look-again-button")
                            .label("Look again")
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| this.look_again(window, cx))),
                    ),
                )
            })
            .child(div().id("pdf-message").test_support().aria_label(message.clone()).flex_1().overflow_hidden().child(message));

        let this = cx.entity().clone();
        let measure = canvas(
            move |bounds: Bounds<Pixels>, _window, cx| {
                this.update(cx, |view, cx| {
                    if view.viewport != Some(bounds.size) {
                        view.viewport = Some(bounds.size);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        let body = match &self.mode {
            #[cfg(windows)]
            Mode::Host(host) => {
                let placer = host.clone().map(|host| {
                    canvas(
                        |_, _, _| {},
                        move |bounds: Bounds<Pixels>, _, window, _| {
                            let scale = window.scale_factor();
                            let at = |v: Pixels| (f32::from(v) * scale).round() as i32;
                            host.borrow_mut().place(crate::pdf_host::Placement {
                                x: at(bounds.origin.x),
                                y: at(bounds.origin.y),
                                width: at(bounds.size.width),
                                height: at(bounds.size.height),
                            });
                        },
                    )
                    .absolute()
                    .size_full()
                });
                let note = match self.host_state {
                    HostState::NoWindow => "SumatraPDF needs a window to show the document in",
                    _ => "SumatraPDF shows the document here",
                };
                div()
                    .id("pdf-host")
                    .test_support()
                    .aria_label(note)
                    .relative()
                    .size_full()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .child(note)
                    .children(placer)
                    .into_any_element()
            }
            Mode::Pages(pages) => {
                let zoom = self.effective_zoom();
                let page_boxes: Vec<_> = pages
                    .sizes
                    .iter()
                    .enumerate()
                    .map(|(ix, size)| {
                        let page = ix + 1;
                        let (w, h) = (size.width_pt * 96.0 / 72.0 * zoom, size.height_pt * 96.0 / 72.0 * zoom);
                        let image = pages.images.get(&page).map(|(_, image)| image.clone());
                        let label = format!("page {page}: {} × {} pt{}", size.width_pt, size.height_pt, if image.is_some() { ", drawn" } else { "" });
                        div()
                            .id(("pdf-page", page))
                            .test_support()
                            .aria_label(label)
                            .flex_shrink_0()
                            .w(px(w))
                            .h(px(h))
                            .bg(gpui_kit::white())
                            .shadow_md()
                            .children(image.map(|image| img(image).size_full().object_fit(ObjectFit::Fill)))
                    })
                    .collect();
                div()
                    .id("pdf-scroll")
                    .test_support()
                    .relative()
                    .size_full()
                    .min_h_0()
                    .overflow_scroll()
                    .track_scroll(&pages.scroll)
                    .child(measure)
                    .child(
                        v_flex()
                            .id("pdf-column")
                            .items_center()
                            .gap(px(PAGE_GAP))
                            .p(px(PAGE_GAP))
                            .children(page_boxes),
                    )
                    .into_any_element()
            }
            Mode::Missing => {
                let hex = self.hex.clone();
                let hex_error = self.hex_error.clone();
                div()
                    .id("pdf-hex")
                    .test_support()
                    .size_full()
                    .min_h_0()
                    .children(hex)
                    .children(hex_error.map(|error| div().p_4().text_color(muted).child(error)))
                    .into_any_element()
            }
        };

        v_flex()
            .id("pdf-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_fit))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(body)
            .child(status_row)
    }
}

impl SharedStringExt for PdfView {}

/// Nothing yet: a place for a shared helper if the markdown and pdf views converge.
pub trait SharedStringExt {
    fn shared(text: &str) -> SharedString {
        SharedString::from(text.to_string())
    }
}
