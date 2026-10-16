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
