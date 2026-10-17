//! Headless tests of editing and saving (C-008): a real gpui-kit window edits a text
//! file, marks the tab, writes the file back in its own encoding and endings, notices a
//! change made elsewhere, and keeps a modified tab open.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::component::Placement;
use gpui_kit::component::dock::{BasePanel as _, ClosePanel, DockPlacement, InsertTarget, NodeId, PaneNode, PaneRef, Panel as _, PanelId};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use sfoglia::workspace::FilePanel;
use sfoglia::{Workspace, window_options};

/// A workspace with a rule file path of this test alone (the file need not exist).
fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-edit-rules-{}-{n}.toml", std::process::id()));
    cx.update(gpui_kit::init);
    cx.update(sfoglia::init);
    cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |window, cx| {
            cx.new(|cx| Workspace::with_rules_path(Some(rules_path.clone()), window, cx))
        })
        .expect("open the main window headlessly")
    })
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)).unwrap()
}

/// A fresh directory for this test's files, and a copy of a fixture in it: the tests write.
fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sfoglia-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_fixture(name: &str, dir: &std::path::Path) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, fixture_bytes(name)).unwrap();
    path
}

/// Typing marks the tab, Ctrl+S writes the exact bytes in the file's encoding and endings,
/// an untouched file is never rewritten, a character the encoding cannot hold refuses the
/// save, and the status row changes what the next save writes.
#[gpui_kit::test]
fn typing_marks_the_tab_dirty_and_ctrl_s_writes_the_exact_bytes(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-save");
    let latin = copy_fixture("latin1.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[latin.clone()], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let view = panel.read(cx).text_view().unwrap().clone();
        assert!(view.read(cx).is_editable());
        assert!(!view.read(cx).is_dirty());
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("latin1.txt"));
        assert_eq!(window.find("text-dirty").label(), Some("saved"));
        assert!(window.try_find("text-revert").is_none());
        let editor = view.read(cx).editor().clone();
        assert!(editor.read(cx).is_editable(), "the viewer's editor accepts edits");
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("X", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let view = panel.read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), "Xcaffè latte à Roma\n");
        assert!(view.read(cx).is_dirty());
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("latin1.txt •"));
        assert_eq!(window.find("text-dirty").label(), Some("modified"));
        assert!(window.try_find("text-revert").is_some());
        assert_eq!(std::fs::read(&latin).unwrap(), fixture_bytes("latin1.txt"), "nothing is written before Ctrl+S");
        // Ctrl+S writes Latin-1 bytes, as the file was read.
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read(&latin).unwrap(), b"Xcaff\xe8 latte \xe0 Roma\n");
        assert!(!view.read(cx).is_dirty());
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("latin1.txt"));
        assert_eq!(window.find("text-dirty").label(), Some("saved"));
        assert_eq!(window.find("text-message").label(), Some("saved, 20 bytes"));
        assert_eq!(window.find("text-encoding").label(), Some("Latin-1"));
        assert_eq!(window.find("text-line-endings").label(), Some("LF"));
        // Ctrl+S on an untouched file leaves it alone.
        let stamp = std::fs::metadata(&latin).unwrap().modified().unwrap();
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::metadata(&latin).unwrap().modified().unwrap(), stamp, "an untouched file is not rewritten");
        // A character Latin-1 cannot hold refuses the save and says why.
        window.input("€", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), "X€caffè latte à Roma\n");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(
            window.find("text-message").label(),
            Some("Latin-1 cannot hold '€' (U+20AC) on line 1; choose another encoding")
        );
        assert!(view.read(cx).is_dirty());
        assert_eq!(std::fs::read(&latin).unwrap(), b"Xcaff\xe8 latte \xe0 Roma\n", "the file is untouched");
        // Clicking the encoding cycles to the next, Windows-1252, which holds the euro.
        window.click("text-encoding", cx);
        window.render_frame(cx);
        assert_eq!(window.find("text-encoding").label(), Some("Windows-1252 (was Latin-1)"));
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read(&latin).unwrap(), b"X\x80caff\xe8 latte \xe0 Roma\n");
        assert_eq!(window.find("text-encoding").label(), Some("Windows-1252"));
        assert!(!view.read(cx).is_dirty());
        // Once more to UTF-8, and the endings to CRLF: the bytes follow both.
        window.click("text-encoding", cx);
        window.render_frame(cx);
        window.click("text-line-endings", cx);
        window.render_frame(cx);
        assert_eq!(window.find("text-encoding").label(), Some("UTF-8 (was Windows-1252)"));
        assert_eq!(window.find("text-line-endings").label(), Some("CRLF (was LF)"));
        assert!(view.read(cx).is_dirty(), "a changed encoding or ending is a modification");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&latin).unwrap(), "X€caffè latte à Roma\r\n");
        assert_eq!(window.find("text-encoding").label(), Some("UTF-8"));
        assert_eq!(window.find("text-line-endings").label(), Some("CRLF"));
        // A UTF-16 file keeps its mark, byte order and CRLF endings.
        workspace.update(cx, |w, cx| w.open_paths(&[utf16.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[1].read(cx).text_view().unwrap().clone();
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Y", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        let expected: Vec<u8> = [0xff, 0xfe].into_iter().chain("Yhi\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        assert_eq!(std::fs::read(&utf16).unwrap(), expected);
        assert_eq!(window.find("text-encoding").label(), Some("UTF-16 LE"));
        assert_eq!(window.find("text-message").label(), Some("saved, 12 bytes"));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Undo, redo, cut, paste and the editor's find-and-replace change the buffer as the
/// clause states, and a save writes the result.
#[gpui_kit::test]
fn undo_redo_cut_paste_and_replace_change_the_buffer_as_stated(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-undo");
    let hello = copy_fixture("hello.txt", &dir);
    let original = "Sfoglia reads anything.\nSecond line.\n";

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        let editor = view.read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("ab", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), format!("ab{original}"));
        // Undo takes the typing back; redo brings it again.
        window.press("ctrl-z", cx);
        window.press("ctrl-z", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), original);
        window.press("ctrl-y", cx);
        window.press("ctrl-y", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), format!("ab{original}"));
        // Select all, cut, paste.
        window.press("ctrl-a", cx);
        window.press("ctrl-x", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), "");
        window.press("ctrl-v", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), format!("ab{original}"));
        // Ctrl+F opens the editor's own search panel; Escape closes it.
        window.press("ctrl-f", cx);
        window.render_frame(cx);
        assert!(editor.read(cx).search_session().open, "Ctrl+F opens the search panel");
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(!editor.read(cx).search_session().open);
        // Replace through the same engine the panel drives.
        editor.update(cx, |state, cx| {
            state.open_search(true, cx);
            assert!(state.search_session().replace_mode);
            state.set_search_query("line", false, cx);
            assert_eq!(state.replace_all_search_matches("LINE", window, cx), 1);
            state.close_search(cx);
        });
        assert_eq!(view.read(cx).text(cx).as_ref(), "abSfoglia reads anything.\nSecond LINE.\n");
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert!(view.read(cx).is_dirty());
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&hello).unwrap(), "abSfoglia reads anything.\nSecond LINE.\n");
        assert!(!view.read(cx).is_dirty());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file changed on disk by another program is noticed within the check interval and
/// offered for reload; keeping the buffer lets the next save overwrite the file.
#[gpui_kit::test]
fn a_file_changed_on_disk_is_offered_for_reload(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-disk");
    let hello = copy_fixture("hello.txt", &dir);

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
        window.render_frame(cx);
        assert!(window.try_find("text-changed-on-disk").is_none());
    })
    .unwrap();
    // Another program writes the file; the next check notices.
    std::fs::write(&hello, "changed elsewhere\n").unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert!(view.read(cx).changed_on_disk());
        assert_eq!(window.find("text-changed-on-disk").label(), Some("The file changed on disk."));
        assert_eq!(view.read(cx).text(cx).as_ref(), "Sfoglia reads anything.\nSecond line.\n", "nothing changes until asked");
        window.click("text-reload", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), "changed elsewhere\n");
        assert!(!view.read(cx).changed_on_disk());
        assert!(!view.read(cx).is_dirty());
        assert!(window.try_find("text-changed-on-disk").is_none());
        assert_eq!(window.find("text-lines").label(), Some("2 lines"));
        // Edit, then the file changes again: keep the buffer, and the next save overwrites.
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Z", cx);
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::write(&hello, "another writer\n").unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert!(window.try_find("text-changed-on-disk").is_some());
        assert_eq!(view.read(cx).text(cx).as_ref(), "Zchanged elsewhere\n");
        window.click("text-keep", cx);
        window.render_frame(cx);
        assert!(window.try_find("text-changed-on-disk").is_none());
        assert!(view.read(cx).is_dirty(), "the edit is kept");
        assert_eq!(view.read(cx).text(cx).as_ref(), "Zchanged elsewhere\n");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&hello).unwrap(), "Zchanged elsewhere\n");
    })
    .unwrap();
    // A save of our own is not a change made elsewhere.
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("text-changed-on-disk").is_none());
    })
    .unwrap();
    // Keep mine on a tab without edits: the text on screen now differs from the file, so
    // it counts as modified and the next save puts it back.
    std::fs::write(&hello, "third writer\n").unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let view = panel.read(cx).text_view().unwrap().clone();
        assert!(!view.read(cx).is_dirty());
        assert!(window.try_find("text-changed-on-disk").is_some());
        window.click("text-keep", cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_dirty(), "the kept text differs from the file");
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("hello.txt •"));
        assert_eq!(window.find("text-dirty").label(), Some("modified"));
        assert_eq!(std::fs::read_to_string(&hello).unwrap(), "third writer\n", "nothing written yet");
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&hello).unwrap(), "Zchanged elsewhere\n", "the next save overwrites");
        assert!(!view.read(cx).is_dirty());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A modified tab refuses to close; Revert restores the saved text and the tab closes.
#[gpui_kit::test]
fn a_modified_tab_cannot_be_closed_until_saved_or_reverted(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-close");
    let hello = copy_fixture("hello.txt", &dir);
    let other = copy_fixture("data.json", &dir);

    cx.update_window(handle, |_, window, cx| {
        // Two tabs: the dock never closes the last tab of its only group, so the second
        // tab gives the first somewhere to go. The text file, opened last, is active.
        workspace.update(cx, |w, cx| w.open_paths(&[other.clone(), hello.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("hello.txt".into()));
        let view = workspace.read(cx).panels(cx)[1].read(cx).text_view().unwrap().clone();
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Q", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[1].clone();
        assert!(panel.read(cx).is_dirty(cx));
        assert!(!panel.read(cx).closable(cx), "a modified tab is not closable");
        // The dock closes the active, focused panel: focus it as the C-004 test does.
        let focus = panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(ClosePanel), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 2, "the modified tab stays open");
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("hello.txt".into()));
        let panel = workspace.read(cx).panels(cx)[1].clone();
        let view = panel.read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), "QSfoglia reads anything.\nSecond line.\n");
        window.click("text-revert", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), "Sfoglia reads anything.\nSecond line.\n");
        assert!(!view.read(cx).is_dirty());
        assert!(panel.read(cx).closable(cx));
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("hello.txt"));
        assert_eq!(std::fs::read(&hello).unwrap(), fixture_bytes("hello.txt"), "revert writes nothing");
        let focus = panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.dispatch_action(Box::new(ClosePanel), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["data.json"], "the reverted tab closes");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file whose line endings were mixed is written with LF once edited, and its status
/// row then says LF; choosing CRLF writes CRLF.
#[gpui_kit::test]
fn a_mixed_endings_file_is_written_with_lf_and_says_so(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-mixed");
    let mixed = copy_fixture("mixed-endings.txt", &dir);

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[mixed.clone()], window, cx));
        window.render_frame(cx);
        assert_eq!(window.find("text-line-endings").label(), Some("mixed, saves as LF"));
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("X", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read(&mixed).unwrap(), b"Xone\ntwo\nthree\n");
        assert_eq!(window.find("text-line-endings").label(), Some("LF"), "the file now holds LF");
        assert_eq!(view.read(cx).status().line_endings, sfoglia::text::LineEndings::Lf);
        assert!(!view.read(cx).is_dirty());
        // CRLF chosen: the item says what was read until the save, then just CRLF.
        window.click("text-line-endings", cx);
        window.render_frame(cx);
        assert_eq!(window.find("text-line-endings").label(), Some("CRLF (was LF)"));
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read(&mixed).unwrap(), b"Xone\r\ntwo\r\nthree\r\n");
        assert_eq!(window.find("text-line-endings").label(), Some("CRLF"));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file with undecodable units is read-only: typing, replacing, the status-row choices
/// and Ctrl+S leave the buffer and the file as they are, and the status row says why.
#[gpui_kit::test]
fn a_file_with_undecodable_units_is_read_only(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-undecodable");
    let path = dir.join("lone-surrogate.txt");
    // UTF-16 LE: "h", a lone high surrogate, "i".
    let bytes: Vec<u8> = vec![0xff, 0xfe, 0x68, 0x00, 0x00, 0xd8, 0x69, 0x00];
    std::fs::write(&path, &bytes).unwrap();

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[path.clone()], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let view = panel.read(cx).text_view().expect("UTF-16 text opens in the text viewer").clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), "h\u{fffd}i");
        assert!(!view.read(cx).is_editable());
        let editor = view.read(cx).editor().clone();
        assert!(!editor.read(cx).is_editable(), "the editor itself is read-only");
        assert_eq!(window.find("text-replaced").label(), Some("1 undecodable, read-only"));
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Q", cx);
        assert_eq!(view.read(cx).text(cx).as_ref(), "h\u{fffd}i", "typing changes nothing");
        editor.update(cx, |state, cx| {
            state.open_search(true, cx);
            state.set_search_query("h", false, cx);
            assert_eq!(state.replace_all_search_matches("H", window, cx), 0, "nothing is replaced");
            state.close_search(cx);
        });
        assert_eq!(view.read(cx).text(cx).as_ref(), "h\u{fffd}i");
        window.click("text-encoding", cx);
        window.click("text-line-endings", cx);
        window.render_frame(cx);
        assert_eq!(window.find("text-encoding").label(), Some("UTF-16 LE"), "the choices do not move");
        assert_eq!(window.find("text-line-endings").label(), Some("none"));
        assert!(!view.read(cx).is_dirty());
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("lone-surrogate.txt"));
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(window.find("text-message").label(), Some("this file has undecodable units and is read-only"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file is untouched");
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[0].read(cx).text_view().unwrap().clone();
        assert!(!view.read(cx).is_dirty(), "no change event came from the read-only editor");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Reload or Revert on a file that can no longer be read keeps the edits on screen, says
/// why, leaves the tab modified and unclosable, and the next save recreates the file.
#[gpui_kit::test]
fn an_unreadable_file_keeps_the_edits_protected(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("edit-deleted");
    let other = copy_fixture("data.json", &dir);
    let hello = copy_fixture("hello.txt", &dir);

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[other.clone(), hello.clone()], window, cx));
        window.render_frame(cx);
        let view = workspace.read(cx).panels(cx)[1].read(cx).text_view().unwrap().clone();
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Q", cx);
    })
    .unwrap();
    cx.run_until_parked();
    // Another program deletes the file: the bar comes up; Reload cannot read it.
    std::fs::remove_file(&hello).unwrap();
    cx.background_executor.advance_clock(Duration::from_secs(2));
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[1].clone();
        let view = panel.read(cx).text_view().unwrap().clone();
        assert!(window.try_find("text-changed-on-disk").is_some(), "a deleted file is a change on disk");
        window.click("text-reload", cx);
        window.render_frame(cx);
        let message = window.find("text-message").label().unwrap_or_default().to_string();
        assert!(message.starts_with("cannot read "), "{message}");
        assert_eq!(view.read(cx).text(cx).as_ref(), "QSfoglia reads anything.\nSecond line.\n", "the edits stay");
        assert!(view.read(cx).is_dirty(), "and stay protected");
        assert_eq!(panel.read(cx).tab_name(cx).as_deref(), Some("hello.txt •"));
        assert!(!panel.read(cx).closable(cx));
        assert!(window.try_find("text-changed-on-disk").is_none());
        assert!(!hello.exists());
        // Ctrl+S recreates the file with the screen text.
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&hello).unwrap(), "QSfoglia reads anything.\nSecond line.\n");
        assert!(!view.read(cx).is_dirty());
        // The same through Revert, with the file gone again.
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("R", cx);
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::remove_file(&hello).unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[1].clone();
        let view = panel.read(cx).text_view().unwrap().clone();
        assert!(view.read(cx).is_dirty());
        window.click("text-revert", cx);
        window.render_frame(cx);
        assert!(window.find("text-message").label().unwrap_or_default().starts_with("cannot read "));
        // The caret stayed after the Q, so the R follows it.
        assert_eq!(view.read(cx).text(cx).as_ref(), "QRSfoglia reads anything.\nSecond line.\n");
        assert!(view.read(cx).is_dirty());
        assert!(!panel.read(cx).closable(cx));
        window.press("ctrl-s", cx);
        window.render_frame(cx);
        assert_eq!(std::fs::read_to_string(&hello).unwrap(), "QRSfoglia reads anything.\nSecond line.\n");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Type `text` into the panel's editor and let the change land.
fn type_into(cx: &mut TestAppContext, handle: AnyWindowHandle, workspace: &Entity<Workspace>, path: &std::path::Path, panel: &Entity<FilePanel>, text: &str) {
    cx.update_window(handle, |_, window, cx| {
        // Only the active tab is laid out, so its editor is the one that takes the keys: bring the tab up.
        workspace.update(cx, |w, cx| w.open_paths(&[path.to_path_buf()], window, cx));
        window.render_frame(cx);
        let view = panel.read(cx).text_view().expect("a text tab").clone();
        let focus = view.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input(text, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The kit's dialog slides in over a quarter of a second of real time, and a click aimed at one frame's bounds
/// can land where the button no longer is: let the entrance finish before reading or clicking.
fn settle_dialog(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    std::thread::sleep(Duration::from_millis(300));
    window.render_frame(cx);
    assert!(window.try_find("dialog").is_some(), "the closing dialog shows");
}

/// A folder as the dialog names it: canonical, Windows' `\\?\` prefix taken off (the temp folder's 8.3 spelling expands).
fn canonical_folder(path: &std::path::Path) -> String {
    let text = std::fs::canonicalize(path).unwrap().display().to_string();
    text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text)
}

/// The closing dialog's title, folder line, "more" line and message, as shown once its entrance is over.
fn closing_dialog(window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) -> (String, String, Option<String>, Option<String>) {
    settle_dialog(window, cx);
    (
        window.find("close-title").label().unwrap_or_default().to_string(),
        window.find("close-folder").label().unwrap_or_default().to_string(),
        window.try_find("close-more").and_then(|e| e.label().map(|l| l.to_string())),
        window.try_find("close-message").and_then(|e| e.label().map(|l| l.to_string())),
    )
}

/// The window with every tab saved closes at once; with modified tabs it asks about each in tab order,
/// Cancel keeps everything, Discard changes restores a tab and moves on, Save writes and the window
/// goes once the last tab is settled (C-012).
#[gpui_kit::test]
fn closing_the_window_asks_about_every_modified_tab_before_it_closes(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-ask");
    let hello = copy_fixture("hello.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), utf16.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert!(vcx.simulate_close(), "nothing modified: the window may close");
    assert_eq!(cx.windows().len(), 1, "the test's simulated close removes nothing by itself");

    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    assert!(!vcx.simulate_close(), "a modified tab holds the window");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (title, folder, more, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        assert_eq!(folder, canonical_folder(&dir));
        assert_eq!(more, None);
        assert_eq!(message, None);
        window.click("close-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none(), "Cancel closes the dialog");
        assert!(panels[0].read(cx).is_dirty(cx), "and leaves the tab modified");
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the window stays");

    type_into(cx, handle, &workspace, &utf16, &panels[1], "Y");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?", "the first modified tab in tab order");
        assert_eq!(more.as_deref(), Some("1 more file has unsaved changes"));
        assert!(window.try_find("close-save-all").is_some());
        assert!(window.try_find("close-discard-all").is_some());
        window.click("close-discard", cx);
        window.render_frame(cx);
        let view = panels[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), "Sfoglia reads anything.\nSecond line.\n", "restored");
        assert!(!panels[0].read(cx).is_dirty(cx));
        assert_eq!(std::fs::read(&hello).unwrap(), fixture_bytes("hello.txt"), "the file is untouched");
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “utf16le.txt”?", "the dialog moves to the next modified tab");
        assert_eq!(more, None);
        assert!(window.try_find("close-save-all").is_none(), "no more tabs follow");
        window.click("close-cancel", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert!(!panels[0].read(cx).is_dirty(cx), "the discarded tab stays restored");
        assert!(panels[1].read(cx).is_dirty(cx), "the other stays modified");
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);

    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (title, _, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “utf16le.txt”?");
        window.click("close-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let expected: Vec<u8> = [0xff, 0xfe].into_iter().chain("Yhi\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    assert_eq!(std::fs::read(&utf16).unwrap(), expected, "Save writes the exact bytes");
    assert!(cx.windows().is_empty(), "the last modified tab saved: the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save all writes every modified tab and the window goes (C-012).
#[gpui_kit::test]
fn save_all_writes_every_modified_tab_and_closes_the_window(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-save-all");
    let hello = copy_fixture("hello.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), utf16.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    type_into(cx, handle, &workspace, &utf16, &panels[1], "Y");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        assert_eq!(more.as_deref(), Some("1 more file has unsaved changes"));
        window.click("close-save-all", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&hello).unwrap(), b"QSfoglia reads anything.\nSecond line.\n");
    let expected: Vec<u8> = [0xff, 0xfe].into_iter().chain("Yhi\r\n".encode_utf16().flat_map(u16::to_le_bytes)).collect();
    assert_eq!(std::fs::read(&utf16).unwrap(), expected);
    assert!(cx.windows().is_empty(), "every tab saved: the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Discard all restores every modified tab, writes nothing, and the window goes (C-012).
#[gpui_kit::test]
fn discard_all_restores_every_modified_tab_and_closes_the_window(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-discard-all");
    let hello = copy_fixture("hello.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), utf16.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    type_into(cx, handle, &workspace, &utf16, &panels[1], "Y");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.click("close-discard-all", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&hello).unwrap(), fixture_bytes("hello.txt"), "nothing written");
    assert_eq!(std::fs::read(&utf16).unwrap(), fixture_bytes("utf16le.txt"));
    assert!(cx.windows().is_empty(), "every tab restored: the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A save the text viewer refuses keeps the dialog on that tab with the viewer's message and the window
/// open; Discard changes then restores the text and the window goes (C-012).
#[gpui_kit::test]
fn a_refused_save_keeps_the_window_open_with_its_message(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-refused");
    let latin = copy_fixture("latin1.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[latin.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &latin, &panels[0], "€");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.click("close-save", cx);
        window.render_frame(cx);
        let (title, _, _, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “latin1.txt”?", "the dialog stays on the tab");
        assert_eq!(message.as_deref(), Some("Latin-1 cannot hold '€' (U+20AC) on line 1; choose another encoding"));
        assert!(panels[0].read(cx).is_dirty(cx));
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&latin).unwrap(), fixture_bytes("latin1.txt"), "nothing written");
    assert_eq!(cx.windows().len(), 1, "the window stays");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("close-discard", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&latin).unwrap(), fixture_bytes("latin1.txt"));
    assert!(cx.windows().is_empty(), "the tab restored: the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Escape is Cancel: the dialog goes, the tab stays modified, and the next close request asks again (C-012).
#[gpui_kit::test]
fn escape_is_cancel_and_the_question_returns_on_the_next_request(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-escape");
    let hello = copy_fixture("hello.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.press("escape", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none(), "Escape closes the dialog");
        assert!(panels[0].read(cx).is_dirty(cx), "and leaves the tab modified");
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the window stays");
    assert!(!vcx.simulate_close(), "the next request is refused again");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, _, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?", "and asks again");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save all writes the tabs before a refused save, stops on it with the message, and leaves the later tabs
/// modified and their files untouched (C-012).
#[gpui_kit::test]
fn save_all_stops_at_a_refused_save_with_the_rest_untouched(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-save-all-refused");
    let hello = copy_fixture("hello.txt", &dir);
    let latin = copy_fixture("latin1.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), latin.clone(), utf16.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    type_into(cx, handle, &workspace, &latin, &panels[1], "€");
    type_into(cx, handle, &workspace, &utf16, &panels[2], "Y");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        assert_eq!(more.as_deref(), Some("2 more files have unsaved changes"));
        window.click("close-save-all", cx);
        window.render_frame(cx);
        let (title, _, more, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “latin1.txt”?", "Save all stops on the refused tab");
        assert_eq!(message.as_deref(), Some("Latin-1 cannot hold '€' (U+20AC) on line 1; choose another encoding"));
        assert_eq!(more.as_deref(), Some("1 more file has unsaved changes"), "only what is left counts");
        assert!(!panels[0].read(cx).is_dirty(cx), "the tab before it was saved");
        assert!(panels[1].read(cx).is_dirty(cx));
        assert!(panels[2].read(cx).is_dirty(cx), "the tab after it is still modified");
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&hello).unwrap(), b"QSfoglia reads anything.\nSecond line.\n", "written");
    assert_eq!(std::fs::read(&latin).unwrap(), fixture_bytes("latin1.txt"), "refused: untouched");
    assert_eq!(std::fs::read(&utf16).unwrap(), fixture_bytes("utf16le.txt"), "not reached: untouched");
    assert_eq!(cx.windows().len(), 1, "the window stays");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Discard changes on a file that can no longer be read restores nothing: the dialog stays on the tab with the
/// viewer's message and the window open, as after a refused save (C-012).
#[gpui_kit::test]
fn a_restore_the_viewer_refuses_keeps_the_window_open_like_a_refused_save(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-restore-refused");
    let hello = copy_fixture("hello.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    std::fs::remove_file(&hello).unwrap();
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.click("close-discard", cx);
        window.render_frame(cx);
        let (title, folder, _, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?", "the dialog stays on the tab");
        assert_eq!(folder, canonical_folder(&dir), "the folder is still shown as it stands on disk, the file gone");
        assert!(message.as_deref().unwrap_or("").starts_with("cannot read "), "with the viewer's message");
        assert!(panels[0].read(cx).is_dirty(cx), "nothing was restored");
        let view = panels[0].read(cx).text_view().unwrap().clone();
        assert_eq!(view.read(cx).text(cx).as_ref(), "QSfoglia reads anything.\nSecond line.\n", "the edit is kept");
        assert!(message.unwrap_or_default().ends_with("; Discard changes again drops the edit"), "and says what a second Discard does");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1, "the window stays");
    // Asked again and told twice: the edit is dropped as asked and the window goes.
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.click("close-discard", cx);
        window.render_frame(cx);
        assert!(window.try_find("close-message").is_some(), "the first Discard of a new request reports the failure again");
        window.click("close-discard", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.windows().is_empty(), "the second Discard drops the edit and the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}


/// The question follows the tabs as they stand, group by group and left to right, not the order the files
/// were opened in: a tab moved to the front of its group, then one split off to the left, come first (C-012).
#[gpui_kit::test]
fn the_question_follows_the_tabs_order_not_the_opening_order(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-tab-order");
    let hello = copy_fixture("hello.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let latin = copy_fixture("latin1.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), utf16.clone(), latin.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    type_into(cx, handle, &workspace, &utf16, &panels[1], "Y");
    type_into(cx, handle, &workspace, &latin, &panels[2], "Z");
    fn first_tabs(node: &PaneNode) -> Option<NodeId> {
        match node.kind() {
            PaneRef::Tabs { .. } => Some(node.id()),
            PaneRef::Split { children, .. } => children.iter().find_map(first_tabs),
        }
    }
    let order = |workspace: &Entity<Workspace>, cx: &gpui_kit::App| -> Vec<String> {
        workspace.read(cx).panels_in_tab_order(cx).iter().map(|p| p.read(cx).info().name.clone()).collect()
    };
    cx.update_window(handle, |_, window, cx| {
        let dock = workspace.read(cx).dock_area().clone();
        let node = first_tabs(dock.read(cx).layout(DockPlacement::Center).expect("the centre layout").root()).expect("a tab group");
        // utf16le.txt's tab in front of hello.txt's, as a drag onto the first tab would put it.
        dock.update(cx, |dock, cx| {
            dock.move_panel(PanelId::from(panels[1].entity_id()), InsertTarget::Tabs { node, ix: Some(0), activate: true }, window, cx)
        });
        window.render_frame(cx);
        assert_eq!(order(&workspace, cx), ["utf16le.txt", "hello.txt", "latin1.txt"], "the tabs as they stand");
        // latin1.txt split off to the left of the group: its own group comes first of all.
        dock.update(cx, |dock, cx| {
            dock.move_panel(
                PanelId::from(panels[2].entity_id()),
                InsertTarget::Split { node, placement: Placement::Left, size: None },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        assert_eq!(order(&workspace, cx), ["latin1.txt", "utf16le.txt", "hello.txt"], "group by group, left to right");
        let opened: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(opened, ["hello.txt", "utf16le.txt", "latin1.txt"], "the opening order is unchanged");
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “latin1.txt”?", "the first tab as the tabs stand");
        assert_eq!(more.as_deref(), Some("2 more files have unsaved changes"));
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Enter is Save: on the last modified tab it writes the file and the window goes; on a tab whose save is
/// refused the dialog stays with the message, a second Enter too (C-012).
#[gpui_kit::test]
fn enter_is_save_and_a_refused_enter_keeps_the_dialog(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-enter");
    let latin = copy_fixture("latin1.txt", &dir);
    let hello = copy_fixture("hello.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[latin.clone(), hello.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &latin, &panels[0], "€");
    type_into(cx, handle, &workspace, &hello, &panels[1], "Q");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, _, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “latin1.txt”?");
        window.press("enter", cx);
        window.render_frame(cx);
        let (title, _, _, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “latin1.txt”?", "a refused Enter keeps the dialog on the tab");
        assert_eq!(message.as_deref(), Some("Latin-1 cannot hold '€' (U+20AC) on line 1; choose another encoding"));
        window.press("enter", cx);
        window.render_frame(cx);
        let (_, _, _, message) = closing_dialog(window, cx);
        assert!(message.is_some(), "and a second Enter too");
        // Discard the refused tab: the dialog moves to hello.txt, where Enter saves and closes the window.
        window.click("close-discard", cx);
        window.render_frame(cx);
        let (title, _, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&hello).unwrap(), b"QSfoglia reads anything.\nSecond line.\n", "Enter saved");
    assert_eq!(std::fs::read(&latin).unwrap(), fixture_bytes("latin1.txt"), "the refused tab's file untouched");
    assert!(cx.windows().is_empty(), "the last modified tab saved: the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file opened by a relative path shows its folder as it stands on disk, absolute (C-012).
#[gpui_kit::test]
fn a_relative_path_shows_its_absolute_folder(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    // Relative to the crate root, where cargo runs the tests; nothing is written (Cancel).
    let relative = PathBuf::from("tests").join("..").join("tests").join("fixtures").join("hello.txt");
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[relative.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &relative, &panels[0], "Q");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, folder, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        let expected = std::env::current_dir().unwrap().join("tests").join("fixtures");
        assert_eq!(folder, canonical_folder(&expected), "the folder is absolute and resolved");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&relative).unwrap(), fixture_bytes("hello.txt"), "nothing written");
}

/// A second Discard changes on a tab whose file cannot be read back drops the edit for good: the tab closes,
/// the dialog moves on, and a later request no longer asks about it (C-012).
#[gpui_kit::test]
fn a_dropped_edit_closes_its_tab_and_is_not_asked_about_again(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-drop-tab");
    let hello = copy_fixture("hello.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), utf16.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    type_into(cx, handle, &workspace, &utf16, &panels[1], "Y");
    std::fs::remove_file(&hello).unwrap();
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.click("close-discard", cx);
        window.render_frame(cx);
        let (title, _, _, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?", "the refused restore keeps the dialog on the tab");
        assert!(message.is_some());
        window.click("close-discard", cx);
        window.render_frame(cx);
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “utf16le.txt”?", "the second Discard drops the edit and moves on");
        assert_eq!(more, None);
        window.click("close-cancel", cx);
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["utf16le.txt"], "the dropped tab is closed");
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, _, more, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “utf16le.txt”?", "the dropped edit is not asked about again");
        assert_eq!(more, None);
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Discard all stops at a restore that cannot read the file back, like Save all at a refused save; a second
/// Discard all drops that edit and goes on through the rest (C-012).
#[gpui_kit::test]
fn discard_all_stops_at_a_refused_restore_and_a_second_goes_on(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-discard-all-refused");
    let hello = copy_fixture("hello.txt", &dir);
    let utf16 = copy_fixture("utf16le.txt", &dir);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone(), utf16.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    type_into(cx, handle, &workspace, &utf16, &panels[1], "Y");
    std::fs::remove_file(&hello).unwrap();
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        settle_dialog(window, cx);
        window.click("close-discard-all", cx);
        window.render_frame(cx);
        let (title, _, more, message) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?", "Discard all stops on the refused restore");
        assert!(message.as_deref().unwrap_or("").starts_with("cannot read "));
        assert_eq!(more.as_deref(), Some("1 more file has unsaved changes"));
        assert!(panels[1].read(cx).is_dirty(cx), "the tab after it is still modified");
        window.click("close-discard-all", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&utf16).unwrap(), fixture_bytes("utf16le.txt"), "restored, nothing written");
    assert!(cx.windows().is_empty(), "the second Discard all drops the edit, restores the rest and the window is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file whose whole folder is gone still names its folder as it stood: the nearest folder on disk
/// canonical, the rest as spelled (C-012).
#[gpui_kit::test]
fn a_file_whose_folder_is_gone_still_names_its_folder_as_it_stood(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-folder-gone");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let hello = copy_fixture("hello.txt", &sub);
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &hello, &panels[0], "Q");
    std::fs::remove_dir_all(&sub).unwrap();
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, folder, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        assert_eq!(folder, PathBuf::from(canonical_folder(&dir)).join("sub").display().to_string(), "the folder as it stood");
        window.click("close-save", cx);
        window.render_frame(cx);
        let (_, folder, _, message) = closing_dialog(window, cx);
        assert!(message.as_deref().unwrap_or("").starts_with("cannot write "), "the save is refused: {message:?}");
        assert_eq!(folder, PathBuf::from(canonical_folder(&dir)).join("sub").display().to_string(), "and the folder line holds");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A `..` after a symbolic link climbs from the link's target, as the system reads and writes the file:
/// the folder line names that target's folder (C-012; links need no privilege on Unix alone).
#[cfg(unix)]
#[gpui_kit::test]
fn a_link_in_the_path_resolves_as_the_system_does(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-link");
    let real = dir.join("real");
    std::fs::create_dir_all(real.join("sub")).unwrap();
    let hello = copy_fixture("hello.txt", &real);
    std::os::unix::fs::symlink(real.join("sub"), dir.join("link")).unwrap();
    let spelled = dir.join("link").join("..").join("hello.txt");
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[spelled.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &spelled, &panels[0], "Q");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, folder, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        assert_eq!(folder, canonical_folder(&real), "the link's target's folder, as the system resolves it");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&hello).unwrap(), fixture_bytes("hello.txt"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file that is itself a link counts where the link sits: its folder line names the link's folder, with
/// the target present and after it is gone alike (C-012).
#[cfg(unix)]
#[gpui_kit::test]
fn a_file_that_is_a_link_counts_where_the_link_sits(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let dir = scratch_dir("close-file-link");
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let target = copy_fixture("hello.txt", &b);
    let link = a.join("hello.txt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut vcx = VisualTestContext::from_window(handle, cx);
    let panels = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[link.clone()], window, cx));
            window.render_frame(cx);
            workspace.read(cx).panels(cx)
        })
        .unwrap();
    cx.run_until_parked();
    type_into(cx, handle, &workspace, &link, &panels[0], "Q");
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (title, folder, _, _) = closing_dialog(window, cx);
        assert_eq!(title, "Save changes to “hello.txt”?");
        assert_eq!(folder, canonical_folder(&a), "where the link sits, the target present");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    std::fs::remove_file(&target).unwrap();
    assert!(!vcx.simulate_close());
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        let (_, folder, _, _) = closing_dialog(window, cx);
        assert_eq!(folder, canonical_folder(&a), "and the same with the target gone");
        window.click("close-cancel", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let _ = std::fs::remove_dir_all(&dir);
}
