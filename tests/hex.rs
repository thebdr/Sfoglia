//! Headless tests of the hex view (C-007): a real gpui-kit window shows a mapped file as
//! rows of offset, hex and ASCII, toggles any file to it, goes to offsets and searches.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, ElementId, Entity, Focusable as _, TestAppContext};
use sfoglia::hex::SearchMode;
use sfoglia::viewers::hex_view::WINDOW_ROWS;
use sfoglia::{Workspace, window_options};

/// A workspace with a rule file path of this test alone (the file need not exist).
fn open_workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-hex-rules-{}-{n}.toml", std::process::id()));
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

/// The element id of hex row `ix`.
fn hex_row(ix: usize) -> ElementId {
    ("hex-row", ix).into()
}

/// A binary nobody claims opens as hex rows; any other file switches to the hex view and
/// back with Ctrl+Shift+B.
#[gpui_kit::test]
fn binary_files_open_in_the_hex_view_and_any_file_toggles_to_it(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let noise = fixture("noise.bin");
    let hello = fixture("hello.txt");
    let dot = fixture("dot.png");

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[noise.clone()], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let hex = panel.read(cx).hex_view().expect("a binary opens as hex").clone();
        assert!(panel.read(cx).text_view().is_none());
        assert!(window.try_find("file-viewer").is_none(), "the info rows give way to the hex view");
        assert_eq!(hex.read(cx).file().len(), 16);
        assert_eq!(
            window.find(hex_row(0)).label(),
            Some("00000000  00 01 02 03 fe ff 10 80  00 7f 00 01 02 03 fe ff  |................|")
        );
        assert!(window.try_find(hex_row(1)).is_none());
        assert_eq!(window.find("hex-size").label(), Some("16 bytes"));
        assert_eq!(window.find("hex-rows").label(), Some("1 rows"));
        assert_eq!(window.find("hex-mark").label(), Some("no mark"));
        assert_eq!(window.find("hex-search").label(), Some("text search"));

        // A text file opens in the editor; Ctrl+Shift+B with the editor focused shows its bytes.
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[1].clone();
        let editor = panel.read(cx).text_view().expect("text opens in the editor").read(cx).editor().clone();
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("ctrl-shift-b", cx);
        window.render_frame(cx);
        let hex = panel.read(cx).hex_view().expect("Ctrl+Shift+B shows the hex view").clone();
        assert!(panel.read(cx).text_view().is_none(), "the text view is hidden while hex shows");
        assert_eq!(hex.read(cx).file().len(), 37);
        assert_eq!(window.find("hex-rows").label(), Some("3 rows"));
        assert_eq!(
            window.find(hex_row(0)).label(),
            Some("00000000  53 66 6f 67 6c 69 61 20  72 65 61 64 73 20 61 6e  |Sfoglia reads an|")
        );
        assert_eq!(
            window.find(hex_row(1)).label(),
            Some("00000010  79 74 68 69 6e 67 2e 0a  53 65 63 6f 6e 64 20 6c  |ything..Second l|")
        );
        let last = window.find(hex_row(2)).label().unwrap_or_default().to_string();
        assert!(last.starts_with("00000020  69 6e 65 2e 0a   "), "{last}");
        assert!(last.ends_with("  |ine..           |"), "{last}");
        assert_eq!(last.len(), 78, "the last row is padded to the full width");
        assert_eq!(last, hex.read(cx).file().row(2, None).line());
        // The chord toggles back to the editor.
        window.press("ctrl-shift-b", cx);
        window.render_frame(cx);
        assert!(panel.read(cx).hex_view().is_none());
        assert!(panel.read(cx).text_view().is_some());
        assert_eq!(window.find("text-lines").label(), Some("3 lines"));
        assert!(window.try_find("hex-status").is_none());

        // A file whose family has no viewer yet shows its info rows and toggles too.
        workspace.update(cx, |w, cx| w.open_paths(&[dot.clone()], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[2].clone();
        assert_eq!(window.find("file-viewer").label(), Some("images"));
        assert!(panel.read(cx).hex_view().is_none());
        let focus = panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("ctrl-shift-b", cx);
        window.render_frame(cx);
        assert!(panel.read(cx).hex_view().is_some());
        assert_eq!(window.find("hex-rows").label(), Some("5 rows"));
        let first = window.find(hex_row(0)).label().unwrap_or_default().to_string();
        assert!(first.starts_with("00000000  89 50 4e 47 0d 0a 1a 0a  "), "{first}");
        assert!(window.try_find("file-viewer").is_none());
        window.press("ctrl-shift-b", cx);
        window.render_frame(cx);
        assert_eq!(window.find("file-viewer").label(), Some("images"));
        assert!(panel.read(cx).hex_view().is_none());
    })
    .unwrap();
}

/// Go-to-offset and search move the list and mark the bytes; the status row says where.
/// The file is 4096 rows of `rNNNNN abcdefg`, a NUL and a newline.
#[gpui_kit::test]
fn goto_offset_and_search_scroll_and_mark_the_hit(cx: &mut TestAppContext) {
    let (handle, workspace) = open_workspace(cx);
    let path = std::env::temp_dir().join(format!("sfoglia-hex-{}.bin", std::process::id()));
    let data: String = (0..4096).map(|i| format!("r{i:05} abcdefg\0\n")).collect();
    assert_eq!(data.len(), 65536);
    std::fs::write(&path, &data).unwrap();
    let hex_of = |workspace: &Entity<Workspace>, cx: &gpui_kit::App| {
        workspace.read(cx).panels(cx)[0].read(cx).hex_view().expect("the NUL makes it binary: hex").clone()
    };

    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[path.clone()], window, cx));
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).file().rows(), 4096);
        assert_eq!(hex.read(cx).top_row(), 0);
        let focus = hex.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        // Ctrl+Shift+G focuses the offset field; Enter goes there.
        window.press("ctrl-shift-g", cx);
        window.render_frame(cx);
        let goto = hex.read(cx).goto_input().clone();
        assert!(goto.read(cx).focus_handle(cx).is_focused(window), "Ctrl+Shift+G focuses the offset field");
        window.input("0x8000", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).mark(), Some(&(0x8000..0x8001)));
        assert_eq!(hex.read(cx).status(), "at 0x8000 (32768)");
        assert_eq!(hex.read(cx).top_row(), 2048, "the row holding the offset is at the top");
        assert_eq!(window.find("hex-mark").label(), Some("mark 0x8000..0x8001"));
        assert_eq!(window.find("hex-status-text").label(), Some("at 0x8000 (32768)"));
        assert_eq!(
            window.find(hex_row(2048)).label(),
            Some("00008000  72 30 32 30 34 38 20 61  62 63 64 65 66 67 00 0a  |r02048 abcdefg..|")
        );
        let parts = hex.read(cx).file().row(2048, hex.read(cx).mark());
        assert_eq!(parts.hex[1], "72");
        assert_eq!(parts.ascii[1], "r");
        assert!(window.try_find(hex_row(0)).is_none(), "row 0 is off screen");
        // Ctrl+Shift+F focuses the search field; Enter on text that occurs nowhere says so.
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        let search = hex.read(cx).search_input().clone();
        assert!(search.read(cx).focus_handle(cx).is_focused(window), "Ctrl+Shift+F focuses the search field");
        assert_eq!(window.find("hex-mode").label(), Some("text"));
        window.input("zzz", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).status(), "not found");
        assert_eq!(window.find("hex-status-text").label(), Some("not found"));
        assert_eq!(hex.read(cx).mark(), Some(&(0x8000..0x8001)), "a miss leaves the mark");
        // `r0010` matches r00100 to r00109: ten hits, one row apart, from row 100.
        let search = hex.read(cx).search_input().clone();
        search.update(cx, |state, cx| state.set_value("r0010", window, cx));
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).mark(), Some(&(1600..1605)));
        assert_eq!(hex.read(cx).status(), "hit at 0x640 (1600)");
        assert_eq!(window.find("hex-status-text").label(), Some("hit at 0x640 (1600)"));
        let top = hex.read(cx).top_row();
        assert!(top <= 100, "the hit row is on screen: top row {top}");
        assert!(window.try_find(hex_row(100)).is_some());
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).mark(), Some(&(1616..1621)), "the next hit, one row down");
        for _ in 0..8 {
            window.press("enter", cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).mark(), Some(&(1744..1749)), "the tenth hit, r00109");
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).status(), "no more hits after 0x6d0; Enter searches from the start");
        assert_eq!(hex.read(cx).mark(), Some(&(1744..1749)), "the mark stays on the last hit");
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).status(), "hit at 0x640 (1600)", "from the start again");
        // Ctrl+M switches to bytes: 72 30 30 32 is `r002`, first in row 200.
        window.press("ctrl-m", cx);
        window.render_frame(cx);
        assert_eq!(hex.read(cx).search_mode(), SearchMode::Bytes);
        assert_eq!(window.find("hex-search").label(), Some("bytes search"));
        assert_eq!(window.find("hex-mode").label(), Some("bytes"), "the mode button says so");
        // The button itself switches too: back to text, and once more to bytes.
        window.click("hex-mode", cx);
        window.render_frame(cx);
        assert_eq!(hex.read(cx).search_mode(), SearchMode::Text);
        assert_eq!(window.find("hex-mode").label(), Some("text"));
        window.click("hex-mode", cx);
        window.render_frame(cx);
        assert_eq!(hex.read(cx).search_mode(), SearchMode::Bytes);
        // The click took the focus; the chord brings it back to the search field.
        window.press("ctrl-shift-f", cx);
        window.render_frame(cx);
        let search = hex.read(cx).search_input().clone();
        assert!(search.read(cx).focus_handle(cx).is_focused(window));
        search.update(cx, |state, cx| state.set_value("72 30 30 32", window, cx));
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).mark(), Some(&(3200..3204)));
        assert_eq!(hex.read(cx).status(), "hit at 0xc80 (3200)");
        // A bad pattern is reported, not applied.
        let search = hex.read(cx).search_input().clone();
        search.update(cx, |state, cx| state.set_value("abc", window, cx));
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let hex = hex_of(&workspace, cx);
        assert_eq!(hex.read(cx).status(), "`abc` has an odd number of hexadecimal digits");
        assert_eq!(hex.read(cx).mark(), Some(&(3200..3204)));
        // Past the end marks the last byte and shows the last row; not an offset is reported.
        hex.update(cx, |hex, cx| hex.go_to("99999999", cx));
        window.render_frame(cx);
        assert_eq!(hex.read(cx).mark(), Some(&(65535..65536)));
        assert_eq!(hex.read(cx).status(), "at 0xffff (65535)");
        assert!(window.try_find(hex_row(4095)).is_some(), "the last row is on screen");
        assert!(hex.read(cx).top_row() > 4000, "top row {}", hex.read(cx).top_row());
        hex.update(cx, |hex, cx| hex.go_to("zz", cx));
        assert!(hex.read(cx).status().contains("not an offset"), "{}", hex.read(cx).status());
        assert_eq!(hex.read(cx).mark(), Some(&(65535..65536)));
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}

/// A 2 GiB file is mapped, not read: it opens at once, counts its rows exactly, and
/// go-to-offset reaches its middle and its last row.
#[gpui_kit::test]
fn a_two_gibibyte_file_opens_at_once_and_its_last_row_renders(cx: &mut TestAppContext) {
    use std::io::{Seek, SeekFrom, Write};
    let (handle, workspace) = open_workspace(cx);
    // One fixed name: a run that panics leaves the file behind, and the next run reuses it
    // instead of filling the disk.
    let path = std::env::temp_dir().join("sfoglia-huge.bin");
    {
        let mut file = std::fs::File::create(&path).unwrap();
        // Linux extends the file sparsely; NTFS allocates the whole 2 GiB unless the file
        // is flagged sparse first, which once filled the disk.
        if cfg!(windows) {
            let flagged = std::process::Command::new("fsutil")
                .args(["sparse", "setflag"])
                .arg(&path)
                .status()
                .map(|status| status.success())
                .unwrap_or(false);
            println!("2 GiB file: sparse flag {}", if flagged { "set" } else { "not set, the file is allocated in full" });
        }
        file.set_len(1 << 31).unwrap();
        file.seek(SeekFrom::End(0)).unwrap();
        file.write_all(b"END-OF-THE-FILE!").unwrap();
    }
    let len = (1usize << 31) + 16;
    let rows = (1usize << 27) + 1;
    let opened = Instant::now();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[path.clone()], window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    let first = opened.elapsed();
    println!("2 GiB file: open+first frame {first:?}");
    assert!(first.as_millis() < 1500, "a mapped file opens at once: {first:?}");

    cx.update_window(handle, |_, window, cx| {
        let hex = workspace.read(cx).panels(cx)[0].read(cx).hex_view().expect("hex").clone();
        assert_eq!(hex.read(cx).file().len(), len);
        assert_eq!(hex.read(cx).file().rows(), rows);
        assert_eq!(window.find("hex-size").label(), Some("2147483664 bytes"));
        assert_eq!(window.find("hex-rows").label(), Some("134217729 rows"));
        assert_eq!(
            window.find(hex_row(0)).label(),
            Some("00000000  00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00  |................|")
        );
        assert!(window.try_find(hex_row(200)).is_none(), "only a screenful of rows is laid out");
        // The middle: the row is at the top, exactly, inside a window of a million rows
        // centred on it.
        hex.update(cx, |hex, cx| hex.go_to("0x40000000", cx));
        window.render_frame(cx);
        let middle = 1usize << 26;
        assert_eq!(hex.read(cx).top_row(), middle);
        assert_eq!(hex.read(cx).base_row(), middle - WINDOW_ROWS / 2);
        assert!(window.try_find(hex_row(middle)).is_some(), "the row is on screen");
        assert_eq!(hex.read(cx).mark(), Some(&((1 << 30)..(1 << 30) + 1)));
        // Scrolling up to the window's edge slides the window; the top row stays put.
        let near_edge = hex.read(cx).base_row() + 4000;
        hex.update(cx, |hex, cx| hex.scroll_by_rows(-((middle - near_edge) as isize), cx));
        window.render_frame(cx);
        assert_eq!(hex.read(cx).top_row(), near_edge, "scrolled {} rows up", middle - near_edge);
        assert_eq!(hex.read(cx).base_row(), near_edge - WINDOW_ROWS / 2, "the window slid");
        assert!(window.try_find(hex_row(near_edge)).is_some());
        window.render_frame(cx);
        assert_eq!(hex.read(cx).top_row(), near_edge, "and stays");
        // A row near the window's edge, asked for directly, moves the window too.
        hex.update(cx, |hex, cx| hex.show_row_at_top(100, cx));
        window.render_frame(cx);
        assert_eq!((hex.read(cx).top_row(), hex.read(cx).base_row()), (100, 0));
        assert!(window.try_find(hex_row(100)).is_some());
        // Scrolling down from the start to the window's far edge slides it forward, and
        // scrolling on reaches rows the first window never held.
        assert_eq!(WINDOW_ROWS, 1_000_000, "the clause says a million rows");
        hex.update(cx, |hex, cx| hex.show_row_at_top(0, cx));
        window.render_frame(cx);
        assert_eq!((hex.read(cx).top_row(), hex.read(cx).base_row()), (0, 0));
        let far = WINDOW_ROWS - 4000;
        hex.update(cx, |hex, cx| hex.scroll_by_rows(far as isize, cx));
        window.render_frame(cx);
        assert_eq!(hex.read(cx).top_row(), far, "scrolled {far} rows down");
        assert_eq!(hex.read(cx).base_row(), far - WINDOW_ROWS / 2, "the window slid forward");
        assert!(window.try_find(hex_row(far)).is_some());
        hex.update(cx, |hex, cx| hex.scroll_by_rows(10_000, cx));
        window.render_frame(cx);
        assert_eq!(hex.read(cx).top_row(), far + 10_000, "past the first window's end");
        assert!(window.try_find(hex_row(far + 10_000)).is_some());
        window.render_frame(cx);
        assert_eq!(hex.read(cx).top_row(), far + 10_000, "and stays");
        // Past the end: the last row with its bytes.
        hex.update(cx, |hex, cx| hex.go_to("0xffffffffff", cx));
        window.render_frame(cx);
        assert_eq!(hex.read(cx).mark(), Some(&(len - 1..len)));
        assert_eq!(
            window.find(hex_row(rows - 1)).label(),
            Some("80000000  45 4e 44 2d 4f 46 2d 54  48 45 2d 46 49 4c 45 21  |END-OF-THE-FILE!|")
        );
        let top = hex.read(cx).top_row();
        assert!(top + 60 >= rows, "the end of the file is on screen: top row {top}");
        // A search runs over the whole mapping.
        let timed = Instant::now();
        hex.update(cx, |hex, cx| hex.find_next("THE-FILE", cx));
        println!("2 GiB file: search {:?}", timed.elapsed());
        assert_eq!(hex.read(cx).mark(), Some(&(len - 9..len - 1)));
    })
    .unwrap();
    let _ = std::fs::remove_file(&path);
}
