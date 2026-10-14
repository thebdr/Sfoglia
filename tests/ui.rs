//! Headless UI integration tests: a real gpui-kit window is rendered and the observed
//! element tree is queried through its accessibility properties.
//!
//! C-001: the window shows the application name when nothing is open. The pinned
//! GPUI exposes no rendered text, so the drawn text is pinned indirectly: the element
//! must be exactly as wide and as tall as the application name shaped in the window's
//! text style. A string of a different width, or an empty one, fails; a same-width
//! variant such as a case change is not distinguishable in-process and is covered only
//! by the manual capture in the evidence log.
//!
//! C-004: opening files adds one tab each in the centre of the dock, and opening a file
//! that is already open activates its tab instead of adding another.

use std::path::PathBuf;

use gpui_kit::component::ActiveTheme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, TextRun, font, px};
use sfoglia::{APP_NAME, Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(gpui_kit::init);
    cx.update(sfoglia::init);
    cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(window, cx))
        })
        .expect("open the main window headlessly")
    })
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[gpui_kit::test]
fn shell_window_shows_the_application_name(cx: &mut TestAppContext) {
    let (handle, _workspace) = open_workspace(cx);

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

#[gpui_kit::test]
fn opening_files_adds_tabs_and_reopening_activates_the_existing_tab(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let hello = fixture("hello.txt");
    let dot = fixture("dot.png");
    let renamed = fixture("renamed-png.txt");

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("welcome").is_some(), "nothing open: the welcome shows");
        assert!(window.try_find("file-viewer").is_none());

        // Two files: two tabs; the last opened is the active one and is what renders.
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), dot.clone()], window, cx));
        window.render_frame(cx);
        assert!(window.try_find("welcome").is_none(), "something open: no welcome");
        let open = workspace.read(cx).open_files(cx);
        assert_eq!(
            open.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["hello.txt", "dot.png"]
        );
        assert_eq!(open[0].detection.route.id(), "text");
        assert_eq!(open[1].detection.route.id(), "images");
        assert_eq!(open[0].size, Some(37));
        assert_eq!(open[1].size, Some(67));
        assert_eq!(open[0].error, None);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("dot.png".into()));
        assert_eq!(window.find("file-viewer").label(), Some("images"));
        assert_eq!(window.find("file-name").label(), Some("dot.png"));
        assert_eq!(window.find("file-size").label(), Some("67 bytes"));
        assert_eq!(window.find("file-family").label(), Some("Images (images)"));

        // Reopening the first file activates its tab; nothing is added.
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).open_files(cx).len(), 2);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("hello.txt".into()));
        assert_eq!(window.find("file-viewer").label(), Some("text"));
        assert_eq!(window.find("file-name").label(), Some("hello.txt"));

        // A misleading extension is routed by content; a missing file still opens a tab.
        let missing = fixture("does-not-exist.txt");
        workspace.update(cx, |w, cx| w.open_paths(&[renamed.clone(), missing.clone()], window, cx));
        window.render_frame(cx);
        let open = workspace.read(cx).open_files(cx);
        assert_eq!(open.len(), 4);
        assert_eq!(open[2].detection.route.id(), "images");
        assert_eq!(open[3].detection.route.id(), "text");
        assert!(open[3].error.is_some(), "the missing file reports its error");
        assert_eq!(open[3].size, None);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("does-not-exist.txt".into()));
        let status = window.find("file-status").label().unwrap_or_default().to_string();
        assert!(status.starts_with("Cannot read this file:"), "{status}");
    })
    .unwrap();
}
