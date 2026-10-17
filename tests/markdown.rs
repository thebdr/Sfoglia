//! Headless tests of the Markdown viewer (C-011): a real gpui-kit window opens a Markdown file
//! rendered, toggles to the raw text and back keeping the place, edits and saves it, and shows the
//! text viewer's messages and the Reload / Keep mine bar on the rendered side.
//!
//! The kit parses Markdown on a background task and an editor's change lands after the window
//! update ends: the tests leave `update_window`, run until parked, and assert in the next block.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::input::Position;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, ListOffset, TestAppContext, px};
use sfoglia::viewers::MarkdownView;
use sfoglia::workspace::FilePanel;
use sfoglia::{Family, Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-markdown-rules-{}-{n}.toml", std::process::id()));
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
    std::env::temp_dir().join(format!("sfoglia-markdown-{}-{name}", std::process::id()))
}

fn label(window: &gpui_kit::Window, id: impl Into<gpui_kit::ElementId>) -> String {
    window.find(id).label().unwrap_or_default().to_string()
}

/// Open `path` in a fresh workspace, let the parse land, and hand back its panel and view.
fn open_markdown(cx: &mut TestAppContext, path: PathBuf) -> (AnyWindowHandle, Entity<FilePanel>, Entity<MarkdownView>) {
    let (handle, workspace) = open_workspace(cx);
    let (panel, view) = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[path], window, cx));
            window.render_frame(cx);
            let panel = workspace.read(cx).panels(cx)[0].clone();
            let view = panel.read(cx).markdown_view().expect("the file opens in the Markdown viewer").clone();
            (panel, view)
        })
        .unwrap();
    cx.run_until_parked();
    (handle, panel, view)
}

/// The rendered side's plain text, as the kit's rendered text reads once the parse has landed.
fn rendered(view: &Entity<MarkdownView>, cx: &gpui_kit::App) -> String {
    view.read(cx).rich_text().read(cx).rendered_text().as_str().to_string()
}

#[gpui_kit::test]
fn the_markdown_extensions_map_to_their_family(_: &mut TestAppContext) {
    let markdown: Vec<&str> = sfoglia::generated::families::EXTENSIONS
        .iter()
        .filter(|(_, families)| families.contains(&Family::Markdown))
        .map(|(ext, _)| *ext)
        .collect();
    assert_eq!(markdown, vec!["markdown", "md"]);
    for (ext, families) in sfoglia::generated::families::EXTENSIONS {
        if families.contains(&Family::Markdown) {
            assert_eq!(*families, &[Family::Markdown], "{ext} belongs to Markdown alone");
        }
    }
    assert_eq!(sfoglia::text::language_for("notes.md"), Some("markdown"));
    assert_eq!(sfoglia::text::language_for("notes.markdown"), Some("markdown"));
}

#[gpui_kit::test]
fn a_markdown_file_opens_rendered_with_its_summary_and_text(cx: &mut TestAppContext) {
    let (handle, panel, view) = open_markdown(cx, fixture("guide.md"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown(), "it opens rendered");
        assert!(window.try_find("markdown-rendered").is_some());
        assert_eq!(label(window, "markdown-summary"), "Markdown, 20 blocks, 6 headings");
        // Both images, the inline one and the one written with a reference, were asked for from the file's folder.
        let dot = fixture("dot.png");
        assert_eq!(view.read(cx).requested_images().iter().filter(|p| **p == dot).count(), 2, "{:?}", view.read(cx).requested_images());
        assert_eq!(label(window, "markdown-message"), "Ctrl+Shift+D shows the text, a click follows a link");
        assert!(!panel.read(cx).is_dirty(cx));
        // Every heading, item, cell and code line, in order, is in the rendered text.
        let text = rendered(&view, cx);
        let expected = [
            "Sfoglia guide",
            "Sfoglia opens anything. This guide shows the rendered Markdown with emphasis, strong, strike and code.",
            "Install",
            "Install Rust",
            "Clone the repository",
            "with git",
            "or as a zip",
            "Run cargo run",
            "Features",
            "text and hex",
            "office documents",
            "Nothing is true until reality shows it.",
            "Formats",
            "Format",
            "Read",
            "Write",
            "text",
            "yes",
            "yes",
            "hex",
            "yes",
            "no",
            "images",
            "yes",
            "save-as",
            "Code",
            "fn main() {",
            "    println!(\"hello\");",
            "}",
            "plain text",
            "Links and images",
            "The site, the readme, the install section, a note",
            "and the dot.",
            "A footnote.",
        ];
        let mut rest: &str = &text;
        for part in expected {
            match rest.find(part) {
                Some(at) => rest = &rest[at + part.len()..],
                None => panic!("{part:?} not found in order in the rendered text:\n{text}"),
            }
        }
        assert!(view.read(cx).focus_handle(cx).is_focused(window), "the rendered side has the focus");
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_raw_text_toggles_edits_saves_and_re_renders(cx: &mut TestAppContext) {
    let notes = scratch("notes.md");
    std::fs::write(&notes, "# Title\n\nSome text.\n").unwrap();
    let (handle, panel, view) = open_markdown(cx, notes.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 2 blocks, 1 heading");
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "the raw text shows");
        assert!(window.try_find("markdown-rendered").is_none());
        assert_eq!(label(window, "markdown-message"), "Ctrl+Shift+D shows the rendered view");
        let text = view.read(cx).text_view().clone();
        assert_eq!(text.read(cx).text(cx).as_ref(), "# Title\n\nSome text.\n");
        let editor = text.read(cx).editor().clone();
        assert!(editor.read(cx).focus_handle(cx).is_focused(window), "the editor has the focus");
        window.press("ctrl-end", cx);
        window.input("## More", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(panel.read(cx).is_dirty(cx), "the edit marks the tab modified");
        assert_eq!(view.read(cx).text_view().read(cx).text(cx).as_ref(), "# Title\n\nSome text.\n## More");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "# Title\n\nSome text.\n## More");
        assert!(!panel.read(cx).is_dirty(cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 3 blocks, 2 headings");
        assert!(rendered(&view, cx).contains("More"), "{}", rendered(&view, cx));
        assert_eq!(view.read(cx).outline().headings[1].text, "More");
        assert!(view.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    std::fs::remove_file(&notes).ok();
}

#[gpui_kit::test]
fn the_toggle_carries_the_place_between_the_views(cx: &mut TestAppContext) {
    // Sixty sections of a heading and a paragraph: 120 blocks, four lines each section.
    let long = scratch("long.md");
    let mut text = String::new();
    for n in 0..60 {
        text.push_str(&format!("## Section {n}\n\nparagraph {n}\n\n"));
    }
    std::fs::write(&long, &text).unwrap();
    let (handle, _panel, view) = open_markdown(cx, long.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 120 blocks, 60 headings");
        // Scroll the rendered view so block 40 (the heading of section 20, line 81) is at the top.
        view.read(cx).rich_text().read(cx).list_state().scroll_to(ListOffset { item_ix: 40, offset_in_item: px(0.) });
        window.render_frame(cx);
        assert_eq!(view.read(cx).rich_text().read(cx).list_state().logical_scroll_top().item_ix, 40);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(80, 0), "the cursor sits on the block that was at the top");
        // Put the cursor on "paragraph 45" (section 45, line 183) and go back: the rendered view
        // scrolls to that block (index 91).
        editor.update(cx, |editor, cx| editor.set_cursor_position(Position::new(182, 0), window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        // Before the preparation lands, the summary says so and a toggle back to the raw text leaves the
        // cursor where it was; the next toggle prepares again and still reveals the cursor's block.
        assert_eq!(label(window, "markdown-summary"), "Markdown, preparing");
        assert!(view.read(cx).is_preparing());
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        assert_eq!(editor.read(cx).cursor_position(), Position::new(182, 0), "the place is kept while the rendered side is still preparing");
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let top = view.read(cx).rich_text().read(cx).list_state().logical_scroll_top().item_ix;
        // The block lands at the bottom edge of the view (row 76 at the top here); a tolerant range
        // covers the other font metrics of the Linux runner.
        assert!((60..=91).contains(&top), "block 91 is on screen: the top row is {top}");
    })
    .unwrap();
    std::fs::remove_file(&long).ok();
}

#[gpui_kit::test]
fn the_rendered_side_shows_the_text_viewers_messages_and_the_disk_change_bar(cx: &mut TestAppContext) {
    // A save refused by the encoding (C-008) is reported on the rendered side.
    let latin = scratch("latin.md");
    std::fs::write(&latin, b"# Caf\xe9\n").unwrap();
    let (handle, panel, view) = open_markdown(cx, latin.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        window.press("ctrl-end", cx);
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
        assert_eq!(label(window, "markdown-message"), "Latin-1 cannot hold '€' (U+20AC) on line 2; choose another encoding");
        assert!(panel.read(cx).is_dirty(cx));
        assert_eq!(std::fs::read(&latin).unwrap(), b"# Caf\xe9\n");
        assert!(view.read(cx).rich_text().read(cx).rendered_text().as_str().contains("Café"));
    })
    .unwrap();
    std::fs::remove_file(&latin).ok();

    // A change on disk raises the bar on the rendered side; Reload re-renders the newer file.
    let watched = scratch("watched.md");
    std::fs::write(&watched, "# One\n").unwrap();
    let (handle, panel, view) = open_markdown(cx, watched.clone());
    std::fs::write(&watched, "# One\n\n## Two\n").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).changed_on_disk(cx));
        assert_eq!(label(window, "markdown-changed-on-disk"), "The file changed on disk.");
        assert_eq!(label(window, "markdown-summary"), "Markdown, 1 block, 1 heading", "still the file as read");
        window.click("markdown-reload", cx);
        window.render_frame(cx);
        assert!(window.try_find("markdown-changed-on-disk").is_none());
        assert_eq!(label(window, "markdown-message"), "reloaded");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 2 blocks, 2 headings");
        assert!(rendered(&view, cx).contains("Two"), "{}", rendered(&view, cx));
    })
    .unwrap();
    std::fs::remove_file(&watched).ok();

    // Keep mine marks the tab modified and the next save overwrites the file (C-008).
    let kept = scratch("kept.md");
    std::fs::write(&kept, "# Mine\n").unwrap();
    let (handle, panel, _view) = open_markdown(cx, kept.clone());
    std::fs::write(&kept, "# Theirs\n").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-changed-on-disk"), "The file changed on disk.");
        window.click("markdown-keep-mine", cx);
        window.render_frame(cx);
        assert!(window.try_find("markdown-changed-on-disk").is_none());
        assert_eq!(label(window, "markdown-message"), "kept yours; the next save overwrites the file");
        assert!(panel.read(cx).is_dirty(cx), "kept text counts as modified");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&kept).unwrap(), "# Mine\n");
        assert!(!panel.read(cx).is_dirty(cx));
    })
    .unwrap();
    std::fs::remove_file(&kept).ok();
}

#[gpui_kit::test]
fn a_link_to_another_file_opens_it_at_its_heading(cx: &mut TestAppContext) {
    // A link to a file that is already open, under another spelling of its path, brings its tab up
    // scrolled to the heading its fragment names; a bare fragment scrolls the document itself; a
    // fragment into a tab showing its raw text puts the cursor on the heading; the app's own HTTP
    // client stands behind remote images.
    let folder = scratch("linked");
    std::fs::create_dir_all(&folder).unwrap();
    let a = folder.join("a.md");
    let b = folder.join("b.md");
    let mut text = String::new();
    for n in 0..60 {
        text.push_str(&format!("## Section {n}\n\nparagraph {n}\n\n"));
    }
    std::fs::write(&a, "# A\n\nSee [b](b.md#section-50) and [the top](#a).\n\n## Second\n\nmore\n").unwrap();
    std::fs::write(&b, &text).unwrap();
    let (handle, workspace) = open_workspace(cx);
    let agent = cx.update(|cx| cx.http_client().user_agent().map(|agent| agent.to_str().unwrap_or_default().to_string()));
    assert_eq!(agent.as_deref(), Some("Sfoglia"), "the app's HTTP client is installed, so remote images are fetched");
    let view = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[folder.join(".").join("b.md"), a.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)[1].read(cx).markdown_view().expect("rendered").clone()
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        view.update(cx, |view, cx| view.follow_link("#second", window, cx));
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Second");
        view.update(cx, |view, cx| view.follow_link("#nowhere", window, cx));
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "no heading named #nowhere");
        view.update(cx, |view, cx| view.follow_link("b.md#section-50", window, cx));
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), format!("opening {}", b.display()));
    })
    .unwrap();
    cx.run_until_parked();
    let other = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let panels = workspace.read(cx).panels(cx);
            assert_eq!(panels.len(), 2, "the linked file was already open: no third tab");
            panels[0].read(cx).markdown_view().expect("the linked file is rendered").clone()
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(other.read(cx).path(), folder.join(".").join("b.md").as_path(), "the tab opened first, under its own spelling");
        assert_eq!(label(window, "markdown-summary"), "Markdown, 120 blocks, 60 headings");
        assert_eq!(label(window, "markdown-message"), "at Section 50");
        let top = other.read(cx).rich_text().read(cx).list_state().logical_scroll_top().item_ix;
        assert!((70..=100).contains(&top), "section 50 (block 100) is on screen: the top row is {top}");
        // With the raw text showing, a fragment puts the cursor on the heading's line (section 20: line 81).
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(other.read(cx).raw_shown());
        other.update(cx, |view, cx| view.reveal_heading("section-20", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Section 20");
        let editor = other.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(80, 0));
        // Two lines typed above it move the heading: the fragment follows the text as edited (line 83).
        let edited = format!("intro\n\n{}", std::fs::read_to_string(&b).unwrap());
        editor.update(cx, |editor, cx| editor.set_value(edited, window, cx));
        other.update(cx, |view, cx| view.reveal_heading("section-20", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Section 20");
        let editor = other.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(82, 0));
    })
    .unwrap();
    // A heading search started on the raw side and overtaken by a toggle to the rendered side places nothing
    // and claims nothing: the hint stays.
    cx.update_window(handle, |_, window, cx| {
        other.update(cx, |view, cx| view.reveal_heading("section-50", window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!other.read(cx).raw_shown());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "Ctrl+Shift+D shows the text, a click follows a link");
    })
    .unwrap();
    std::fs::remove_dir_all(&folder).ok();
}

#[gpui_kit::test]
fn a_large_markdown_file_opens_on_its_text(cx: &mut TestAppContext) {
    // Past 2 MB the outline and the renderer's parse would take seconds on the window thread: the file
    // opens on its raw text with the status row saying so, and Ctrl+Shift+D keeps it there.
    let big = scratch("big.md");
    let mut text = String::from("# Big\n\n## Target\n\n");
    let mut n = 0;
    while text.len() < 2_600_000 {
        text.push_str(&format!("A paragraph of text in the big file, number {n}, long enough.\n\n"));
        n += 1;
    }
    std::fs::write(&big, &text).unwrap();
    let (handle, panel, view) = open_markdown(cx, big.clone());
    let message = format!("{:.1} MB of Markdown opens as text: the rendered view is kept for files under 2 MB", text.len() as f64 / (1024.0 * 1024.0));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "the raw text shows");
        assert!(window.try_find("markdown-rendered").is_none());
        assert_eq!(label(window, "markdown-summary"), format!("Markdown, {:.1} MB", text.len() as f64 / (1024.0 * 1024.0)));
        assert_eq!(label(window, "markdown-message"), message);
        assert_eq!(view.read(cx).text_view().read(cx).text(cx).len(), text.len(), "the whole text is there");
        assert!(!panel.read(cx).is_dirty(cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "the toggle keeps the text");
        assert_eq!(label(window, "markdown-message"), message);
        assert_eq!(view.read(cx).outline().blocks.len(), 0, "no outline is built for it");
        // A fragment into the raw text past the budget still finds its heading, by a scan of the lines.
        view.update(cx, |view, cx| view.reveal_heading("target", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Target");
        assert_eq!(view.read(cx).text_view().read(cx).editor().read(cx).cursor_position(), Position::new(2, 0));
    })
    .unwrap();
    // Shrunk on disk and reloaded, the text fits again and the next toggle renders it; grown past the budget
    // in the editor, the toggle keeps the raw text and says so, with the new size.
    std::fs::write(&big, "# Small\n\nagain\n").unwrap();
    cx.background_executor.advance_clock(std::time::Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("text-reload", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let grown = format!("# Grown\n\n{}\n", "x".repeat(2_200_000));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "a reload leaves the side as it was");
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown(), "the text fits now");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 2 blocks, 1 heading");
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        editor.update(cx, |editor, cx| editor.set_value(grown.clone(), window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown(), "the grown text stays raw");
        let message = format!("{:.1} MB of Markdown opens as text: the rendered view is kept for files under 2 MB", grown.len() as f64 / (1024.0 * 1024.0));
        assert_eq!(label(window, "markdown-message"), message);
        assert_eq!(label(window, "markdown-summary"), format!("Markdown, {:.1} MB", grown.len() as f64 / (1024.0 * 1024.0)));
        // Cut below the budget in the editor, the next toggle renders it and the hint is back.
        editor.update(cx, |editor, cx| editor.set_value("# Small\n\nagain\n".to_string(), window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown(), "the cut text renders");
        assert_eq!(label(window, "markdown-message"), "Ctrl+Shift+D shows the text, a click follows a link");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 2 blocks, 1 heading");
    })
    .unwrap();
    std::fs::remove_file(&big).ok();
}

#[gpui_kit::test]
fn the_place_is_kept_while_the_renderer_catches_up(cx: &mut TestAppContext) {
    // A text past the renderer's synchronous limit parses on its own task after the outline lands: a toggle
    // to the raw text in that window leaves the cursor where it was, and a fragment clicked while the
    // rendered side is still preparing has its way over the toggle's own scroll once the outline is there.
    let long = scratch("catch-up.md");
    let mut text = String::new();
    for n in 0..200 {
        text.push_str(&format!("## Section {n}\n\nparagraph {n} [to five](#section-5)\n\n"));
    }
    std::fs::write(&long, &text).unwrap();
    let (handle, _panel, view) = open_markdown(cx, long.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 400 blocks, 200 headings");
        view.read(cx).rich_text().read(cx).list_state().scroll_to(ListOffset { item_ix: 40, offset_in_item: px(0.) });
        window.render_frame(cx);
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(80, 0), "section 20, the block at the top");
        // Eight lines above everything move every block down; the cursor goes to line 191, inside section 45
        // of the new text.
        let edited = format!("one\n\ntwo\n\nthree\n\nfour\n\n{text}");
        editor.update(cx, |editor, cx| editor.set_value(edited, window, cx));
        editor.update(cx, |editor, cx| editor.set_cursor_position(Position::new(190, 0), window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
        assert!(view.read(cx).is_preparing());
    })
    .unwrap();
    // Step until the outline has landed but before the renderer's own parse of the new text has.
    while cx.update(|cx| view.read(cx).is_preparing()) {
        cx.background_executor.tick();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).rich_text().read(cx).rendered_text().source(), text, "the renderer still shows the old text");
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(190, 0), "the cursor stays while the renderer still shows the old text");
        // Back to the rendered side, and a fragment clicked while it prepares wins over the toggle's scroll.
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_preparing());
        view.update(cx, |view, cx| view.follow_link("#section-5", window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Section 5");
        let top = view.read(cx).rich_text().read(cx).list_state().logical_scroll_top().item_ix;
        assert!(top <= 14, "section 5 (block 14 after the four new paragraphs) is on screen, not the cursor's block: the top row is {top}");
    })
    .unwrap();
    // A fragment clicked while the rendered side prepares, then a toggle to the raw text before the outline
    // lands: the fragment is followed on the raw side, where the cursor goes to its heading (section 50, line 209
    // of the edited text).
    cx.update_window(handle, |_, window, cx| {
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        editor.update(cx, |editor, cx| editor.set_cursor_position(Position::new(10, 0), window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_preparing());
        view.update(cx, |view, cx| view.follow_link("#section-50", window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Section 50");
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(208, 0));
    })
    .unwrap();
    // The same one window later: the outline has landed but the renderer still parses the new text when the
    // fragment is clicked; a toggle to the raw text then follows it there (section 50, line 211 after ten
    // new lines).
    let edited = format!("zero\n\none\n\ntwo\n\nthree\n\nfour\n\n{text}");
    cx.update_window(handle, |_, window, cx| {
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        editor.update(cx, |editor, cx| editor.set_value(edited.clone(), window, cx));
        editor.update(cx, |editor, cx| editor.set_cursor_position(Position::new(10, 0), window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(!view.read(cx).raw_shown());
    })
    .unwrap();
    while cx.update(|cx| view.read(cx).is_preparing()) {
        cx.background_executor.tick();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_ne!(view.read(cx).rich_text().read(cx).rendered_text().source(), edited, "the renderer still parses the new text");
        view.update(cx, |view, cx| view.follow_link("#section-50", window, cx));
        window.press("ctrl-shift-d", cx);
        window.render_frame(cx);
        assert!(view.read(cx).raw_shown());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Section 50");
        let editor = view.read(cx).text_view().read(cx).editor().clone();
        assert_eq!(editor.read(cx).cursor_position(), Position::new(210, 0));
    })
    .unwrap();
    std::fs::remove_file(&long).ok();
}

#[gpui_kit::test]
fn a_reference_link_opens_like_an_inline_one(cx: &mut TestAppContext) {
    // The renderer resolves no link definition, so a click on `[text][id]` would hand over an empty address:
    // the preparation writes reference links inline, and a real click on one reaches its heading.
    let referenced = scratch("referenced.md");
    let mut text = String::from("[go to the target][r]\n\n[r]: #target\n\n");
    for n in 0..60 {
        text.push_str(&format!("## Section {n}\n\nparagraph {n}\n\n"));
    }
    text.push_str("# Target\n\nhere\n");
    std::fs::write(&referenced, &text).unwrap();
    let (handle, _panel, view) = open_markdown(cx, referenced.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-summary"), "Markdown, 124 blocks, 61 headings");
        assert!(view.read(cx).rich_text().read(cx).rendered_text().as_str().starts_with("go to the target"), "the link text renders");
        // The link is the first line of the rendered side: a click on its first word.
        window.click_at("markdown-rendered", gpui_kit::point(px(40.), px(16.)), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        assert_eq!(label(window, "markdown-message"), "at Target");
        let top = view.read(cx).rich_text().read(cx).list_state().logical_scroll_top().item_ix;
        assert!(top >= 100, "the target, block 122, is on screen: the top row is {top}");
    })
    .unwrap();
    std::fs::remove_file(&referenced).ok();
}
