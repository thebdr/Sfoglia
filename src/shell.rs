//! The application shell: the native window every viewer lives in (C-001).

use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, IntoElement, Role, SharedString, TestSupportExt, TitlebarOptions, WindowBounds,
    WindowOptions, div, point, px, size,
};

/// The name shown in the window title and, until a file is open, in the window body.
pub const APP_NAME: &str = "Sfoglia";

/// A one-line label whose accessibility name is the very string it renders, so a
/// headless test that reads the name reads what is drawn.
pub(crate) fn title(text: impl Into<SharedString>) -> impl IntoElement {
    let text = text.into();
    div()
        .id("shell-title")
        .role(Role::Label)
        .test_support()
        .aria_label(text.clone())
        .child(text)
}

/// Options for the main window: titled with the application name, 1200 x 800 logical pixels.
pub fn window_options() -> WindowOptions {
    WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some(APP_NAME.into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(80.), px(80.)),
            size(px(1200.), px(800.)),
        ))),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_window_is_titled_with_the_application_name() {
        let options = window_options();
        let titlebar = options.titlebar.expect("the main window has a title bar");
        assert_eq!(titlebar.title.as_deref(), Some("Sfoglia"));
    }

    #[test]
    fn main_window_opens_at_a_usable_size() {
        let options = window_options();
        match options.window_bounds {
            Some(WindowBounds::Windowed(bounds)) => {
                assert_eq!(bounds.size, size(px(1200.), px(800.)));
            }
            other => panic!("expected windowed bounds, got {other:?}"),
        }
    }
}
