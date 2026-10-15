//! The highlighting rules (C-006): the rule file as application state, and the dock
//! panel that edits it. Every change is parsed, applied to the open files and written
//! to the file at once.

use std::path::{Path, PathBuf};

use gpui_kit::component::dock::{BasePanel, Panel as KitPanel, PanelEvent};
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Role,
    SharedString, Subscription, TestSupportExt, Window, div, relative,
};

use crate::rules::{RuleFile, STARTER};

/// The rule file as the application holds it: its text, what it parsed to, and where
/// it is written. Views observe this entity and re-highlight when it notifies.
pub struct RulesStore {
    path: Option<PathBuf>,
    source: String,
    file: RuleFile,
    write_error: Option<String>,
}

impl RulesStore {
    /// Read the rule file at `path`. A missing file starts from the commented example
    /// without writing it; an unreadable one is reported as a write error.
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut write_error = None;
        let source = match path.as_deref().map(std::fs::read_to_string) {
            Some(Ok(source)) => source,
            Some(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => STARTER.to_string(),
            Some(Err(error)) => {
                write_error = Some(format!("cannot read the rule file: {error}"));
                STARTER.to_string()
            }
            None => {
                write_error = Some("no configuration directory: rules are not saved".into());
                STARTER.to_string()
            }
        };
        let file = RuleFile::parse(&source);
        Self { path, source, file, write_error }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn file(&self) -> &RuleFile {
        &self.file
    }

    /// Why the last write failed, if it did.
    pub fn write_error(&self) -> Option<&str> {
        self.write_error.as_deref()
    }

    /// Replace the file's text: parse it, write it, and tell the observers.
    pub fn set_source(&mut self, source: String, cx: &mut Context<Self>) {
        if source == self.source {
            return;
        }
        self.file = RuleFile::parse(&source);
        self.source = source;
        self.write_error = match &self.path {
            Some(path) => {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|_| std::fs::write(path, &self.source));
                written.err().map(|error| format!("cannot write {}: {error}", path.display()))
            }
            None => Some("no configuration directory: rules are not saved".into()),
        };
        cx.notify();
    }
}

/// The dock panel: the rule file in an editor, the problems under it.
pub struct RulesPanel {
    focus_handle: FocusHandle,
    store: Entity<RulesStore>,
    editor: Entity<EditorState>,
    _subscriptions: Vec<Subscription>,
}

impl RulesPanel {
    pub fn new(store: &Entity<RulesStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = store.read(cx).source().to_string();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("toml")
                .line_number(true)
                .soft_wrap(false)
                .default_value(source)
        });
        let store_for_edits = store.clone();
        let on_change = cx.subscribe_in(&editor, window, move |_, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let source = state.read(cx).value().to_string();
                store_for_edits.update(cx, |store, cx| store.set_source(source, cx));
            }
        });
        let on_store = cx.observe(store, |_, _, cx| cx.notify());
        Self {
            focus_handle: cx.focus_handle(),
            store: store.clone(),
            editor,
            _subscriptions: vec![on_change, on_store],
        }
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn store(&self) -> &Entity<RulesStore> {
        &self.store
    }
}

impl BasePanel for RulesPanel {
    fn panel_name(&self) -> &'static str {
        "sfoglia-rules"
    }
}

impl KitPanel for RulesPanel {
    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some("Highlighting rules".into())
    }

    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        SharedString::from("Highlighting rules")
    }
}

impl EventEmitter<PanelEvent> for RulesPanel {}

impl Focusable for RulesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RulesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let path = store
            .path()
            .map_or_else(|| "not saved".to_string(), |p| p.display().to_string());
        let problems: Vec<String> = store.file().problems.iter().map(|p| p.label()).collect();
        let problems_label: SharedString = if problems.is_empty() {
            "No problems.".into()
        } else {
            problems.join("\n").into()
        };
        let write_error = store.write_error().map(str::to_string);
        v_flex()
            .id("rules-panel")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_3()
                    .py_1()
                    .gap_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Saved on every change.")
                    .child(div().id("rules-path").test_support().aria_label(path.clone()).child(path)),
            )
            .child(
                Editor::new(&self.editor)
                    .bordered(false)
                    .aria_label("rules-text")
                    .h(relative(1.))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size),
            )
            .child(
                v_flex()
                    .id("rules-problems")
                    .role(Role::Status)
                    .test_support()
                    .aria_label(problems_label.clone())
                    .flex_shrink_0()
                    .max_h_48()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .when(problems.is_empty(), |this| {
                        this.text_color(cx.theme().muted_foreground).child(problems_label)
                    })
                    .when(!problems.is_empty(), |this| {
                        this.text_color(cx.theme().danger)
                            .children(problems.into_iter().map(|p| div().child(p)))
                    })
                    .children(write_error.map(|error| {
                        div()
                            .id("rules-write-error")
                            .role(Role::Status)
                            .test_support()
                            .aria_label(error.clone())
                            .text_color(cx.theme().warning)
                            .child(error)
                    })),
            )
    }
}
