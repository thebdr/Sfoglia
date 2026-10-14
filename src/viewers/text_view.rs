//! The text viewer (C-005): a read-only code editor with line numbers and syntax
//! highlighting, and a status row naming the encoding, line endings, line count and
//! language.

use gpui_kit::component::input::{Editor, EditorState};
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
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .soft_wrap(false)
                .default_value(text)
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
