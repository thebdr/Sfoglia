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

use std::time::Instant;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{ClosePanel, Panel as _};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, TextRun, font, px,
};
use sfoglia::text::{Encoding, LineEndings};
use sfoglia::workspace::{FileInfo, FilePanel};
use sfoglia::{APP_NAME, ToggleTheme, Workspace, window_options};

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

        // Reopening the first file activates its tab; nothing is added. A text file shows
        // the text viewer (C-005) rather than the info rows.
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).open_files(cx).len(), 2);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("hello.txt".into()));
        assert!(window.try_find("file-viewer").is_none(), "a text file shows the editor, not the info rows");
        assert_eq!(window.find("text-language").label(), Some("text"));
        let hello_view = workspace.read(cx).panels(cx)[0].read(cx).text_view().cloned().unwrap();
        assert_eq!(hello_view.read(cx).text(cx).as_ref(), "Sfoglia reads anything.\nSecond line.\n");

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

        // Closing the active tab through the dock's own Close action forgets the file;
        // opening it again adds a fresh tab and activates it.
        workspace.update(cx, |w, cx| w.open_paths(&[dot.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("dot.png".into()));
        let dot_panel = workspace.read(cx).panels(cx)[1].clone();
        assert_eq!(dot_panel.read(cx).info().name, "dot.png");
        let focus = dot_panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        assert!(focus.is_focused(window), "the active panel holds focus");
        window.dispatch_action(Box::new(ClosePanel), cx);
        window.render_frame(cx);

        // The tab is named after the file, not after the panel kind. The kit draws the
        // tab bar from these names; the drawn tab bar itself is checked in the captures.
        let panel = cx.new(|cx| FilePanel::new(FileInfo::probe(&hello), window, cx));
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("hello.txt"));
        let panel = cx.new(|cx| FilePanel::new(FileInfo::probe(&missing), window, cx));
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("does-not-exist.txt"));
    })
    .unwrap();

    // The dock applies a close through an event, delivered once the update above ends.
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["hello.txt", "renamed-png.txt", "does-not-exist.txt"]);
        assert_ne!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("dot.png".into()));
        workspace.update(cx, |w, cx| w.open_paths(&[dot.clone()], window, cx));
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["hello.txt", "renamed-png.txt", "does-not-exist.txt", "dot.png"]);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("dot.png".into()));
        assert_eq!(window.find("file-name").label(), Some("dot.png"));
        assert_eq!(workspace.read(cx).panels(cx).len(), 4);
    })
    .unwrap();
}

#[gpui_kit::test]
fn text_files_open_in_the_editor_with_their_status_and_theme_switches(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let sample = fixture("sample.rs");
    let latin1 = fixture("latin1.txt");
    let dot = fixture("dot.png");
    let expected = std::fs::read_to_string(&sample).unwrap();

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[sample.clone()], window, cx));
        window.render_frame(cx);

        // The Rust file is in the editor, exactly, with its status row.
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let view = panel.read(cx).text_view().expect("a text file opens in the text viewer").clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), expected);
        let status = view.read(cx).status().clone();
        assert_eq!(status.encoding, Encoding::Utf8);
        assert_eq!(status.line_endings, LineEndings::Lf);
        assert_eq!(status.lines, 6);
        assert_eq!(status.language, "rust");
        assert_eq!(window.find("text-encoding").label(), Some("UTF-8"));
        assert_eq!(window.find("text-line-endings").label(), Some("LF"));
        assert_eq!(window.find("text-lines").label(), Some("6 lines"));
        assert_eq!(window.find("text-language").label(), Some("rust"));
        assert!(window.try_find("text-replaced").is_none());
        assert!(window.try_find("file-viewer").is_none(), "the info rows give way to the editor");
        // The status row sits at the bottom of the panel, below an editor that fills the rest.
        let status_row = window.find("text-status");
        let viewport = window.viewport_size();
        assert!(status_row.bounds().bottom() > viewport.height - px(60.), "{:?}", status_row.bounds());
        assert!(status_row.bounds().top() > viewport.height * 0.5, "{:?}", status_row.bounds());

        // Latin-1 and a binary file: the first is text in the editor, the second stays an info tab.
        workspace.update(cx, |w, cx| w.open_paths(&[latin1.clone(), dot.clone()], window, cx));
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        let latin = panels[1].read(cx).text_view().expect("latin-1 is text").clone();
        assert_eq!(latin.read(cx).text(cx).as_ref(), "caffè latte à Roma\n");
        assert_eq!(latin.read(cx).status().encoding, Encoding::Latin1);
        assert_eq!(latin.read(cx).status().language, "text");
        assert!(panels[2].read(cx).text_view().is_none(), "a PNG is not text");
        assert_eq!(window.find("file-viewer").label(), Some("images"));

    })
    .unwrap();

    // The theme flips on the action and back again. The change lands once the window
    // update ends, like any other deferred effect.
    let was_dark = cx.update(|cx| cx.theme().is_dark());
    cx.update_window(handle, |_, window, cx| window.dispatch_action(Box::new(ToggleTheme), cx))
        .unwrap();
    cx.run_until_parked();
    assert_ne!(cx.update(|cx| cx.theme().is_dark()), was_dark, "Ctrl+Shift+T switches the theme");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.dispatch_action(Box::new(ToggleTheme), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.theme().is_dark()), was_dark);
}

#[gpui_kit::test]
fn three_hundred_thousand_lines_render_a_frame_within_budget(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let big: String = (0..300_000).map(|i| format!("line {i}: the quick brown fox jumps over the lazy dog\n")).collect();
    let path = std::env::temp_dir().join(format!("sfoglia-big-{}.txt", std::process::id()));
    std::fs::write(&path, &big).unwrap();

    cx.update_window(handle, |_, window, cx| {
        let opened = Instant::now();
        workspace.update(cx, |w, cx| w.open_paths(&[path.clone()], window, cx));
        window.render_frame(cx);
        let first = opened.elapsed();
        let timed = Instant::now();
        window.render_frame(cx);
        let second = timed.elapsed();
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).status().lines, 300_001);
        assert_eq!(window.find("text-lines").label(), Some("300001 lines"));
        println!("300k lines: open+first frame {first:?}, steady frame {second:?}");
        assert!(second.as_millis() < 250, "a steady frame took {second:?}");
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}
