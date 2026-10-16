//! Headless tests of editing and saving (C-008): a real gpui-kit window edits a text
//! file, marks the tab, writes the file back in its own encoding and endings, notices a
//! change made elsewhere, and keeps a modified tab open.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::component::dock::{BasePanel as _, ClosePanel, Panel as _};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
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
