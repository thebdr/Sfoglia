//! The text viewer (C-005): a read-only code editor with line numbers and syntax
//! highlighting, and a status row naming the encoding, line endings, line count and
//! language.

use gpui_kit::component::input::{Editor, EditorState, RopeExt as _};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, IntoElement, Render, Role, SharedString, TestSupportExt, Window, div,
    relative,
};

use crate::text::{Decoded, Encoding, LineEndings, decode, language_for};

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

pub struct TextView {
    editor: Entity<EditorState>,
    status: TextStatus,
}

impl TextView {
    /// Decode `bytes` as the file named `name` and put the text in a read-only editor.
    pub fn new(name: &str, bytes: &[u8], window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Decoded { text, encoding, line_endings, lines, replaced } = decode(bytes);
        let language = language_for(name).unwrap_or("text");
        let shown = display_text(&text, line_endings);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .soft_wrap(false)
                .default_value(shown)
        });
        Self {
            editor,
            status: TextStatus { encoding, line_endings, lines, replaced, language },
        }
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn status(&self) -> &TextStatus {
        &self.status
    }

    /// The text as the editor holds it.
    pub fn text(&self, cx: &App) -> SharedString {
        self.editor.read(cx).value()
    }

    /// The number of lines the editor lays out, which must agree with the status row.
    pub fn editor_lines(&self, cx: &App) -> usize {
        self.editor.read(cx).text().lines_len()
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

fn status_item(id: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    let value = value.into();
    div()
        .id(id)
        .role(Role::Status)
        .test_support()
        .aria_label(value.clone())
        .child(value)
}

impl Render for TextView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.status;
        let replaced = (s.replaced > 0).then(|| format!("{} undecodable", s.replaced));
        v_flex()
            .size_full()
            .child(
                Editor::new(&self.editor)
                    .readonly(true)
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
                    .child(status_item("text-encoding", s.encoding.label()))
                    .child(status_item("text-line-endings", s.line_endings.label()))
                    .child(status_item("text-lines", format!("{} lines", s.lines)))
                    .child(status_item("text-language", s.language))
                    .children(replaced.map(|r| status_item("text-replaced", r))),
            )
    }
}
