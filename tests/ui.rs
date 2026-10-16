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
use std::sync::atomic::{AtomicUsize, Ordering};

use std::time::Instant;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dock::{ClosePanel, Panel as _};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Focusable as _, FontWeight, HighlightStyle, Rgba,
    TestAppContext, TextRun, font, point, px,
};
use sfoglia::rules::STARTER;
use sfoglia::viewers::text_view::highlight_style;
use sfoglia::text::{Encoding, LineEndings};
use sfoglia::workspace::{FileInfo, FilePanel};
use sfoglia::{APP_NAME, ToggleHighlightRules, ToggleTheme, Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    open_workspace_with_rules(cx, temp_rules_path())
}

/// A rule file path of this test alone; the file need not exist (the workspace then
/// starts from the example rules without writing them).
fn temp_rules_path() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("sfoglia-rules-{}-{n}.toml", std::process::id()))
}

fn open_workspace_with_rules(cx: &mut TestAppContext, rules_path: PathBuf) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(gpui_kit::init);
    cx.update(sfoglia::init);
    cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |window, cx| {
            cx.new(|cx| Workspace::with_rules_path(Some(rules_path.clone()), window, cx))
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
        // The image opens in the image viewer (C-009) rather than the info rows.
        assert!(workspace.read(cx).panels(cx)[1].read(cx).image_view().is_some());
        assert!(window.try_find("file-viewer").is_none());
        assert_eq!(window.find("image-size").label(), Some("1 × 1 pixels"));
        assert_eq!(window.find("image-bytes").label(), Some("67 bytes"));

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
        let rules = workspace.read(cx).rules().clone();
        let panel = cx.new(|cx| FilePanel::new(FileInfo::probe(&hello), &rules, window, cx));
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("hello.txt"));
        let panel = cx.new(|cx| FilePanel::new(FileInfo::probe(&missing), &rules, window, cx));
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
        assert_eq!(window.find("image-size").label(), Some("1 × 1 pixels"));
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
        assert!(panels[2].read(cx).image_view().is_some(), "a PNG opens in the image viewer");

        // CR-only and mixed endings: the editor lays out as many lines as the status row
        // counts, and the rendered row names the endings.
        let cr = fixture("cr-only.txt");
        let mixed = fixture("mixed-endings.txt");
        workspace.update(cx, |w, cx| w.open_paths(&[cr.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[3].read(cx).text_view().cloned().unwrap();
        assert_eq!(view.read(cx).status().lines, 3);
        assert_eq!(view.read(cx).editor_lines(cx), 3, "the editor breaks on lone CR too");
        assert_eq!(view.read(cx).text(cx).as_ref(), "alpha\nbeta\ngamma");
        assert_eq!(window.find("text-line-endings").label(), Some("CR"));
        assert_eq!(window.find("text-lines").label(), Some("3 lines"));
        workspace.update(cx, |w, cx| w.open_paths(&[mixed.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[4].read(cx).text_view().cloned().unwrap();
        assert_eq!(view.read(cx).status().lines, 4);
        assert_eq!(view.read(cx).editor_lines(cx), 4);
        assert_eq!(window.find("text-line-endings").label(), Some("mixed, saves as LF"));
        assert_eq!(window.find("text-lines").label(), Some("4 lines"));
        assert_eq!(window.find("text-encoding").label(), Some("UTF-8"));
        // The editor accepts edits (C-008); nothing reaches the file until a save.
        let editor = view.read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        assert!(editor.read(cx).is_editable(), "the viewer's editor is editable");
        window.input("XYZ", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), "XYZone\r\ntwo\nthree\r\n");
        assert_eq!(std::fs::read_to_string(&mixed).unwrap(), "one\r\ntwo\nthree\r\n", "the fixture is untouched");

        // Latin-1 source and a Latin-1 file without extension reach the editor too, with
        // the language of their name; the other encodings and endings show their labels.
        let cases: [(&str, &str, &str, &str, &str); 6] = [
            ("latin1.rs", "Latin-1", "LF", "3 lines", "rust"),
            ("latin1-readme", "Latin-1", "LF", "2 lines", "text"),
            ("utf16le.txt", "UTF-16 LE", "CRLF", "2 lines", "text"),
            ("utf16be.txt", "UTF-16 BE", "LF", "2 lines", "text"),
            ("no-final-newline.txt", "UTF-8", "none", "1 lines", "text"),
            ("empty.txt", "UTF-8", "none", "0 lines", "text"),
        ];
        for (i, (name, encoding, endings, lines, language)) in cases.iter().enumerate() {
            workspace.update(cx, |w, cx| w.open_paths(&[fixture(name)], window, cx));
            window.render_frame(cx);
            let view = workspace.read(cx).panels(cx)[5 + i].read(cx).text_view().cloned()
                .unwrap_or_else(|| panic!("{name} opens in the text viewer"));
            assert_eq!(window.find("text-encoding").label(), Some(*encoding), "{name}");
            assert_eq!(window.find("text-line-endings").label(), Some(*endings), "{name}");
            assert_eq!(window.find("text-lines").label(), Some(*lines), "{name}");
            assert_eq!(window.find("text-language").label(), Some(*language), "{name}");
            assert_eq!(view.read(cx).editor_lines(cx).max(1), lines.split(' ').next().unwrap().parse::<usize>().unwrap().max(1), "{name}");
        }
        let latin_rs = workspace.read(cx).panels(cx)[5].read(cx).text_view().cloned().unwrap();
        assert_eq!(latin_rs.read(cx).status().encoding, Encoding::Latin1);
        assert_eq!(latin_rs.read(cx).text(cx).as_ref(), "// caffè latte\nfn main() {}\n");
        let utf16 = workspace.read(cx).panels(cx)[7].read(cx).text_view().cloned().unwrap();
        assert_eq!(utf16.read(cx).text(cx).as_ref(), "hi\r\n");
    })
    .unwrap();

    // The theme flips on the action and back again. The change lands once the window
    // update ends, like any other deferred effect.
    // The real key chord, pressed with the visible editor focused, reaches the workspace action.
    let was_dark = cx.update(|cx| cx.theme().is_dark());
    cx.update_window(handle, |_, window, cx| {
        let active = workspace.read(cx).panels(cx).last().cloned().unwrap();
        let editor = active.read(cx).text_view().cloned().unwrap().read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        assert!(focus.is_focused(window), "the visible editor holds focus");
        window.press("ctrl-shift-t", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_ne!(cx.update(|cx| cx.theme().is_dark()), was_dark, "Ctrl+Shift+T switches the theme");
    // The action itself, dispatched directly, switches back.
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
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        let editor = view.read(cx).editor().clone();
        assert_eq!(view.read(cx).status().lines, 300_001);
        assert_eq!(view.read(cx).editor_lines(cx), 300_001);
        assert_eq!(window.find("text-lines").label(), Some("300001 lines"));

        // Only a screenful of rows is laid out at any depth, and every frame stays cheap:
        // scroll to the top, a quarter, half, three quarters and the end of the file. The
        // editor clamps an overshoot, which gives the real maximum offset.
        editor.update(cx, |state, cx| state.set_scroll_offset(point(px(0.), px(-1.0e9)), cx));
        window.render_frame(cx);
        let max_y = f32::from(editor.read(cx).scroll_offset().y);
        assert!(max_y < -1000.0, "the file scrolls far: max offset {max_y}");
        let mut previous_start = 0usize;
        let mut frames = Vec::new();
        for (i, depth) in [0.0f32, 0.25, 0.5, 0.75, 1.0].into_iter().enumerate() {
            editor.update(cx, |state, cx| state.set_scroll_offset(point(px(0.), px(max_y * depth)), cx));
            // The other headless tests render in parallel threads, so a single frame's wall
            // time can spike; the best of three frames at the same depth is what the budget judges.
            let mut frame = std::time::Duration::MAX;
            for _ in 0..3 {
                let timed = Instant::now();
                window.render_frame(cx);
                frame = frame.min(timed.elapsed());
            }
            let visible = editor.read(cx).visible_row_range().expect("rows are laid out");
            assert!(visible.len() <= 120, "depth {depth}: {visible:?} is not one screenful");
            if depth == 0.0 {
                assert_eq!(visible.start, 0);
                assert!(visible.len() >= 10, "{visible:?}");
            } else if depth == 1.0 {
                assert_eq!(visible.end, 300_001, "the end of the file is reached: {visible:?}");
            } else {
                assert!(visible.len() >= 10, "depth {depth}: {visible:?}");
                let expected = (300_001.0 * depth) as usize;
                assert!(visible.start.abs_diff(expected) < 3_000, "depth {depth}: {visible:?} vs {expected}");
            }
            if i > 0 {
                assert!(visible.start > previous_start, "depth {depth}: {visible:?} after {previous_start}");
            }
            previous_start = visible.start;
            frames.push((depth, frame, visible));
        }
        println!("300k lines: open+first frame {first:?}; scrolled frames {frames:?}");
        for (depth, frame, _) in &frames {
            assert!(frame.as_millis() < 100, "frame at depth {depth} took {frame:?}");
        }
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}

#[gpui_kit::test]
fn long_lines_are_not_wrapped(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let long = "x".repeat(2_000);
    let path = std::env::temp_dir().join(format!("sfoglia-long-{}.txt", std::process::id()));
    std::fs::write(&path, format!("{long}\nshort\n")).unwrap();

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[path.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        let editor = view.read(cx).editor().clone();
        assert_eq!(view.read(cx).status().lines, 3);
        assert_eq!(view.read(cx).editor_lines(cx), 3);
        // Three logical lines, three display rows: the 2000-character line takes one row.
        let visible = editor.read(cx).visible_row_range().expect("rows are laid out");
        assert_eq!(visible, 0..3, "with soft wrap the long line would take many rows");
        assert_eq!(window.find("text-lines").label(), Some("3 lines"));
        // The long line runs past the viewport instead of wrapping, so the editor scrolls
        // sideways: an overshoot to the left clamps to a far negative x offset.
        editor.update(cx, |state, cx| state.set_scroll_offset(point(px(-1.0e6), px(0.)), cx));
        window.render_frame(cx);
        let x = f32::from(editor.read(cx).scroll_offset().x);
        let viewport = f32::from(window.viewport_size().width);
        assert!(x < -viewport, "the 2000-character line scrolls sideways: max x offset {x}, viewport {viewport}");
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}

/// C-006: the rule sets that name a file paint it, through the editor's decorations;
/// typing into the rules panel re-paints at once and writes the file; a bad pattern is
/// reported with its line.
#[gpui_kit::test]
fn rule_spans_reach_the_editor_and_follow_edits_to_the_rule_file(cx: &mut TestAppContext) {
    let rules_path = temp_rules_path();
    std::fs::write(
        &rules_path,
        r##"[[set]]
name = "logs"
files = ["*.log"]

[[set.rule]]
name = "error"
pattern = "ERROR.*"
priority = 1
foreground = "#f00"

[[set.rule]]
name = "number"
pattern = "\\d+"
priority = 5
bold = true
"##,
    )
    .unwrap();
    let (handle, workspace) = open_workspace_with_rules(cx, rules_path.clone());
    let log = fixture("sample.log");
    let text = std::fs::read_to_string(&log).unwrap();
    let painted = |view: &Entity<sfoglia::viewers::TextView>, cx: &gpui_kit::App| -> Vec<(String, String)> {
        let rules = workspace.read(cx).rules().read(cx);
        view.read(cx)
            .rule_spans()
            .iter()
            .map(|s| (text[s.range.clone()].to_string(), rules.file().rule(s).name.clone()))
            .collect()
    };
    let numbers = |values: &[&str]| -> Vec<(String, String)> {
        values.iter().map(|v| (v.to_string(), "number".to_string())).collect()
    };

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("sample.rs"), log.clone()], window, cx));
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        let rs_view = panels[0].read(cx).text_view().unwrap().clone();
        let log_view = panels[1].read(cx).text_view().unwrap().clone();

        // The log is painted by its set, the Rust file by none; the status row says so.
        assert_eq!(log_view.read(cx).rule_sets(), ["logs"]);
        assert_eq!(rs_view.read(cx).rule_sets(), Vec::<String>::new());
        assert!(rs_view.read(cx).rule_spans().is_empty());
        assert!(rs_view.read(cx).decoration_ranges(cx).is_empty());
        assert_eq!(window.find("text-rules").label(), Some("rules: logs"));
        let spans = painted(&log_view, cx);
        let mut expected = numbers(&["2026", "10", "15", "09", "00", "01"]);
        expected.extend(numbers(&["2026", "10", "15", "09", "00", "02", "91"]));
        expected.extend(numbers(&["2026", "10", "15", "09", "00", "03"]));
        expected.extend([
            ("ERROR cannot open plc_".to_string(), "error".to_string()),
            ("7".to_string(), "number".to_string()),
            (".txt: TODO retry".to_string(), "error".to_string()),
        ]);
        assert_eq!(spans, expected);
        // The same ranges are what the editor's decoration layer holds.
        let ranges: Vec<_> = log_view.read(cx).rule_spans().iter().map(|s| s.range.clone()).collect();
        assert_eq!(log_view.read(cx).decoration_ranges(cx), ranges);
        assert_eq!(ranges[19], text.find("ERROR").unwrap()..text.find("plc_").unwrap() + 4);
        // Each decoration carries its rule's style: red for the error rule, bold for numbers.
        let decorations = log_view.read(cx).rule_decorations().to_vec();
        assert_eq!(decorations.len(), ranges.len());
        {
            let rules = workspace.read(cx).rules().read(cx);
            for (decoration, span) in decorations.iter().zip(log_view.read(cx).rule_spans()) {
                assert_eq!(decoration.range, span.range);
                assert_eq!(decoration.style, highlight_style(&rules.file().rule(span).style));
            }
        }
        assert_eq!(decorations[19].style.color, Some(Rgba { r: 1., g: 0., b: 0., a: 1. }.into()));
        assert_eq!(decorations[19].style.font_weight, None);
        assert_eq!(decorations[0].style.font_weight, Some(FontWeight::BOLD));
        assert_eq!(decorations[0].style.color, None);
        assert_ne!(decorations[0].style, HighlightStyle::default());

        // The rules panel starts closed.
        assert!(!workspace.read(cx).rules_panel_open(cx));
        assert!(window.try_find("rules-problems").is_none());
        let editor = log_view.read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("ctrl-shift-h", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).rules_panel_open(cx), "Ctrl+Shift+H opens the rules panel");
        assert_eq!(window.find("rules-problems").label(), Some("No problems."));
        assert_eq!(window.find("rules-path").label().map(str::to_string), Some(rules_path.display().to_string()));
        assert!(window.try_find("rules-write-error").is_none());

        // Type a new set at the top of the file: TODO, at a priority above the error rule.
        let rules_editor = workspace.read(cx).rules_panel().read(cx).editor().clone();
        let focus = rules_editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        assert!(focus.is_focused(window), "the rules editor holds focus");
        for line in ["[[set]]", "files=\"*\"", "[[set.rule]]", "name=\"todo\"", "pattern=\"TODO\"", "priority=9", "background=\"#ff0\"", ""] {
            window.input(line, cx);
            window.press("enter", cx);
        }
        let typed = rules_editor.read(cx).value();
        assert!(typed.starts_with("[[set]]\nfiles=\"*\"\n[[set.rule]]\nname=\"todo\"\npattern=\"TODO\"\npriority=9\nbackground=\"#ff0\"\n\n[[set]]\nname = \"logs\""), "{typed}");
    })
    .unwrap();
    // The editor's change event reaches the store, and the store's observers the views,
    // once the update that typed ends: the next frame already paints the new rule.
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rules_editor = workspace.read(cx).rules_panel().read(cx).editor().clone();
        let typed = rules_editor.read(cx).value();
        assert_eq!(window.find("rules-problems").label(), Some("No problems."));
        // Painted in both open files, and written to the file.
        let panels = workspace.read(cx).panels(cx);
        let rs_view = panels[0].read(cx).text_view().unwrap().clone();
        let log_view = panels[1].read(cx).text_view().unwrap().clone();
        assert_eq!(log_view.read(cx).rule_sets(), ["set 1", "logs"]);
        assert_eq!(rs_view.read(cx).rule_sets(), ["set 1"]);
        let spans = painted(&log_view, cx);
        assert_eq!(
            &spans[19..],
            [
                ("ERROR cannot open plc_".to_string(), "error".to_string()),
                ("7".to_string(), "number".to_string()),
                (".txt: ".to_string(), "error".to_string()),
                ("TODO".to_string(), "todo".to_string()),
                (" retry".to_string(), "error".to_string()),
            ]
        );
        let ranges: Vec<_> = log_view.read(cx).rule_spans().iter().map(|s| s.range.clone()).collect();
        assert_eq!(log_view.read(cx).decoration_ranges(cx), ranges);
        assert_eq!(std::fs::read_to_string(&rules_path).unwrap(), typed.as_ref());
        assert_eq!(workspace.read(cx).rules().read(cx).source(), typed.as_ref());

        // A broken pattern typed in is reported with its line; the other rules still paint.
        let focus = rules_editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        for line in ["[[set]]", "files=\"*\"", "[[set.rule]]", "pattern=\"(\"", ""] {
            window.input(line, cx);
            window.press("enter", cx);
        }
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let rules_editor = workspace.read(cx).rules_panel().read(cx).editor().clone();
        let log_view = workspace.read(cx).panels(cx)[1].read(cx).text_view().unwrap().clone();
        let problems = window.find("rules-problems").label().unwrap_or_default().to_string();
        // The cursor stayed after the first typed set, so the broken set is the second one
        // and its rule header is line 11.
        assert!(problems.starts_with("line 11: set `set 2`, rule `rule 1`: regex parse error"), "{problems}");
        assert_eq!(problems.lines().count(), 1, "one problem, on one line: {problems}");
        assert!(problems.ends_with("error: unclosed group"), "{problems}");
        assert_eq!(workspace.read(cx).rules().read(cx).file().problems.len(), 1);
        let spans = painted(&log_view, cx);
        assert_eq!(spans[22], ("TODO".to_string(), "todo".to_string()));
        assert_eq!(log_view.read(cx).rule_sets(), ["set 1", "set 2", "logs"]);
        assert_eq!(std::fs::read_to_string(&rules_path).unwrap(), rules_editor.read(cx).value().as_ref());

        // The chord closes the panel again.
        window.press("ctrl-shift-h", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!workspace.read(cx).rules_panel_open(cx));
    })
    .unwrap();
    let _ = std::fs::remove_file(&rules_path);
}

/// C-006: with no rule file, the panel starts from the commented example, which already
/// paints; nothing is written until the first change, which creates the directory and
/// the file.
#[gpui_kit::test]
fn a_missing_rule_file_starts_from_the_example_and_is_written_on_the_first_change(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!("sfoglia-rules-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rules_path = dir.join("sfoglia").join("highlight-rules.toml");
    let (handle, workspace) = open_workspace_with_rules(cx, rules_path.clone());
    let log = fixture("sample.log");

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[log.clone()], window, cx));
        window.render_frame(cx);
        assert!(!rules_path.exists(), "nothing is written before a change");
        let store = workspace.read(cx).rules().clone();
        assert_eq!(store.read(cx).source(), STARTER);
        let editor = workspace.read(cx).rules_panel().read(cx).editor().clone();
        assert_eq!(editor.read(cx).value().as_ref(), STARTER);
        // The example's log set paints the log: three timestamps, the warning and the error.
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).rule_sets(), ["logs"]);
        let names: Vec<String> = view
            .read(cx)
            .rule_spans()
            .iter()
            .map(|s| store.read(cx).file().rule(s).name.clone())
            .collect();
        assert_eq!(names, ["timestamp", "timestamp", "warning", "timestamp", "error"]);
        assert_eq!(window.find("text-rules").label(), Some("rules: logs"));
        window.dispatch_action(Box::new(ToggleHighlightRules), cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).rules_panel_open(cx));
        assert!(window.try_find("rules-write-error").is_none());
        assert!(!rules_path.exists());
        let editor = workspace.read(cx).rules_panel().read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        // One character at the start of the file: the example's first comment line gains a `#`.
        window.input("#", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let expected = format!("#{STARTER}");
        assert_eq!(workspace.read(cx).rules().read(cx).source(), expected);
        assert_eq!(std::fs::read_to_string(&rules_path).unwrap(), expected, "the first change creates the directory and the file");
        assert!(window.try_find("rules-write-error").is_none());
        assert_eq!(window.find("rules-problems").label(), Some("No problems."));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// C-006: a rule file that exists but cannot be read is shown as the example and never
/// written over; edits still re-highlight.
#[gpui_kit::test]
fn an_unreadable_rule_file_is_never_written_over(cx: &mut TestAppContext) {
    let rules_path = temp_rules_path();
    let bytes = [0xffu8, 0xfe, 0x00, 0x41, 0xff];
    std::fs::write(&rules_path, bytes).unwrap();
    let (handle, workspace) = open_workspace_with_rules(cx, rules_path.clone());
    let log = fixture("sample.log");

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[log.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).rules().read(cx).source(), STARTER);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).rule_spans().len(), 5, "the example paints meanwhile");
        window.dispatch_action(Box::new(ToggleHighlightRules), cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let error = window.find("rules-write-error").label().unwrap_or_default().to_string();
        assert!(error.starts_with("the rule file could not be read"), "{error}");
        assert!(error.ends_with("edits are not saved"), "{error}");
        let editor = workspace.read(cx).rules_panel().read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("#", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).rules().read(cx).source(), format!("#{STARTER}"), "the edit is applied in memory");
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).rule_spans().len(), 5);
        assert_eq!(std::fs::read(&rules_path).unwrap(), bytes, "the unreadable file is byte for byte as it was");
        assert!(window.find("rules-write-error").label().unwrap_or_default().starts_with("the rule file could not be read"));
    })
    .unwrap();
    let _ = std::fs::remove_file(&rules_path);
}

/// C-006: a set's globs see the file name alone, never its directory; `^` and `$` see
/// CRLF line ends.
#[gpui_kit::test]
fn globs_see_the_file_name_alone_and_anchors_see_crlf_lines(cx: &mut TestAppContext) {
    let rules_path = temp_rules_path();
    std::fs::write(
        &rules_path,
        "[[set]]\nname = \"plc\"\nfiles = [\"plc_*.txt\"]\n\n[[set.rule]]\nname = \"word\"\npattern = \"^[a-z]+$\"\nbold = true\n\n[[set]]\nname = \"logs\"\nfiles = \"*logs*\"\n\n[[set.rule]]\npattern = \"x\"\n",
    )
    .unwrap();
    // The directory's name matches `*logs*`; neither file's name does. The first file's
    // name matches `plc_*.txt`, which its path, taken from the start, would not.
    let dir = std::env::temp_dir().join(format!("sfoglia-logs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plc = dir.join("plc_1.txt");
    std::fs::write(&plc, "abc\r\ndef\r\nnot this one\r\n").unwrap();
    let other = dir.join("notes.txt");
    std::fs::write(&other, "x\n").unwrap();
    let (handle, workspace) = open_workspace_with_rules(cx, rules_path.clone());

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[plc.clone(), other.clone()], window, cx));
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        let plc_view = panels[0].read(cx).text_view().unwrap().clone();
        let other_view = panels[1].read(cx).text_view().unwrap().clone();
        assert_eq!(plc_view.read(cx).rule_sets(), ["plc"]);
        assert_eq!(other_view.read(cx).rule_sets(), Vec::<String>::new());
        assert!(other_view.read(cx).rule_spans().is_empty());
        assert_eq!(window.find("text-rules").label(), Some("no rules"));
        // The two short lines match between their CRLF ends; the carriage returns are outside.
        assert_eq!(plc_view.read(cx).status().line_endings, LineEndings::CrLf);
        assert_eq!(plc_view.read(cx).text(cx).as_ref(), "abc\r\ndef\r\nnot this one\r\n");
        let ranges: Vec<_> = plc_view.read(cx).rule_spans().iter().map(|s| s.range.clone()).collect();
        assert_eq!(ranges, [0..3, 5..8]);
        assert_eq!(plc_view.read(cx).decoration_ranges(cx), ranges);
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&rules_path);
}
