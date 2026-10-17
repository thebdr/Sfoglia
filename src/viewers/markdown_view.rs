//! The Markdown viewer (C-011): a file rendered by the kit's rich text (headings, lists, tables,
//! highlighted code, images, links), with the raw text in the text viewer one chord away and the
//! place kept across the toggle.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gpui_kit::base::text::TextView as RichText;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Position;
use gpui_kit::component::text::TextViewState;
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Action, App, ClickEvent, Context, Entity, FocusHandle, Focusable, Image, ImageFormat, ImageSource,
    IntoElement, Render, Resource, Role, SharedString, SharedUri, Subscription, Task, TestSupportExt, Window,
    actions, div,
};

use crate::markdown::{self, ImageSrc, LinkTarget, Outline};
use crate::viewers::rules_panel::RulesStore;
use crate::viewers::text_view::TextView;

actions!(markdown_view, [ToggleRaw]);

/// Asks the workspace to open the file a link points at, and to scroll to the heading its fragment names.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = sfoglia, no_json)]
pub struct OpenLinkedFile {
    pub path: PathBuf,
    pub fragment: Option<String>,
}

/// The key context of the whole view: the chord works from the raw text too.
pub const KEY_CONTEXT: &str = "MarkdownView";

/// The longest text that renders: past this the outline and the renderer's parse would take too long
/// even off the window thread, so the file opens on its raw text and the toggle keeps it there while
/// the text (as decoded, line feeds alone, measured again after every reload and toggle) is that long.
pub const MAX_RENDERED_BYTES: usize = 2 * 1024 * 1024;

/// A 1×1 transparent PNG, shown for an image whose address cannot be read; encoded once.
fn blank_png() -> &'static [u8] {
    static BLANK: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    BLANK.get_or_init(|| {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1).write_to(&mut bytes, image::ImageFormat::Png).expect("a 1x1 PNG encodes");
        bytes.into_inner()
    })
}

pub struct MarkdownView {
    focus_handle: FocusHandle,
    path: PathBuf,
    /// The file's folder, which relative links and images are read from.
    folder: PathBuf,
    /// The kit's rich text: the rendered side.
    rich: Entity<TextViewState>,
    /// The raw text (C-005, C-008), built at open: it watches the disk and holds the edits.
    text: Entity<TextView>,
    raw_shown: bool,
    /// Past `MAX_RENDERED_BYTES`: the raw text alone, the toggle refused until the text is shorter.
    too_big: bool,
    /// The text's size as last measured, for the status row of a text too long to render.
    size: usize,
    /// The blocks of the source last rendered, which the status row, the toggle and links use.
    outline: Outline,
    /// The source the rich text was last given, to tell when its parse has landed.
    source: String,
    /// The preparation of the text for the renderer (reference images, fences, the outline), run off
    /// the window thread; a newer one replaces, and so cancels, an older one.
    preparing: Option<Task<()>>,
    /// The search for a fragment's heading in the raw text, run off the window thread too.
    finding: Option<Task<()>>,
    /// A source range to scroll the rendered view to once the parse of `source` has landed.
    pending_reveal: Option<Range<usize>>,
    status: String,
    /// The text viewer's operation and reload counts as last followed (C-010's pattern).
    seen_operations: u64,
    seen_reloads: u64,
    /// The image files the renderer asked for, as the tests read them.
    requested: Arc<Mutex<Vec<PathBuf>>>,
    _subscriptions: Vec<Subscription>,
}

impl MarkdownView {
    /// Open `bytes`, the file at `path`, rendered.
    pub fn new(path: &Path, bytes: &[u8], rules: &Entity<RulesStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| TextView::new(path, bytes, rules, window, cx));
        let decoded = Self::source_of(&text, cx);
        let size = decoded.len();
        let too_big = size > MAX_RENDERED_BYTES;
        let rich = cx.new(|cx| TextViewState::markdown("", cx).scrollable(true).selectable(true));
        let follow = cx.observe_in(&text, window, |this, _, window, cx| this.follow_text(window, cx));
        let parsed = cx.observe(&rich, |this, _, cx| this.on_parsed(cx));
        let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            path: path.to_path_buf(),
            folder,
            rich,
            text,
            raw_shown: too_big,
            too_big,
            size,
            outline: Outline::default(),
            source: String::new(),
            preparing: None,
            finding: None,
            pending_reveal: None,
            status: if too_big { too_big_message(size) } else { String::new() },
            seen_operations: 0,
            seen_reloads: 0,
            requested: Arc::default(),
            _subscriptions: vec![follow, parsed],
        };
        if !too_big {
            view.render_source(decoded, None, cx);
        }
        view
    }

    /// The editor's text with line feeds alone, as the parser counts positions.
    fn source_of(text: &Entity<TextView>, cx: &App) -> String {
        let shown = text.read(cx).text(cx);
        crate::data::with_line_feeds(shown.as_ref()).into_owned()
    }

    /// Render `source`: prepared off the window thread (reference images inline, code fences named,
    /// the outline built), then handed to the renderer; a reveal of the block holding `reveal_line`
    /// waits for the renderer's own parse. A newer call cancels an older one still running.
    fn render_source(&mut self, source: String, reveal_line: Option<usize>, cx: &mut Context<Self>) {
        self.preparing = Some(cx.spawn(async move |this, cx| {
            let prepared = cx.background_spawn(async move { markdown::prepare(&source) }).await;
            this.update(cx, |this, cx| {
                this.outline = prepared.outline;
                this.rich.update(cx, |rich, cx| rich.set_text(&prepared.text, cx));
                this.source = prepared.text;
                this.preparing = None;
                if let Some(line) = reveal_line
                    && let Some(span) =
                        this.outline.top_block_at_line(line).and_then(|ix| this.outline.top_block(ix)).map(|b| b.span.clone())
                {
                    this.reveal_source(span, cx);
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// The rich text changed (a parse landed, a scroll, a selection): carry out a waiting reveal once
    /// the rendered text is the source it was asked for.
    fn on_parsed(&mut self, cx: &mut Context<Self>) {
        if let Some(span) = self.pending_reveal.clone() {
            let rendered = self.rich.read(cx).rendered_text();
            if rendered.source() == self.source {
                self.pending_reveal = None;
                let range = rendered
                    .range_for_source(span.clone())
                    .or_else(|| rendered.range_for_source(span.start..span.start));
                if let Some(range) = range {
                    self.rich.update(cx, |rich, cx| {
                        let _ = rich.reveal_range(range, cx);
                    });
                }
            }
        }
        cx.notify();
    }

    /// Scroll the rendered view to the block whose source `span` this is, now or when its parse lands.
    fn reveal_source(&mut self, span: Range<usize>, cx: &mut Context<Self>) {
        self.pending_reveal = Some(span);
        self.on_parsed(cx);
    }

    /// The raw text, with the cursor on the first line of the block at the top of the rendered view. While
    /// the rendered side is still being prepared the list and the outline are the old ones, so the cursor
    /// stays where the user left it.
    fn show_raw(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let top = self.rich.read(cx).list_state().logical_scroll_top().item_ix;
        if self.preparing.is_none()
            && let Some(block) = self.outline.top_block(top)
        {
            let position = Position::new(block.line.saturating_sub(1) as u32, 0);
            let editor = self.text.read(cx).editor().clone();
            editor.update(cx, |editor, cx| editor.set_cursor_position(position, window, cx));
        }
        self.raw_shown = true;
        let focus = self.text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// The rendered view, built from the text as edited and scrolled to the block holding the cursor's line;
    /// a text past the budget stays raw and says so.
    fn show_rendered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = Self::source_of(&self.text, cx);
        self.size = source.len();
        self.too_big = self.size > MAX_RENDERED_BYTES;
        if self.too_big {
            self.raw_shown = true;
            self.status = too_big_message(self.size);
            cx.notify();
            return;
        }
        let line = self.text.read(cx).editor().read(cx).cursor_position().line as usize + 1;
        self.render_source(source, Some(line), cx);
        self.raw_shown = false;
        // A heading search still running was for the raw side: its answer would land on the wrong side.
        self.finding = None;
        // Back to the rendered side: the hint again, whatever the raw side said last (the budget message included).
        self.status = String::new();
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    fn on_toggle_raw(&mut self, _: &ToggleRaw, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_shown {
            self.show_rendered(window, cx);
        } else {
            self.show_raw(window, cx);
        }
    }

    /// The text viewer's messages (a save, a refused save, a failed reload) show on the rendered side
    /// too, once per operation, so a repeated refusal shows again.
    fn show_text_message(&mut self, cx: &App) {
        let operations = self.text.read(cx).operations();
        if operations != self.seen_operations {
            self.seen_operations = operations;
            if let Some(message) = self.text.read(cx).message() {
                self.status = message.to_string();
            }
        }
    }

    /// The text viewer ran a save, a reload or a keep-mine, from its own bar or the rendered side's:
    /// its message shows, and after a reload that read the file the rendered side follows the text.
    fn follow_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.show_text_message(cx);
        let reloads = self.text.read(cx).reloads();
        if reloads != self.seen_reloads {
            self.seen_reloads = reloads;
            let source = Self::source_of(&self.text, cx);
            self.size = source.len();
            self.too_big = self.size > MAX_RENDERED_BYTES;
            if self.too_big {
                self.raw_shown = true;
                self.status = too_big_message(self.size);
                cx.notify();
                return;
            }
            self.render_source(source, None, cx);
            if self.text.read(cx).message().is_none() {
                self.status = "reloaded".to_string();
            }
        }
        cx.notify();
    }

    /// Reload through the text viewer, so C-008's rules hold (edits dropped, an unreadable file kept
    /// and marked modified); the rendered side follows.
    fn on_reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.clone();
        text.update(cx, |text, cx| text.reload(window, cx));
        self.follow_text(window, cx);
    }

    /// Keep mine through the text viewer: the tab counts as modified and the next save overwrites (C-008).
    fn on_keep_mine(&mut self, cx: &mut Context<Self>) {
        self.text.update(cx, |text, cx| text.keep_mine(cx));
        self.seen_operations = self.text.read(cx).operations();
        self.status = "kept yours; the next save overwrites the file".to_string();
        cx.notify();
    }

    /// Follow a link the rendered text was clicked on: an address opens outside, a file opens in
    /// Sfoglia, a fragment scrolls to its heading.
    pub fn follow_link(&mut self, href: &str, window: &mut Window, cx: &mut Context<Self>) {
        match markdown::link_target(&self.folder, href) {
            LinkTarget::Outside(url) => {
                cx.open_url(&url);
                self.status = format!("opened {url}");
            }
            LinkTarget::File { path, fragment } => {
                self.status = format!("opening {}", path.display());
                // After this update ends: the workspace then reads every Markdown view, this one included.
                let focus = self.focus_handle.clone();
                window.defer(cx, move |window, cx| focus.dispatch_action(&OpenLinkedFile { path, fragment }, window, cx));
            }
            LinkTarget::Heading(fragment) => self.reveal_heading(&fragment, window, cx),
            LinkTarget::None => {}
        }
        cx.notify();
    }

    /// Scroll the rendered view to the heading `fragment` names, or say there is none. While the raw
    /// text shows, the cursor goes to the heading's line in the text as it stands (edited, or past the
    /// budget and so never rendered), found by one parse off the window thread, so the next toggle
    /// lands there too.
    pub fn reveal_heading(&mut self, fragment: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.raw_shown {
            let source = Self::source_of(&self.text, cx);
            let fragment = fragment.to_string();
            let searched = source.clone();
            self.finding = Some(cx.spawn_in(window, async move |this, cx| {
                let headings = cx.background_spawn(async move { markdown::outline(&source).headings }).await;
                this.update_in(cx, |this, window, cx| {
                    this.finding = None;
                    if !this.raw_shown {
                        // The user went back to the rendered side meanwhile: nothing to place.
                        return;
                    }
                    if Self::source_of(&this.text, cx) != searched {
                        // The text changed meanwhile (an edit, a reload): search the text as it stands now.
                        this.reveal_heading(&fragment, window, cx);
                        return;
                    }
                    match markdown::heading_for_fragment(&headings, &fragment) {
                        Some(heading) => {
                            this.status = format!("at {}", heading.text);
                            let editor = this.text.read(cx).editor().clone();
                            let position = Position::new(heading.line.saturating_sub(1) as u32, 0);
                            editor.update(cx, |editor, cx| editor.set_cursor_position(position, window, cx));
                        }
                        None => this.status = format!("no heading named #{}", fragment.trim_start_matches('#')),
                    }
                    cx.notify();
                })
                .ok();
            }));
            cx.notify();
            return;
        }
        match self.outline.heading_for_fragment(fragment).map(|h| (h.span.clone(), h.text.clone())) {
            Some((span, text)) => {
                self.status = format!("at {text}");
                self.reveal_source(span, cx);
            }
            None => self.status = format!("no heading named #{}", fragment.trim_start_matches('#')),
        }
        cx.notify();
    }

    // ---- what the workspace and the tests read ----

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn raw_shown(&self) -> bool {
        self.raw_shown
    }

    /// The raw text viewer (C-005, C-008).
    pub fn text_view(&self) -> &Entity<TextView> {
        &self.text
    }

    /// The kit's rich text state: the rendered side.
    pub fn rich_text(&self) -> &Entity<TextViewState> {
        &self.rich
    }

    /// The blocks of the source last rendered.
    pub fn outline(&self) -> &Outline {
        &self.outline
    }

    /// Whether the text is still being prepared for the renderer.
    pub fn is_preparing(&self) -> bool {
        self.preparing.is_some()
    }

    /// Whether the raw text has edits not yet saved (C-008).
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.text.read(cx).is_dirty()
    }

    /// Whether another program changed the file since it was read or written (C-008).
    pub fn changed_on_disk(&self, cx: &App) -> bool {
        self.text.read(cx).changed_on_disk()
    }

    /// The image files the renderer has asked for so far, in order.
    pub fn requested_images(&self) -> Vec<PathBuf> {
        self.requested.lock().map(|list| list.clone()).unwrap_or_default()
    }
}

/// `3.1 MB`, as the status row sizes a file.
fn size_label(bytes: usize) -> String {
    if bytes >= 1024 * 1024 { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) } else { format!("{} KB", bytes.div_ceil(1024)) }
}

fn too_big_message(bytes: usize) -> String {
    format!(
        "{} of Markdown opens as text: the rendered view is kept for files under {} MB",
        size_label(bytes),
        MAX_RENDERED_BYTES / (1024 * 1024)
    )
}

/// An image's bytes for the kit, from where C-011 says they come.
fn to_image_source(src: ImageSrc) -> ImageSource {
    let blank = || ImageSource::Image(Arc::new(Image::from_bytes(ImageFormat::Png, blank_png().to_vec())));
    match src {
        ImageSrc::File(path) => ImageSource::Resource(Resource::Path(Arc::from(path))),
        ImageSrc::Remote(url) => ImageSource::Resource(Resource::Uri(SharedUri::from(url))),
        ImageSrc::Data { mime, bytes } => match image_format(&mime) {
            Some(format) => ImageSource::Image(Arc::new(Image::from_bytes(format, bytes))),
            None => blank(),
        },
        ImageSrc::Invalid => blank(),
    }
}

fn image_format(mime: &str) -> Option<ImageFormat> {
    Some(match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::Webp,
        "image/gif" => ImageFormat::Gif,
        "image/svg+xml" => ImageFormat::Svg,
        "image/bmp" => ImageFormat::Bmp,
        "image/tiff" => ImageFormat::Tiff,
        "image/x-icon" | "image/vnd.microsoft.icon" => ImageFormat::Ico,
        _ => return None,
    })
}

impl Focusable for MarkdownView {
    /// The dock focuses the view through this: the raw text hands it to the editor.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.raw_shown { self.text.read(cx).editor().read(cx).focus_handle(cx) } else { self.focus_handle.clone() }
    }
}

impl Render for MarkdownView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.show_text_message(cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let warning = theme.warning.opacity(0.15);
        let summary = if self.too_big {
            format!("Markdown, {}", size_label(self.size))
        } else if self.preparing.is_some() {
            "Markdown, preparing".to_string()
        } else {
            self.outline.summary()
        };
        let status = if self.status.is_empty() {
            if self.raw_shown {
                "Ctrl+Shift+D shows the rendered view".to_string()
            } else {
                "Ctrl+Shift+D shows the text, a click follows a link".to_string()
            }
        } else {
            self.status.clone()
        };
        let changed = !self.raw_shown && self.changed_on_disk(cx);
        let folder = self.folder.clone();
        let requested = self.requested.clone();
        let view = cx.entity().downgrade();
        let body = if self.raw_shown {
            div().id("markdown-raw").size_full().min_h_0().child(self.text.clone()).into_any_element()
        } else {
            v_flex()
                .size_full()
                .min_h_0()
                .when(changed, |view| {
                    view.child(
                        h_flex()
                            .id("markdown-changed-on-disk")
                            .test_support()
                            .role(Role::Status)
                            .aria_label("The file changed on disk.")
                            .flex_shrink_0()
                            .gap_3()
                            .px_3()
                            .py_1()
                            .bg(warning)
                            .border_b_1()
                            .border_color(border)
                            .text_sm()
                            .child("The file changed on disk.")
                            .child(Button::new("markdown-reload").label("Reload").xsmall().on_click(cx.listener(|this, _, window, cx| this.on_reload(window, cx))))
                            .child(Button::new("markdown-keep-mine").label("Keep mine").xsmall().on_click(cx.listener(|this, _, _, cx| this.on_keep_mine(cx)))),
                    )
                })
                .child(
                    div()
                        .id("markdown-rendered")
                        .test_support()
                        .aria_label(summary.clone())
                        .flex_1()
                        .min_h_0()
                        .px_4()
                        .py_2()
                        .child(
                            RichText::new(&self.rich)
                                .scrollable(true)
                                .image_source(move |uri: &SharedUri| {
                                    let src = markdown::image_source(&folder, &uri.to_string());
                                    if let ImageSrc::File(path) = &src
                                        && let Ok(mut list) = requested.lock()
                                    {
                                        list.push(path.clone());
                                    }
                                    to_image_source(src)
                                })
                                .on_link_click(move |href: &SharedString, _: &ClickEvent, window: &mut Window, cx: &mut App| {
                                    let href = href.to_string();
                                    let _ = view.update(cx, |this, cx| this.follow_link(&href, window, cx));
                                }),
                        ),
                )
                .into_any_element()
        };
        v_flex()
            .id("markdown-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_toggle_raw))
            .size_full()
            .child(body)
            .child(
                h_flex()
                    .id("markdown-status")
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
                    .child(div().id("markdown-summary").test_support().aria_label(summary.clone()).child(summary))
                    .child(div().id("markdown-message").test_support().aria_label(status.clone()).flex_1().overflow_hidden().child(status)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_images_take_their_format_from_the_media_type_or_a_blank() {
        assert_eq!(image_format("image/png"), Some(ImageFormat::Png));
        assert_eq!(image_format("image/jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(image_format("image/svg+xml"), Some(ImageFormat::Svg));
        assert_eq!(image_format("image/avif"), None);
        match to_image_source(ImageSrc::Data { mime: "image/gif".into(), bytes: vec![1, 2, 3] }) {
            ImageSource::Image(image) => assert_eq!((image.format, image.bytes.as_slice()), (ImageFormat::Gif, &[1u8, 2, 3][..])),
            _ => panic!("a data image is an in-memory image"),
        }
        // A media type the kit lacks, or an unreadable address, shows the blank 1x1 PNG.
        for src in [ImageSrc::Data { mime: "image/avif".into(), bytes: vec![1] }, ImageSrc::Invalid] {
            match to_image_source(src) {
                ImageSource::Image(image) => {
                    assert_eq!(image.format, ImageFormat::Png);
                    let decoded = image::load_from_memory(&image.bytes).expect("the blank PNG decodes");
                    assert_eq!((decoded.width(), decoded.height()), (1, 1));
                }
                _ => panic!("the blank is an in-memory image"),
            }
        }
        let folder = std::path::Path::new("x");
        assert!(matches!(to_image_source(markdown::image_source(folder, "https://example.com/a.png")), ImageSource::Resource(Resource::Uri(_))));
        assert!(matches!(to_image_source(markdown::image_source(folder, "a.png")), ImageSource::Resource(Resource::Path(_))));
    }
}
