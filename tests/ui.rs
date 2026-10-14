//! C-001: the application shell opens a window that shows the application name.
//!
//! Headless UI integration test: a real gpui-kit window is rendered and the
//! observed element tree is queried through its accessibility properties. The
//! pinned GPUI exposes no rendered text, so the drawn text is pinned indirectly:
//! the element must be exactly as wide and as tall as the application name
//! shaped in the window's text style. A string of a different width, or an empty
//! one, fails; a same-width variant such as a case change is not distinguishable
//! in-process and is covered only by the manual capture in the evidence log.

use gpui_kit::component::ActiveTheme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext as _, TestAppContext, TextRun, font, px};
use sfoglia::{APP_NAME, Shell, window_options};

#[gpui_kit::test]
fn shell_window_shows_the_application_name(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (handle, _shell) = cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |_, cx| cx.new(|_| Shell))
            .expect("open the shell window headlessly")
    });

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);

        let title = window.find("shell-title");
        assert_eq!(title.label(), Some(APP_NAME));
        assert!(title.visible(), "the application name is visible");

        // The element is exactly one line of the application name in the text style the
        // gpui-kit root applies: the theme's font family at the window's default size.
        let style = window.text_style();
        let font_size = style.font_size.to_pixels(window.rem_size());
        let run = TextRun {
            len: APP_NAME.len(),
            font: font(cx.theme().font_family.clone()),
            color: style.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let shaped = window
            .text_system()
            .shape_line(APP_NAME.into(), font_size, &[run], None);
        let bounds = title.bounds();
        assert!(shaped.width > px(20.), "shaped width {:?}", shaped.width);
        assert!(
            (bounds.size.width - shaped.width).abs() <= px(1.),
            "element width {:?} vs shaped {:?}",
            bounds.size.width,
            shaped.width
        );
        assert!(
            (bounds.size.height - window.line_height()).abs() <= px(1.),
            "element height {:?} vs line height {:?}",
            bounds.size.height,
            window.line_height()
        );

        // Laid out at the centre of the window.
        let viewport = window.viewport_size();
        let centre = bounds.center();
        assert!((centre.x - viewport.width * 0.5).abs() <= px(2.), "{centre:?} vs {viewport:?}");
        assert!((centre.y - viewport.height * 0.5).abs() <= px(2.), "{centre:?} vs {viewport:?}");
        assert_eq!(window.try_find("missing-element").map(|_| ()), None);
    })
    .unwrap();
}
