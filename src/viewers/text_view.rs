//! The text viewer (C-005): a code editor with line numbers and syntax highlighting, and
//! a status row naming the encoding, line endings, line count and language. The user's
//! highlighting rules (C-006) paint over the syntax colours as editor decorations and
//! follow every change to the rule file. The text is editable and saved back in the
//! file's own encoding and line endings (C-008); the file on disk is watched for changes
//! made elsewhere.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{
    Editor, EditorState, InputEvent, RopeExt as _, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FontStyle, FontWeight, HighlightStyle, Hsla, IntoElement, Render, Rgba,
    Role, SharedString, Subscription, Task, TestSupportExt, UnderlineStyle, Window, div, px,
    relative,
};

use crate::rules::{Color, Span, Style};
use crate::text::{Decoded, Encoding, LineEndings, decode, encode, language_for};
use crate::viewers::RulesStore;

/// How often the file on disk is compared with what was read (C-008).
pub const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// The encodings a save may use, in the order the status row cycles through them.
pub const ENCODINGS: [Encoding; 6] = [
    Encoding::Utf8,
    Encoding::Utf8Bom,
    Encoding::Utf16Le,
    Encoding::Utf16Be,
    Encoding::Latin1,
    Encoding::Windows1252,
];

/// The line endings a save may use, in the order the status row cycles through them.
pub const LINE_ENDINGS: [LineEndings; 3] = [LineEndings::Lf, LineEndings::CrLf, LineEndings::Cr];

/// What the status row says about the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextStatus {
    pub encoding: Encoding,
    pub line_endings: LineEndings,
    pub lines: usize,
    pub replaced: usize,
    /// The grammar name used for highlighting; `text` when none applies.
    pub language: &'static str,
}

/// The file on disk as last read or written: enough to notice another writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl DiskStamp {
    /// The file's stamp now, or `None` when it cannot be read.
    pub fn of(path: &Path) -> Option<DiskStamp> {
        let meta = std::fs::metadata(path).ok()?;
        Some(DiskStamp { modified: meta.modified().ok(), len: meta.len() })
    }
}

pub struct TextView {
    path: PathBuf,
    name: String,
    editor: Entity<EditorState>,
    status: TextStatus,
    /// The encoding and line endings the next save writes.
    save_encoding: Encoding,
    save_endings: LineEndings,
    /// Whether the text may be edited: a file with undecodable units is read-only.
    editable: bool,
    dirty: bool,
    /// What the last save or failed save said.
    message: Option<String>,
    /// Goes up with every save, reload and keep-mine, so a viewer that mirrors the message sees a
    /// repeated one as new.
    operations: u64,
    /// Goes up with every reload that read the file, so a viewer that mirrors the text knows when
    /// to parse it again.
    reloads: u64,
    on_disk: Option<DiskStamp>,
    changed_on_disk: bool,
    rules: Entity<RulesStore>,
    decorations: TextDecorationCollection,
    /// What the collection was last given: one decoration per span, in span order.
    painted: Vec<TextDecoration>,
    spans: Vec<Span>,
    sets: Vec<String>,
    _subscriptions: Vec<Subscription>,
    _watch: Task<()>,
}

impl TextView {
    /// Decode `bytes` as the file at `path` and put the text in an editor, painted by the
    /// rule sets that name the file; the bytes count as read now.
    pub fn new(
        path: &Path,
        bytes: &[u8],
        rules: &Entity<RulesStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_as_read(path, bytes, DiskStamp::of(path), rules, window, cx)
    }

    /// The same for bytes read earlier, at `read_at`: the disk check then notices a writer that
    /// came between that read and this view (the data viewer builds its text view on first need).
    pub fn new_as_read(
        path: &Path,
        bytes: &[u8],
        read_at: Option<DiskStamp>,
        rules: &Entity<RulesStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let Decoded { text, encoding, line_endings, lines, replaced } = decode(bytes);
        let language = language_for(&name).unwrap_or("text");
        let shown = display_text(&text, line_endings);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .soft_wrap(false)
                .default_value(shown)
        });
        let decorations =
            editor.update(cx, |state, cx| state.create_decorations_collection(Vec::new(), cx));
        let observe = cx.observe(rules, |this, _, cx| this.apply_rules(cx));
        let on_change = cx.subscribe_in(&editor, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.on_edited(cx);
            }
        });
        let watch = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(DISK_CHECK_INTERVAL).await;
                if this.update(cx, |this, cx| this.check_disk(cx)).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            path: path.to_path_buf(),
            name,
            editor,
            status: TextStatus { encoding, line_endings, lines, replaced, language },
            save_encoding: encoding,
            save_endings: line_endings,
            editable: replaced == 0,
            dirty: false,
            message: None,
            operations: 0,
            reloads: 0,
            on_disk: read_at,
            changed_on_disk: false,
            rules: rules.clone(),
            decorations,
            painted: Vec::new(),
            spans: Vec::new(),
            sets: Vec::new(),
            _subscriptions: vec![observe, on_change],
            _watch: watch,
        };
        view.apply_rules(cx);
        view
    }

    /// Recompute the rule spans for the current rule file and hand them to the editor.
    fn apply_rules(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value();
        let rules = self.rules.read(cx);
        let file = rules.file();
        self.sets = file.sets_for(&self.name).into_iter().map(|i| file.sets[i].name.clone()).collect();
        self.spans = file.spans(&self.name, &text);
        self.painted = self
            .spans
            .iter()
            .map(|span| TextDecoration::new(span.range.clone(), highlight_style(&file.rule(span).style)))
            .collect();
        self.decorations.set(self.painted.clone(), cx);
        cx.notify();
    }

    /// The editor's text changed: the tab is modified, the line count follows, and the
    /// rule spans are recomputed.
    fn on_edited(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        self.message = None;
        let rope = self.editor.read(cx).text();
        self.status.lines = if rope.len() == 0 { 0 } else { rope.lines_len() };
        self.apply_rules(cx);
    }

    /// Compare the file on disk with what was last read or written.
    fn check_disk(&mut self, cx: &mut Context<Self>) {
        let now = DiskStamp::of(&self.path);
        if now != self.on_disk && !self.changed_on_disk {
            self.changed_on_disk = true;
            cx.notify();
        }
    }

    /// The text was replaced from outside the editor (a pretty-print, C-010): the tab is
    /// modified as after a keystroke.
    pub fn note_edit(&mut self, cx: &mut Context<Self>) {
        self.on_edited(cx);
    }

    /// The decorations handed to the editor for the current spans, in span order.
    pub fn rule_decorations(&self) -> &[TextDecoration] {
        &self.painted
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn status(&self) -> &TextStatus {
        &self.status
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the text has changes not yet saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Whether the text may be edited.
    pub fn is_editable(&self) -> bool {
        self.editable
    }

    /// Whether another program changed the file since it was read or written.
    pub fn changed_on_disk(&self) -> bool {
        self.changed_on_disk
    }

    /// What the last save, or failed save, said.
    /// How many saves, reloads and keep-mines have run: a repeated message counts as new when this changed.
    pub fn operations(&self) -> u64 {
        self.operations
    }

    /// How many reloads read the file: the text is another one when this changed.
    pub fn reloads(&self) -> u64 {
        self.reloads
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// The encoding the next save writes.
    pub fn save_encoding(&self) -> Encoding {
        self.save_encoding
    }

    /// The line endings the next save writes (`Mixed` and `None` write LF).
    pub fn save_line_endings(&self) -> LineEndings {
        self.save_endings
    }

    /// The text as the editor holds it.
    pub fn text(&self, cx: &App) -> SharedString {
        self.editor.read(cx).value()
    }

    /// The number of lines the editor lays out, which must agree with the status row.
    pub fn editor_lines(&self, cx: &App) -> usize {
        self.editor.read(cx).text().lines_len()
    }

    /// The rule spans painted now, overlaps resolved, in text order.
    pub fn rule_spans(&self) -> &[Span] {
        &self.spans
    }

    /// The names of the rule sets that apply to this file.
    pub fn rule_sets(&self) -> &[String] {
        &self.sets
    }

    /// The byte ranges the editor's decoration layer holds for this view.
    pub fn decoration_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.decorations.get_ranges(cx)
    }

    /// Choose the encoding the next save writes; the tab is modified by the choice.
    pub fn set_save_encoding(&mut self, encoding: Encoding, cx: &mut Context<Self>) {
        if self.save_encoding != encoding {
            self.save_encoding = encoding;
            self.dirty = true;
            self.message = None;
            cx.notify();
        }
    }

    /// Choose the line endings the next save writes; the tab is modified by the choice.
    pub fn set_save_line_endings(&mut self, endings: LineEndings, cx: &mut Context<Self>) {
        if self.save_endings != endings {
            self.save_endings = endings;
            self.dirty = true;
            self.message = None;
            cx.notify();
        }
    }

    fn cycle_encoding(&mut self, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        let at = ENCODINGS.iter().position(|&e| e == self.save_encoding).unwrap_or(0);
        self.set_save_encoding(ENCODINGS[(at + 1) % ENCODINGS.len()], cx);
    }

    fn cycle_line_endings(&mut self, cx: &mut Context<Self>) {
        if !self.editable {
            return;
        }
        let next = match LINE_ENDINGS.iter().position(|&e| e == self.save_endings) {
            Some(at) => LINE_ENDINGS[(at + 1) % LINE_ENDINGS.len()],
            // Mixed or none: the first choice is LF, what such a file is written with anyway.
            None => LineEndings::Lf,
        };
        self.set_save_line_endings(next, cx);
    }

    /// Write the text in the chosen encoding and line endings. An untouched file is left
    /// alone. A character the encoding cannot hold, or a write error, is reported and
    /// the file is not touched.
    pub fn save(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        self.operations += 1;
        if !self.editable {
            let error = "this file has undecodable units and is read-only".to_string();
            self.message = Some(error.clone());
            cx.notify();
            return Err(error);
        }
        if !self.dirty {
            return Ok(());
        }
        let text = self.editor.read(cx).value();
        let bytes = match encode(&text, self.save_encoding, self.save_endings) {
            Ok(bytes) => bytes,
            Err(error) => {
                let message = error.message(self.save_encoding);
                self.message = Some(message.clone());
                cx.notify();
                return Err(message);
            }
        };
        if let Err(error) = std::fs::write(&self.path, &bytes) {
            let message = format!("cannot write {}: {error}", self.path.display());
            self.message = Some(message.clone());
            cx.notify();
            return Err(message);
        }
        self.on_disk = DiskStamp::of(&self.path);
        self.changed_on_disk = false;
        self.dirty = false;
        self.status.encoding = self.save_encoding;
        // The file now holds what was written: the chosen ending, LF for a mixed or
        // break-less file that gained breaks, none when it has no break at all.
        let has_break = text.contains('\n') || text.contains('\r');
        let written = match self.save_endings {
            _ if !has_break => LineEndings::None,
            LineEndings::Mixed | LineEndings::None => LineEndings::Lf,
            chosen => chosen,
        };
        self.status.line_endings = written;
        self.save_endings = written;
        self.message = Some(format!("saved, {} bytes", bytes.len()));
        cx.notify();
        Ok(())
    }

    /// Read the file again and show it, dropping the edits. When the file can no longer
    /// be read (deleted, locked), the text on screen stays, says why, and counts as
    /// modified, so nothing is lost and the next save writes it back.
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.operations += 1;
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let Decoded { text, encoding, line_endings, lines, replaced } = decode(&bytes);
                let shown = display_text(&text, line_endings);
                self.editor.update(cx, |state, cx| state.set_value(shown, window, cx));
                self.status.encoding = encoding;
                self.status.line_endings = line_endings;
                self.status.lines = lines;
                self.status.replaced = replaced;
                self.save_encoding = encoding;
                self.save_endings = line_endings;
                self.editable = replaced == 0;
                self.message = None;
                self.dirty = false;
                self.reloads += 1;
            }
            Err(error) => {
                self.message = Some(format!("cannot read {}: {error}", self.path.display()));
                self.dirty = true;
            }
        }
        self.on_disk = DiskStamp::of(&self.path);
        self.changed_on_disk = false;
        self.apply_rules(cx);
    }

    /// Keep the text on screen although the file changed on disk: it now differs from the
    /// file, so it counts as modified, and the next save overwrites the file.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.operations += 1;
        self.changed_on_disk = false;
        self.on_disk = DiskStamp::of(&self.path);
        self.dirty = true;
        self.message = None;
        cx.notify();
    }
}

fn hsla(colour: Color) -> Hsla {
    Rgba {
        r: f32::from(colour.r) / 255.,
        g: f32::from(colour.g) / 255.,
        b: f32::from(colour.b) / 255.,
        a: f32::from(colour.a) / 255.,
    }
    .into()
}

/// A rule's style as the editor paints it: unset colours leave the syntax colour.
pub fn highlight_style(style: &Style) -> HighlightStyle {
    HighlightStyle {
        color: style.foreground.map(hsla),
        background_color: style.background.map(hsla),
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(FontStyle::Italic),
        underline: style.underline.then(|| UnderlineStyle {
            thickness: px(1.),
            color: style.foreground.map(hsla),
            wavy: false,
        }),
        ..HighlightStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lone_carriage_returns_become_line_feeds_for_display_only() {
        assert_eq!(display_text("alpha\rbeta\rgamma", LineEndings::Cr), "alpha\nbeta\ngamma");
        assert_eq!(display_text("one\r\ntwo\nthree\r\n", LineEndings::Mixed), "one\r\ntwo\nthree\r\n");
        assert_eq!(display_text("a\n\r", LineEndings::Mixed), "a\n\n");
        assert_eq!(display_text("a\r\nb\rc", LineEndings::Mixed), "a\r\nb\nc");
        assert_eq!(display_text("a\r\nb\r\n", LineEndings::CrLf), "a\r\nb\r\n");
        assert_eq!(display_text("a\nb", LineEndings::Lf), "a\nb");
        assert_eq!(display_text("", LineEndings::None), "");
        assert_eq!(display_text("caffè\rlatte", LineEndings::Cr), "caffè\nlatte");
    }

    #[test]
    fn rule_styles_become_editor_styles_field_by_field() {
        let red = Color { r: 255, g: 0, b: 0, a: 255 };
        let style = highlight_style(&Style {
            foreground: Some(red),
            background: None,
            bold: true,
            italic: false,
            underline: true,
        });
        assert_eq!(style.color, Some(hsla(red)));
        assert_eq!(style.background_color, None);
        assert_eq!(style.font_weight, Some(FontWeight::BOLD));
        assert_eq!(style.font_style, None);
        assert_eq!(style.underline.map(|u| (u.thickness, u.color, u.wavy)), Some((px(1.), Some(hsla(red)), false)));
        let plain = highlight_style(&Style::default());
        assert_eq!(plain, HighlightStyle::default(), "an empty style changes nothing");
        let translucent = hsla(Color { r: 0, g: 0, b: 255, a: 128 });
        assert!((translucent.a - 128. / 255.).abs() < 1e-6);
        let italic = highlight_style(&Style { italic: true, ..Style::default() });
        assert_eq!(italic.font_style, Some(FontStyle::Italic));
    }

    #[test]
    fn the_status_row_cycles_through_every_encoding_and_ending() {
        assert_eq!(ENCODINGS.len(), 6);
        assert_eq!(ENCODINGS.iter().map(|e| e.label()).collect::<Vec<_>>(),
            ["UTF-8", "UTF-8 with BOM", "UTF-16 LE", "UTF-16 BE", "Latin-1", "Windows-1252"]);
        assert_eq!(LINE_ENDINGS.iter().map(|e| e.label()).collect::<Vec<_>>(), ["LF", "CRLF", "CR"]);
        assert_eq!(endings_label(LineEndings::Mixed, LineEndings::Mixed), "mixed, saves as LF");
        assert_eq!(endings_label(LineEndings::Lf, LineEndings::Lf), "LF");
        assert_eq!(endings_label(LineEndings::Lf, LineEndings::CrLf), "CRLF (was LF)");
        assert_eq!(endings_label(LineEndings::None, LineEndings::None), "none");
        assert_eq!(encoding_label(Encoding::Latin1, Encoding::Latin1), "Latin-1");
        assert_eq!(encoding_label(Encoding::Latin1, Encoding::Utf8), "UTF-8 (was Latin-1)");
    }
}

/// The text as the editor shows it. The editor breaks lines on LF only, so a lone CR
/// (old Mac endings, or a stray one in a mixed file) becomes LF for display; the status
/// row still reports the file's real endings, and CRLF is left as it is.
pub fn display_text(text: &str, line_endings: LineEndings) -> String {
    if !matches!(line_endings, LineEndings::Cr | LineEndings::Mixed) {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) != Some(&b'\n') {
            out.push(b'\n');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(out).expect("only ASCII CR bytes were replaced")
}

/// What the status row says about the encoding: the one the next save writes, and the
/// one the file was read with when they differ.
pub fn encoding_label(read: Encoding, save: Encoding) -> String {
    if read == save {
        save.label().to_string()
    } else {
        format!("{} (was {})", save.label(), read.label())
    }
}

/// What the status row says about the line endings, likewise; mixed endings say what a
/// save writes.
pub fn endings_label(read: LineEndings, save: LineEndings) -> String {
    match (read, save) {
        (LineEndings::Mixed, LineEndings::Mixed) => "mixed, saves as LF".to_string(),
        (read, save) if read == save => save.label().to_string(),
        (read, save) => format!("{} (was {})", save.label(), read.label()),
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

/// A clickable test-supported wrapper around a small button.
fn small_button(id: &'static str, label: &'static str, on_click: impl Fn(&mut Window, &mut App) + 'static) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(label)
        .child(Button::new(ElementIdSuffix(id)).label(label).xsmall().on_click(move |_, window, cx| on_click(window, cx)))
}

/// A distinct id for the button inside a wrapper that carries the test id.
struct ElementIdSuffix(&'static str);

impl From<ElementIdSuffix> for gpui_kit::ElementId {
    fn from(id: ElementIdSuffix) -> Self {
        (id.0, 1usize).into()
    }
}

impl Render for TextView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.status;
        let replaced = (s.replaced > 0).then(|| format!("{} undecodable, read-only", s.replaced));
        let rules = if self.sets.is_empty() {
            "no rules".to_string()
        } else {
            format!("rules: {}", self.sets.join(", "))
        };
        let encoding = encoding_label(s.encoding, self.save_encoding);
        let endings = endings_label(s.line_endings, self.save_endings);
        let state = if self.dirty { "modified" } else { "saved" };
        let message = self.message.clone();
        let changed = self.changed_on_disk;
        let dirty = self.dirty;
        let this = cx.entity();
        let this_reload = this.clone();
        let this_keep = this.clone();
        let this_revert = this.clone();
        let this_encoding = this.clone();
        let this_endings = this;
        v_flex()
            .size_full()
            .when(changed, |view| {
                view.child(
                    h_flex()
                        .id("text-changed-on-disk")
                        .test_support()
                        .role(Role::Status)
                        .aria_label("The file changed on disk.")
                        .flex_shrink_0()
                        .gap_3()
                        .px_3()
                        .py_1()
                        .bg(cx.theme().warning.opacity(0.15))
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .text_sm()
                        .child("The file changed on disk.")
                        .child(small_button("text-reload", "Reload", move |window, cx| {
                            this_reload.update(cx, |view, cx| view.reload(window, cx));
                        }))
                        .child(small_button("text-keep", "Keep mine", move |_, cx| {
                            this_keep.update(cx, |view, cx| view.keep_mine(cx));
                        })),
                )
            })
            .child(
                Editor::new(&self.editor)
                    .readonly(!self.editable)
                    .bordered(false)
                    .aria_label("file-text")
                    .h(relative(1.))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size),
            )
            .child(
                h_flex()
                    .id("text-status")
                    .test_support()
                    .flex_shrink_0()
                    .gap_4()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .id("text-encoding")
                            .role(Role::Button)
                            .test_support()
                            .aria_label(encoding.clone())
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                this_encoding.update(cx, |view, cx| view.cycle_encoding(cx));
                            })
                            .child(encoding),
                    )
                    .child(
                        div()
                            .id("text-line-endings")
                            .role(Role::Button)
                            .test_support()
                            .aria_label(endings.clone())
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                this_endings.update(cx, |view, cx| view.cycle_line_endings(cx));
                            })
                            .child(endings),
                    )
                    .child(status_item("text-lines", format!("{} lines", s.lines)))
                    .child(status_item("text-language", s.language))
                    .child(status_item("text-rules", rules))
                    .children(replaced.map(|r| status_item("text-replaced", r)))
                    .child(status_item("text-dirty", state))
                    .when(dirty, |row| {
                        row.child(small_button("text-revert", "Revert", move |window, cx| {
                            this_revert.update(cx, |view, cx| view.reload(window, cx));
                        }))
                    })
                    .children(message.map(|m| status_item("text-message", m))),
            )
    }
}
