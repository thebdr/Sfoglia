//! The text viewer (C-005): a read-only code editor with line numbers and syntax
//! highlighting, and a status row naming the encoding, line endings, line count and
//! language. The user's highlighting rules (C-006) paint over the syntax colours as
//! editor decorations and follow every change to the rule file.

use std::ops::Range;

use gpui_kit::component::input::{
    Editor, EditorState, RopeExt as _, TextDecoration, TextDecorationCollection,
};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, FontStyle, FontWeight, HighlightStyle, Hsla, IntoElement, Render, Rgba,
    Role, SharedString, Subscription, TestSupportExt, UnderlineStyle, Window, div, px, relative,
};

use crate::rules::{Color, Span, Style};
use crate::text::{Decoded, Encoding, LineEndings, decode, language_for};
use crate::viewers::RulesStore;

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
    name: String,
    editor: Entity<EditorState>,
    status: TextStatus,
    rules: Entity<RulesStore>,
    decorations: TextDecorationCollection,
    /// What the collection was last given: one decoration per span, in span order.
    painted: Vec<TextDecoration>,
    spans: Vec<Span>,
    sets: Vec<String>,
    _observe_rules: Subscription,
}

impl TextView {
    /// Decode `bytes` as the file named `name` and put the text in a read-only editor,
    /// painted by the rule sets that name the file.
    pub fn new(
        name: &str,
        bytes: &[u8],
        rules: &Entity<RulesStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
        let decorations =
            editor.update(cx, |state, cx| state.create_decorations_collection(Vec::new(), cx));
        let observe = cx.observe(rules, |this, _, cx| this.apply_rules(cx));
        let mut view = Self {
            name: name.to_string(),
            editor,
            status: TextStatus { encoding, line_endings, lines, replaced, language },
            rules: rules.clone(),
            decorations,
            painted: Vec::new(),
            spans: Vec::new(),
            sets: Vec::new(),
            _observe_rules: observe,
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
        let rules = if self.sets.is_empty() {
            "no rules".to_string()
        } else {
            format!("rules: {}", self.sets.join(", "))
        };
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
                    .child(status_item("text-rules", rules))
                    .children(replaced.map(|r| status_item("text-replaced", r))),
            )
    }
}
