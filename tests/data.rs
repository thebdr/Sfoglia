//! Headless tests of the structured data viewer (C-010): a real gpui-kit window opens JSON,
//! JSON Lines, XML, YAML, TOML and INI files as trees, walks and searches them, copies paths,
//! shows and edits the raw text, pretty-prints it and saves it.
//!
//! Events such as a field's Enter or an editor's change land after the window update ends:
//! the tests leave `update_window`, run until parked, and assert in the next block.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::dock::Panel as _;
use gpui_kit::component::input::Position;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use sfoglia::data::Format;
use sfoglia::viewers::DataView;
use sfoglia::workspace::FilePanel;
use sfoglia::{Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-data-rules-{}-{n}.toml", std::process::id()));
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sfoglia-data-{}-{name}", std::process::id()))
}

fn label(window: &gpui_kit::Window, id: impl Into<gpui_kit::ElementId>) -> String {
    window.find(id).label().unwrap_or_default().to_string()
}

/// Open `path` in a fresh workspace and hand back its panel and data view.
fn open_data(cx: &mut TestAppContext, path: PathBuf) -> (AnyWindowHandle, Entity<FilePanel>, Entity<DataView>) {
    let (handle, workspace) = open_workspace(cx);
    let (panel, view) = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[path], window, cx));
            window.render_frame(cx);
            let panel = workspace.read(cx).panels(cx)[0].clone();
            let view = panel.read(cx).data_view().expect("the file opens in the data viewer").clone();
            (panel, view)
        })
        .unwrap();
    cx.run_until_parked();
    (handle, panel, view)
}

#[gpui_kit::test]
fn a_json_tree_opens_expands_collapses_navigates_searches_and_copies_paths(cx: &mut TestAppContext) {
    let (handle, panel, view) = open_data(cx, fixture("sample.json"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(panel.read(cx).text_view().is_none());
        assert_eq!(view.read(cx).format(), Format::Json);
        assert!(!view.read(cx).raw_shown());
        assert_eq!(view.read(cx).tree().unwrap().len(), 13);
        // The root is open, everything under it closed: six rows.
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, 2, 3, 6, 7]);
        assert_eq!(label(window, ("data-row", 0usize)), "0|$|object|5 entries");
        assert_eq!(label(window, ("data-row", 1usize)), "1|name|string|Sfoglia");
        assert_eq!(label(window, ("data-row", 2usize)), "1|version|number|1.5");
        assert_eq!(label(window, ("data-row", 3usize)), "1|tags|array|2 entries");
        assert_eq!(label(window, ("data-row", 4usize)), "1|ok|boolean|true");
        assert_eq!(label(window, ("data-row", 5usize)), "1|nested|object|1 entry");
        assert!(window.try_find(("data-row", 6usize)).is_none());
        assert_eq!(label(window, "data-summary"), "JSON, 13 nodes, 6 levels");
        assert_eq!(view.read(cx).selected_node(), Some(0));
        assert_eq!(label(window, "data-path"), "$");
        // The rows hold the focus from the start, so the keys work without a click.
        assert!(view.read(cx).focus_handle(cx).is_focused(window), "the rows hold the focus when the tab shows");
        window.press("down", cx);
        window.press("down", cx);
        window.press("down", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(3));
        assert_eq!(label(window, "data-path"), "$.tags");
        // Enter opens the array; Right moves into it; Left goes back up, then closes it; Space opens it again.
        window.press("enter", cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_expanded(3));
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(label(window, ("data-row", 4usize)), "2|0|string|rust");
        assert_eq!(label(window, ("data-row", 5usize)), "2|1|string|gpui");
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(4));
        assert_eq!(label(window, "data-path"), "$.tags[0]");
        window.press("left", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(3));
        window.press("left", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).is_expanded(3));
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, 2, 3, 6, 7]);
        window.press("space", cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_expanded(3));
        // Down to the nested object, Right opens it and walks in: the deep path builds up.
        for _ in 0..4 {
            window.press("down", cx);
        }
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(7));
        assert_eq!(label(window, "data-path"), "$.nested");
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        window.press("right", cx);
        window.press("right", cx);
        window.press("right", cx);
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(9));
        assert_eq!(label(window, "data-path"), "$.nested.a.b");
        window.press("down", cx);
        window.press("down", cx);
        window.press("right", cx);
        window.press("right", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(12));
        assert_eq!(label(window, "data-path"), "$.nested.a.b[1].c");
        assert_eq!(label(window, ("data-row", 12usize)), "5|c|string|deep");
        // Ctrl+Shift+C puts the path on the clipboard and says so.
        window.press("ctrl-shift-c", cx);
        window.render_frame(cx);
        assert_eq!(cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("$.nested.a.b[1].c"));
        assert_eq!(label(window, "data-message"), "copied $.nested.a.b[1].c");
        // A click on a container's row selects it and closes it (it was open); a click on its glyph opens it again.
        window.click(("data-row", 7usize), cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(7));
        assert!(!view.read(cx).is_expanded(7));
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, 2, 3, 4, 5, 6, 7]);
        window.click(("data-glyph", 7usize), cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_expanded(7));
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        // A click on a leaf's row only selects it.
        window.click(("data-row", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_node(), Some(1));
        assert_eq!(view.read(cx).visible_nodes().len(), 13);
        // Ctrl+Shift+F focuses the search field.
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        let search = view.read(cx).search_input().clone();
        assert!(search.read(cx).focus_handle(cx).is_focused(window), "the search field takes the focus");
        window.input("deep", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Enter found the match, opening its ancestors.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).matches(), &[12]);
        assert_eq!(view.read(cx).selected_node(), Some(12));
        assert!(view.read(cx).visible_nodes().contains(&12), "the match's ancestors are open");
        assert_eq!(label(window, "data-message"), "match 1 of 1");
        assert_eq!(label(window, "data-path"), "$.nested.a.b[1].c");
        // The matching row is marked, the others are not.
        assert_eq!(label(window, ("data-row", 12usize)), "5|c|string|deep|found");
        assert_eq!(label(window, ("data-row", 11usize)), "4|1|object|1 entry");
        // A search is case-insensitive over keys and values, and wraps around.
        window.press("ctrl-a", cx);
        window.input("E", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).matches(), &[0, 1, 2, 3, 6, 7, 8, 9, 11, 12]);
        assert_eq!(label(window, "data-message"), "match 1 of 10");
        assert_eq!(view.read(cx).selected_node(), Some(0));
        assert_eq!(label(window, ("data-row", 0usize)), "0|$|object|5 entries|found");
        assert_eq!(label(window, ("data-row", 4usize)), "2|0|string|rust");
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "match 2 of 10");
        assert_eq!(view.read(cx).selected_node(), Some(1));
        // The Next button finds the next one too.
        window.click("data-next", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "match 3 of 10");
        assert_eq!(view.read(cx).selected_node(), Some(2));
        window.press("ctrl-a", cx);
        window.input("nothing here", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).matches(), &[] as &[usize]);
        assert_eq!(label(window, "data-message"), "no node matches \"nothing here\"");
        assert_eq!(label(window, ("data-row", 0usize)), "0|$|object|5 entries", "no row is marked any more");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_raw_text_toggles_edits_pretty_prints_and_saves(cx: &mut TestAppContext) {
    let path = scratch("compact.json");
    let compact = "{\"b\":[1,{\"c\":null}],\"a\":\"x\"}";
    std::fs::write(&path, compact).unwrap();
    let pretty = "{\n  \"b\": [\n    1,\n    {\n      \"c\": null\n    }\n  ],\n  \"a\": \"x\"\n}\n";
    let (handle, panel, view) = open_data(cx, path.clone());
    let text = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).tree().unwrap().len(), 6);
            assert!(view.read(cx).text_view().is_none(), "the text viewer is built on first need");
            // Ctrl+Shift+D shows the raw text in the text viewer, which takes the focus.
            window.press("ctrl-shift-d", cx);
            window.render_frame(cx);
            assert!(view.read(cx).raw_shown());
            let text = view.read(cx).text_view().expect("the raw text is shown").clone();
            assert_eq!(text.read(cx).text(cx).as_ref(), compact);
            assert_eq!(text.read(cx).status().language, "json");
            let editor = text.read(cx).editor().clone();
            assert!(editor.read(cx).focus_handle(cx).is_focused(window), "the editor holds the focus");
            assert!(panel.read(cx).editable_text(cx).is_some());
            // Ctrl+Shift+P pretty-prints as one undoable edit: the tab is modified, the tree keeps its nodes.
            window.press("ctrl-shift-p", cx);
            window.render_frame(cx);
            assert_eq!(text.read(cx).text(cx).as_ref(), pretty);
            assert!(text.read(cx).is_dirty());
            assert!(panel.read(cx).is_dirty(cx));
            assert!(panel.read(cx).tab_name(cx).as_deref().unwrap().ends_with(" •"), "{:?}", panel.read(cx).tab_name(cx));
            assert_eq!(view.read(cx).tree().unwrap().len(), 6);
            assert_eq!(label(window, "data-message"), "pretty-printed, 6 nodes");
            window.press("ctrl-z", cx);
            window.render_frame(cx);
            assert_eq!(text.read(cx).text(cx).as_ref(), compact, "the pretty-print undoes");
            window.press("ctrl-y", cx);
            window.render_frame(cx);
            assert_eq!(text.read(cx).text(cx).as_ref(), pretty);
            // A second pretty-print changes nothing and says so.
            window.press("ctrl-shift-p", cx);
            window.render_frame(cx);
            assert_eq!(label(window, "data-message"), "already in its pretty form");
            // Ctrl+S writes the pretty text (C-008).
            window.press("ctrl-s", cx);
            window.render_frame(cx);
            assert!(!text.read(cx).is_dirty());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), pretty);
            // Back to the tree, rebuilt from the text; the rows take the focus again.
            window.press("ctrl-shift-d", cx);
            window.render_frame(cx);
            assert!(!view.read(cx).raw_shown());
            assert_eq!(view.read(cx).tree().unwrap().len(), 6);
            assert_eq!(label(window, ("data-row", 0usize)), "0|$|object|2 entries");
            assert!(view.read(cx).focus_handle(cx).is_focused(window));
            // An edit that breaks the file.
            window.press("ctrl-shift-d", cx);
            window.render_frame(cx);
            window.press("ctrl-a", cx);
            window.input("{\"a\": }", cx);
            text
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(text.read(cx).is_dirty(), "typing marks the raw text modified");
        // The tree side shows the error, the text keeps its edit.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        assert_eq!(view.read(cx).error().map(|e| (e.line, e.column)), Some((1, 7)));
        assert_eq!(label(window, "data-error"), "line 1, column 7: expected value");
        assert_eq!(label(window, "data-message"), "line 1, column 7: expected value");
        assert!(window.try_find(("data-row", 0usize)).is_none());
        assert!(panel.read(cx).is_dirty(cx));
        // Pretty-printing a broken text is refused with the same position.
        window.press("ctrl-shift-p", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "line 1, column 7: expected value");
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\": }");
        // Fixed again.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        window.press("ctrl-a", cx);
        window.input("{\"a\": 2}", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).error().is_none());
        assert_eq!(view.read(cx).tree().unwrap().len(), 2);
        assert_eq!(label(window, ("data-row", 1usize)), "1|a|number|2");
        assert!(panel.read(cx).is_dirty(cx));
        // Ctrl+S from the tree side saves the raw text too.
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert!(!panel.read(cx).is_dirty(cx));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\": 2}");
    })
    .unwrap();
    std::fs::remove_file(&path).ok();
}

#[gpui_kit::test]
fn a_malformed_file_opens_on_its_text_at_the_error_line(cx: &mut TestAppContext) {
    let (handle, panel, view) = open_data(cx, fixture("broken.json"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "it opens on its text");
        assert_eq!(view.read(cx).error().map(|e| (e.line, e.column, e.message.clone())), Some((4, 1, "expected value".to_string())));
        assert_eq!(label(window, "data-message"), "line 4, column 1: expected value");
        assert_eq!(label(window, "data-summary"), "JSON, does not parse");
        let text = view.read(cx).text_view().expect("the text viewer exists at once").clone();
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\n  \"a\": 1,\n  \"b\": [1, 2,\n}\n");
        let editor = text.read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(3, 0), "the cursor sits on the error's line");
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
        assert!(!panel.read(cx).is_dirty(cx));
        // The tree side shows the same error instead of rows.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        assert_eq!(label(window, "data-error"), "line 4, column 1: expected value");
        assert!(window.try_find(("data-row", 0usize)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn every_format_opens_as_a_tree_with_its_root_rows(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    cx.update_window(handle, |_, window, cx| {
        let files = ["records.jsonl", "catalogue.xml", "config.yaml", "config.toml", "settings.ini"];
        workspace.update(cx, |w, cx| w.open_paths(&files.map(fixture), window, cx));
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 5);
        // Each with the grammar its raw text gets in the text viewer (C-005): JSON Lines the JSON one, XML
        // the HTML one, INI none (its rules alone).
        let expected: [(Format, usize, &[usize], &[(usize, &str)], (usize, &str), &str); 5] = [
            (
                Format::JsonLines,
                9,
                &[0, 1, 4],
                &[(0, "0|$|array|2 entries"), (1, "1|0|object|2 entries"), (2, "1|1|object|3 entries")],
                (8, "$[1].tags[0]"),
                "json",
            ),
            (
                Format::Xml,
                8,
                &[0, 1, 2, 5],
                &[(0, "0|catalogue|element|3 entries"), (1, "1|@year|attribute|2026"), (2, "1|book|element|2 entries")],
                (7, "/catalogue/book[2]/title"),
                "html",
            ),
            (
                Format::Yaml,
                7,
                &[0, 1, 2, 5],
                &[(0, "0|$|object|3 entries"), (1, "1|name|string|Sfoglia"), (2, "1|tags|array|2 entries"), (3, "1|server|object|1 entry")],
                (6, "$.server.port"),
                "yaml",
            ),
            (
                Format::Toml,
                5,
                &[0, 1, 2],
                &[(0, "0|$|table|2 entries"), (1, "1|name|string|Sfoglia"), (2, "1|server|table|2 entries")],
                (4, "$.server.port"),
                "toml",
            ),
            (
                Format::Ini,
                4,
                &[0, 1, 2, 3],
                &[(0, "0|name|key|top"), (1, "0|server|section|2 entries"), (2, "1|host|key|localhost"), (3, "1|port|key|8080")],
                (3, "server.port"),
                "text",
            ),
        ];
        for (ix, (format, nodes, visible, rows, (node, path), language)) in expected.into_iter().enumerate() {
            // Opening a file again brings its tab to the front, so its rows are the ones rendered.
            workspace.update(cx, |w, cx| w.open_paths(&[fixture(files[ix])], window, cx));
            window.render_frame(cx);
            let view = panels[ix].read(cx).data_view().unwrap_or_else(|| panic!("{} opens in the data viewer", files[ix])).clone();
            assert_eq!(view.read(cx).format(), format, "{}", files[ix]);
            assert!(!view.read(cx).raw_shown(), "{}", files[ix]);
            let tree = view.read(cx).tree().unwrap_or_else(|| panic!("{} parses", files[ix])).clone();
            assert_eq!(tree.len(), nodes, "{}", files[ix]);
            assert_eq!(view.read(cx).visible_nodes(), visible, "{}", files[ix]);
            for (row, text) in rows {
                assert_eq!(label(window, ("data-row", *row)), *text, "{} row {row}", files[ix]);
            }
            assert_eq!(tree.path(node), path, "{}", files[ix]);
            // The raw text, and back. Opening an already open file brings its tab to the front without
            // focusing it (the dock focuses on a click), so the rows are focused here first.
            let focus = view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.press("ctrl-shift-d", cx);
            window.render_frame(cx);
            let text = view.read(cx).text_view().unwrap_or_else(|| panic!("{} shows its raw text", files[ix])).clone();
            assert_eq!(text.read(cx).status().language, language, "{}", files[ix]);
            window.press("ctrl-shift-d", cx);
            window.render_frame(cx);
            assert!(!view.read(cx).raw_shown(), "{}", files[ix]);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_50_mb_json_file_opens_searches_and_scrolls_to_the_match(cx: &mut TestAppContext) {
    // One fixed name, removed at the end: a leftover would fill the disk on the next run.
    let path = std::env::temp_dir().join("sfoglia-data-big.json");
    let (text, nodes, deep_path) = sfoglia::data::big_json(50 * 1024 * 1024);
    std::fs::write(&path, &text).unwrap();
    drop(text);
    let records = (nodes - 3) / 10;
    let last_name = format!("record-{}", records - 1);
    let (handle, _panel, view) = open_data(cx, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).tree().unwrap().len(), nodes);
        assert_eq!(label(window, "data-summary"), format!("JSON, {nodes} nodes, 5 levels"));
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1, nodes - 1]);
        assert_eq!(label(window, ("data-row", 1usize)), format!("1|records|array|{records} entries"));
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        window.input(&last_name, cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // count, note, score, ok, tags[2], tags[1], tags[0], tags, then the name
        let name_node = nodes - 1 - 8;
        assert_eq!(view.read(cx).matches(), &[name_node]);
        assert_eq!(view.read(cx).selected_node(), Some(name_node));
        assert_eq!(label(window, "data-path"), format!("$.records[{}].name", records - 1));
        assert_eq!(label(window, "data-message"), "match 1 of 1");
        let row = view.read(cx).visible_nodes().iter().position(|&n| n == name_node).expect("the match is a row");
        assert!(row > 1000, "the match sits deep in the list: row {row}");
        let top = view.read(cx).top_row();
        let height = f32::from(window.find("data-tree").bounds().size.height);
        let fitting = (height / 24.).floor() as usize;
        assert!(top <= row && row < top + fitting, "row {row} is scrolled into view: top {top}, {fitting} rows fit");
        assert_eq!(label(window, ("data-row", row)), format!("3|name|string|{last_name}|found"));
        // The tag the parser test names is four nodes further down.
        assert_eq!(view.read(cx).tree().unwrap().path(name_node + 4), deep_path);
    })
    .unwrap();
    std::fs::remove_file(&path).ok();
}

#[gpui_kit::test]
fn the_raw_text_takes_a_space_and_a_read_only_text_stays_as_it_is(cx: &mut TestAppContext) {
    // Space typed into the raw text is the editor's, not the tree's open-or-close key.
    let spaced = scratch("spaced.json");
    std::fs::write(&spaced, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, spaced.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).is_expanded(0));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        let text = view.read(cx).text_view().unwrap().clone();
        window.press("end", cx);
        window.press("space", cx);
        window.input("x", cx);
        window.render_frame(cx);
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\":1} x");
        assert!(view.read(cx).is_expanded(0), "the space did not reach the tree");
        assert_eq!(view.read(cx).visible_nodes(), &[0, 1]);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(panel.read(cx).is_dirty(cx));
        // The tab stays open while it is modified; a save brings the text to disk.
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert!(!panel.read(cx).is_dirty(cx));
        assert_eq!(std::fs::read_to_string(&spaced).unwrap(), "{\"a\":1} x");
    })
    .unwrap();
    std::fs::remove_file(&spaced).ok();

    // A UTF-16 file with a lone surrogate is read-only in the text viewer (C-008): the tree
    // still shows it, and a pretty-print leaves the text alone and says why.
    let readonly = scratch("readonly.json");
    let mut bytes = vec![0xff, 0xfe];
    for unit in "{\"a\":\"".encode_utf16().chain([0xd800u16]).chain("\"}".encode_utf16()) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&readonly, &bytes).unwrap();
    let (handle, panel, view) = open_data(cx, readonly.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).tree().unwrap().len(), 2);
        assert_eq!(label(window, ("data-row", 1usize)), "1|a|string|\u{FFFD}");
        window.press("ctrl-shift-p", cx);
        window.render_frame(cx);
        let text = view.read(cx).text_view().expect("the text viewer exists after the attempt").clone();
        assert!(!text.read(cx).is_editable());
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\":\"\u{FFFD}\"}");
        assert!(!text.read(cx).is_dirty());
        assert!(!panel.read(cx).is_dirty(cx));
        assert_eq!(label(window, "data-message"), "the text is read-only (it has units that did not decode), so it stays as it is");
        assert_eq!(view.read(cx).tree().unwrap().len(), 2);
        // The same from the raw side.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        window.press("ctrl-shift-p", cx);
        window.render_frame(cx);
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\":\"\u{FFFD}\"}");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&readonly).ok();

    // A CRLF file already in its pretty form says so and stays unmodified: the editor holds the file's
    // returns, the pretty form is written with line feeds, and the two are compared with the same endings.
    let crlf = scratch("crlf.json");
    std::fs::write(&crlf, "{\r\n  \"a\": 1\r\n}\r\n").unwrap();
    let (handle, panel, view) = open_data(cx, crlf.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).tree().unwrap().len(), 2);
        window.press("ctrl-shift-p", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "already in its pretty form");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&crlf).ok();

    // A change another program makes between opening the tab and first showing the raw text is noticed:
    // the text viewer takes the disk as it was when the bytes were read (C-008), so its check finds the
    // newer file and offers Reload, which brings the other program's content and rebuilds the tree.
    let shared = scratch("shared.json");
    std::fs::write(&shared, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, shared.clone());
    std::fs::write(&shared, "{\"a\":1,\"other\":\"program\"}").unwrap();
    let text = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press("ctrl-shift-d", cx);
            window.render_frame(cx);
            view.read(cx).text_view().expect("the raw text is shown").clone()
        })
        .unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(text.read(cx).changed_on_disk(), "the newer file is noticed");
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\":1}");
        text.update(cx, |text, cx| text.reload(window, cx));
        window.render_frame(cx);
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\":1,\"other\":\"program\"}");
        assert!(!text.read(cx).changed_on_disk());
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).tree().unwrap().len(), 3);
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&shared).ok();
}

#[gpui_kit::test]
fn the_tree_side_shows_the_text_viewers_messages_and_the_disk_change_bar(cx: &mut TestAppContext) {
    // A save refused by the encoding (C-008) is reported on the tree side too: the tree's status row
    // shows the text viewer's message.
    let latin = scratch("latin.json");
    std::fs::write(&latin, b"{\"a\":\"caf\xe9\"}").unwrap();
    let (handle, panel, _view) = open_data(cx, latin.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        window.press("end", cx);
        window.press("left", cx);
        window.press("left", cx);
        window.input("€", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "Latin-1 cannot hold '€' (U+20AC) on line 1; choose another encoding");
        assert!(panel.read(cx).is_dirty(cx));
        assert_eq!(std::fs::read(&latin).unwrap(), b"{\"a\":\"caf\xe9\"}");
        // A second refusal, after another status, shows again.
        window.press("ctrl-shift-c", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "copied $");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "Latin-1 cannot hold '€' (U+20AC) on line 1; choose another encoding");
    })
    .unwrap();
    std::fs::remove_file(&latin).ok();

    // A change on disk while the tree side shows raises the Reload / Keep mine bar there; Keep mine lets
    // the next save overwrite, as C-008 states.
    let edited = scratch("edited.json");
    std::fs::write(&edited, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, edited.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        window.press("end", cx);
        window.press("space", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(window.try_find("data-changed-on-disk").is_none());
    })
    .unwrap();
    std::fs::write(&edited, "{\"a\":1,\"other\":\"program\"}").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).changed_on_disk(cx));
        assert_eq!(label(window, "data-changed-on-disk"), "The file changed on disk.");
        window.click("data-keep-mine", cx);
        window.render_frame(cx);
        assert!(window.try_find("data-changed-on-disk").is_none());
        assert_eq!(label(window, "data-message"), "kept yours; the next save overwrites the file");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), "{\"a\":1} ");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&edited).ok();

    // Before the text viewer exists the view watches the disk itself: Reload brings the newer file into
    // the tree.
    let watched = scratch("watched.json");
    std::fs::write(&watched, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, watched.clone());
    std::fs::write(&watched, "{\"a\":1,\"other\":\"program\"}").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).text_view().is_none());
        assert!(view.read(cx).changed_on_disk(cx));
        assert_eq!(label(window, "data-changed-on-disk"), "The file changed on disk.");
        assert_eq!(view.read(cx).tree().unwrap().len(), 2, "still the tree as read");
        window.click("data-reload", cx);
        window.render_frame(cx);
        assert!(window.try_find("data-changed-on-disk").is_none());
        assert_eq!(view.read(cx).tree().unwrap().len(), 3);
        assert_eq!(label(window, "data-message"), "reloaded");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&watched).ok();

    // Keep mine before the text viewer exists counts as modified, and the next save writes the kept text (C-008).
    let kept = scratch("kept.json");
    std::fs::write(&kept, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, kept.clone());
    std::fs::write(&kept, "{\"a\":1,\"other\":\"program\"}").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "data-changed-on-disk"), "The file changed on disk.");
        window.click("data-keep-mine", cx);
        window.render_frame(cx);
        assert!(window.try_find("data-changed-on-disk").is_none());
        assert!(panel.read(cx).is_dirty(cx), "kept text counts as modified");
        assert_eq!(view.read(cx).tree().unwrap().len(), 2);
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "{\"a\":1}");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&kept).ok();

    // A Reload that cannot read the file (deleted meanwhile) keeps the text, marks the tab modified and
    // says why; the next save recreates the file (C-008), from the tree side.
    let gone = scratch("gone.json");
    std::fs::write(&gone, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, gone.clone());
    std::fs::remove_file(&gone).unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "data-changed-on-disk"), "The file changed on disk.");
        window.click("data-reload", cx);
        window.render_frame(cx);
        assert!(label(window, "data-message").starts_with("cannot read "), "{}", label(window, "data-message"));
        assert!(window.try_find("data-changed-on-disk").is_none());
        assert!(panel.read(cx).is_dirty(cx), "the only copy counts as modified");
        assert_eq!(view.read(cx).tree().unwrap().len(), 2);
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&gone).unwrap(), "{\"a\":1}");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&gone).ok();

    // A Reload that brings in a file that no longer parses gets what such a file gets when it opens: the
    // error in the status row, the raw text with the cursor on the error, and the tree side showing the
    // error instead of rows.
    let spoiled = scratch("spoiled.json");
    std::fs::write(&spoiled, "{\"a\":1}").unwrap();
    let (handle, panel, view) = open_data(cx, spoiled.clone());
    std::fs::write(&spoiled, "{\"a\":1,\n\"b\": }").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "data-changed-on-disk"), "The file changed on disk.");
        window.click("data-reload", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "data-message"), "line 2, column 6: expected value");
        assert_eq!(label(window, "data-summary"), "JSON, does not parse");
        assert!(view.read(cx).raw_shown(), "the raw text shows");
        assert!(window.try_find("data-changed-on-disk").is_none());
        let text = view.read(cx).text_view().expect("the text viewer exists").clone();
        assert_eq!(text.read(cx).text(cx).as_ref(), "{\"a\":1,\n\"b\": }");
        let editor = text.read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(1, 5), "the cursor sits on the error");
        assert!(editor.read(cx).focus_handle(cx).is_focused(window));
        assert!(!panel.read(cx).is_dirty(cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        assert_eq!(label(window, "data-error"), "line 2, column 6: expected value");
        assert!(window.try_find(("data-row", 0usize)).is_none());
    })
    .unwrap();
    std::fs::remove_file(&spoiled).ok();
}
