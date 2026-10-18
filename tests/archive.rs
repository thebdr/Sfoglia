//! Headless tests of the archive viewer (C-015): a real gpui-kit window opens a zip on its entry list,
//! sorted by a header click and filtered by the search field; Enter opens an entry inside Sfoglia, read-only,
//! in the viewer its name calls for, as a tab `archive › inside` whose folder goes with it; `Extract all` and
//! `Extract selected` write the entries into the folder the prompt answers, never outside it; an encrypted
//! 7z asks for its password and counts the wrong ones; a rar goes through the 7-Zip stand-in, or says what
//! to install until `Look again` finds one.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::dock::Panel as _;
use gpui_kit::component::table::ColumnSort;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use sfoglia::archive::{self, Container};
use sfoglia::detect::{Route, detect};
use sfoglia::generated::families::Family;
use sfoglia::tools::{Tool, ToolSet};
use sfoglia::viewers::ArchiveView;
use sfoglia::workspace::FilePanel;
use sfoglia::{Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext, tools: ToolSet) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-archive-rules-{}-{n}.toml", std::process::id()));
    let tools = Arc::new(tools);
    cx.update(gpui_kit::init);
    cx.update(sfoglia::init);
    cx.update(|cx| {
        gpui_kit::open_window(window_options(), cx, |window, cx| {
            cx.new(|cx| Workspace::with_tools(Some(rules_path.clone()), tools.clone(), window, cx))
        })
        .expect("open the main window headlessly")
    })
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

/// A stand-in tool: a batch file on Windows, a shell script elsewhere.
fn stub(name: &str) -> PathBuf {
    let extension = if cfg!(windows) { "cmd" } else { "sh" };
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("stubs").join(format!("{name}.{extension}"))
}

/// A fresh directory for this test's files.
fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sfoglia-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn label(window: &gpui_kit::Window, id: impl Into<gpui_kit::ElementId>) -> String {
    window.find(id).label().unwrap_or_default().to_string()
}

/// A tool appearing at `path` later in a test: a script that forwards to `real`.
fn install_wrapper(path: &std::path::Path, real: &std::path::Path) {
    let script = if cfg!(windows) {
        format!("@echo off\r\ncall \"{}\" %*\r\nexit /b %errorlevel%\r\n", real.display())
    } else {
        format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", real.display())
    };
    std::fs::write(path, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The tools with no 7-Zip: the library containers need none.
fn no_tools() -> ToolSet {
    ToolSet::stubbed([(Tool::SevenZip, None)])
}

/// Open `path` in a fresh workspace with `tools` and hand back its panel and archive view.
fn open_archive(
    cx: &mut TestAppContext,
    tools: ToolSet,
    path: PathBuf,
) -> (AnyWindowHandle, Entity<Workspace>, Entity<FilePanel>, Entity<ArchiveView>) {
    let (handle, workspace) = open_workspace(cx, tools);
    let (panel, view) = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[path], window, cx));
            window.render_frame(cx);
            let panel = workspace.read(cx).panels(cx)[0].clone();
            let view = panel.read(cx).archive_view().expect("the file opens in the archive viewer").clone();
            (panel, view)
        })
        .unwrap();
    cx.run_until_parked();
    (handle, workspace, panel, view)
}

/// The table's cells as it shows them, top to bottom.
fn rows(view: &Entity<ArchiveView>, cx: &gpui_kit::App) -> Vec<Vec<String>> {
    view.read(cx).table().read(cx).dump(cx).1
}

fn names(view: &Entity<ArchiveView>, cx: &gpui_kit::App) -> Vec<String> {
    view.read(cx).shown_names(cx)
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// Select display row `row` of the table, focus the rows and press Enter: the entry opens after the update.
fn press_enter_on(view: &Entity<ArchiveView>, row: usize, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    let table = view.read(cx).table().clone();
    table.update(cx, |table, cx| table.set_selected_row(row, cx));
    let focus = view.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    window.render_frame(cx);
    window.press("enter", cx);
}

#[test]
fn the_archive_extensions_map_to_their_family() {
    for name in [
        "a.zip", "A.ZIP", "a.jar", "a.7z", "a.rar", "a.cbz", "a.cbr", "a.tar", "a.tgz", "a.tar.gz", "a.tbz2", "a.tar.bz2", "a.txz",
        "a.tar.xz", "a.tzst", "a.tar.zst", "a.tar.lzma", "a.taz", "a.tar.z", "a.gz", "a.bz2", "a.xz", "a.lzma", "a.zst", "a.z",
        "a.cab", "a.iso", "a.rpm", "a.deb", "a.cpio", "a.lha", "a.lzh", "a.arj", "a.chm", "a.xar", "a.wim", "a.dmg", "a.vhd",
        "a.hfs", "a.dd", "a.deskthemepack", "a.apk", "a.war", "a.pk3", "a.pk4", "a.pkz", "a.zipx",
    ] {
        let d = detect(name, b"");
        assert_eq!((d.family, d.route, d.route.clause()), (Some(Family::Archives), Route::Family(Family::Archives), "C-015"), "{name}");
    }
    assert_eq!(Family::Archives.slug(), "archives");
    // The container is the archive module's reading, by name and signature.
    assert_eq!(archive::container_of("a.zip", b"PK\x03\x04"), Some(Container::Zip));
    assert_eq!(archive::container_of("a.zip", b"7z\xbc\xaf\x27\x1c"), Some(Container::SevenZ));
}

#[gpui_kit::test]
fn an_archive_opens_on_its_entry_list_sorted_and_searched(cx: &mut TestAppContext) {
    let path = fixture("bundle.zip");
    let (handle, workspace, panel, view) = open_archive(cx, no_tools(), path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.state_name(), "listed");
        assert_eq!(v.container(), Some(Container::Zip));
        let v_focus = v.focus_handle(cx);
        assert_eq!(panel.read(cx).tab_name(cx).map(|s| s.to_string()), Some("bundle.zip".to_string()));
        let (headers, cells) = v.table().read(cx).dump(cx);
        assert_eq!(headers, strings(&["Name", "Size", "Packed", "Modified", "Method", "CRC"]));
        assert_eq!(cells.len(), 5);
        assert_eq!(cells[0], strings(&["readme.md", "36", "38", "2026-10-18 12:00", "deflate", "860443A3"]));
        assert_eq!(cells[1], strings(&["data/", "", "", "2026-10-18 12:00", "", ""]), "a folder: no size, method or CRC");
        assert_eq!(cells[2], strings(&["data/table.csv", "142", "160", "2026-10-18 12:00", "bzip2", "8BF4FAD8"]));
        assert_eq!(cells[3], strings(&["data/dot.png", "67", "67", "2026-10-18 12:00", "stored", "90C4732E"]));
        assert_eq!(cells[4], strings(&["naïve.txt", "13", "22", "2026-10-18 12:00", "zstd", "DF6A9BBD"]));
        assert_eq!(label(window, "archive-summary"), "ZIP, 5 entries, 287 B packed from 258 B");
        assert_eq!(v.summary(), "ZIP, 5 entries, 287 B packed from 258 B");
        assert_eq!(label(window, "archive-message"), "Enter opens an entry inside Sfoglia, Ctrl+Shift+F finds one, the arrow in a header sorts");
        assert!(window.try_find("archive-tool").is_none(), "no 7-Zip is involved");
        assert!(window.try_find("archive-look-again").is_none());
        assert!(window.try_find("archive-password").is_none(), "nothing is encrypted");
        assert!(window.try_find("archive-hex").is_none());
        // The rows have the keyboard as soon as the tab opens.
        assert_eq!(window.focused(cx), Some(v_focus), "the entry rows are focused at open");
        // The arrows move the selection from the rows, Home and End jump, nothing selected before.
        assert_eq!(view.read(cx).selected_entry(cx), None);
        window.press("down", cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(0));
        window.press("down", cx);
        window.press("down", cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(2));
        window.press("up", cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(1));
        window.press("end", cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(4));
        window.press("down", cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(4), "the last row stays");
        window.press("home", cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(0));
        // A header click sorts descending, a second ascending, a third restores the archive's order.
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data/table.csv", "data/dot.png", "readme.md", "naïve.txt", "data/"]));
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), Some((1, ColumnSort::Descending)));
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data/", "naïve.txt", "readme.md", "data/dot.png", "data/table.csv"]));
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["readme.md", "data/", "data/table.csv", "data/dot.png", "naïve.txt"]));
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), None);
        // Names sort without regard to case, a tie broken by the exact name.
        window.click(("icon-sort", 0usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["readme.md", "naïve.txt", "data/table.csv", "data/dot.png", "data/"]));
        window.click(("icon-sort", 0usize), cx);
        window.click(("icon-sort", 0usize), cx);
        window.render_frame(cx);
        // Ctrl+Shift+F puts the keyboard in the search field; what is typed filters the names, any case.
        window.press("ctrl-shift-f", cx);
        let search = view.read(cx).search_input().clone();
        assert_eq!(window.focused(cx), Some(search.read(cx).focus_handle(cx)));
        window.input("DAT", cx);
    })
    .unwrap();
    // The field's change reaches the view after the update.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data/", "data/table.csv", "data/dot.png"]));
        assert_eq!(rows(&view, cx).len(), 3);
        assert_eq!(view.read(cx).table().read(cx).delegate().filter(), "DAT");
        // The filter and a sort compose.
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data/table.csv", "data/dot.png", "data/"]));
        window.click(("icon-sort", 1usize), cx);
        window.click(("icon-sort", 1usize), cx);
        // The text replaced through the keyboard: a value set from code fires no change.
        let search = view.read(cx).search_input().clone();
        let focus = search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.press("ctrl-a", cx);
        window.input("png", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data/dot.png"]));
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&view, cx).len(), 5, "an empty search shows every entry again");
        assert_eq!(label(window, "archive-summary"), "ZIP, 5 entries, 287 B packed from 258 B");
        // Under a sort, Enter, a double click and Extract selected take the row as shown, not the archive's index.
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx)[0], "data/table.csv");
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 2);
        assert_eq!(panels[1].read(cx).tab_name(cx).map(|s| s.to_string()), Some("bundle.zip › data/table.csv".to_string()), "the first shown row, sorted");
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        window.double_click(("entry", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let sorted = scratch_dir("archive-sorted-selected");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 2, "the double-clicked row is the same entry: its tab comes to the front");
        assert_eq!(workspace.read(cx).active_panel(cx).map(|p| p.entity_id()), Some(panels[1].entity_id()));
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_row(0, cx));
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(2), "the first shown row is the third entry");
        window.click("archive-extract-selected", cx);
    })
    .unwrap();
    let chosen = sorted.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), format!("1 entry extracted into {}", sorted.display()));
        assert_eq!(std::fs::read(sorted.join("data").join("table.csv")).unwrap(), std::fs::read(fixture("table.csv")).unwrap());
        assert!(!sorted.join("readme.md").exists(), "not the archive's first entry");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&sorted);
}

#[gpui_kit::test]
fn an_entry_opens_inside_sfoglia_read_only_in_its_own_viewer(cx: &mut TestAppContext) {
    let path = fixture("bundle.zip");
    let (handle, workspace, _panel, view) = open_archive(cx, no_tools(), path.clone());
    // Enter on a folder opens nothing and says so.
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        press_enter_on(&view, 1, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 1);
        assert_eq!(label(window, "archive-message"), "data/ is a folder");
        // Enter on readme.md: a Markdown tab named after the archive and the path inside.
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let (entry_panel, entry_file) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let panels = workspace.read(cx).panels(cx);
            assert_eq!(panels.len(), 2, "the entry opened in a tab of its own");
            let entry = panels[1].clone();
            let info = entry.read(cx).info().clone();
            let origin = info.origin.clone().expect("an entry tab knows its archive");
            assert_eq!(origin.archive, path);
            assert_eq!(origin.inside, "readme.md");
            assert_eq!(origin.label(), "bundle.zip › readme.md");
            assert_eq!(entry.read(cx).tab_name(cx).map(|s| s.to_string()), Some("bundle.zip › readme.md".to_string()));
            assert_eq!(info.name, "readme.md");
            assert!(info.path.starts_with(origin.folder.path()), "{} is in {}", info.path.display(), origin.folder.path().display());
            assert!(origin.folder.path().starts_with(std::env::temp_dir().join("sfoglia")), "{}", origin.folder.path().display());
            assert_eq!(std::fs::read(&info.path).unwrap(), b"# Bundle\n\nThree files and a folder.\n");
            assert_eq!(workspace.read(cx).active_panel(cx).map(|p| p.entity_id()), Some(entry.entity_id()), "the new tab is the active one");
            let markdown = entry.read(cx).markdown_view().expect("a .md entry opens in the Markdown viewer").clone();
            let text = markdown.read(cx).text_view().clone();
            assert!(!text.read(cx).is_editable(), "an entry is read-only");
            assert_eq!(view.read(cx).message(), Some("opening readme.md"));
            // Ctrl+S on the entry says why nothing is saved.
            let focus = entry.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.press("ctrl-s", cx);
            assert_eq!(text.read(cx).message(), Some("read-only: an entry of bundle.zip; extract it to edit a copy"));
            assert_eq!(text.update(cx, |text, cx| text.save(cx)), Err("read-only: an entry of bundle.zip; extract it to edit a copy".to_string()));
            (entry, info.path.clone())
        })
        .unwrap();
    // Enter on the same entry again brings its tab to the front rather than opening another.
    cx.update_window(handle, |_, window, cx| {
        let archive_panel = workspace.read(cx).panels(cx)[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 2, "no second tab for the same entry");
        assert_eq!(workspace.read(cx).active_panel(cx).map(|p| p.entity_id()), Some(entry_panel.entity_id()));
        // A spreadsheet entry opens in the spreadsheet viewer, an image in the image viewer: the viewer the
        // name calls for, out of the same archive.
        let archive_panel = workspace.read(cx).panels(cx)[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        press_enter_on(&view, 2, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 3);
        let sheet = panels[2].clone();
        assert_eq!(sheet.read(cx).tab_name(cx).map(|s| s.to_string()), Some("bundle.zip › data/table.csv".to_string()));
        let sheet_view = sheet.read(cx).sheet_view().expect("a .csv entry opens in the spreadsheet viewer").clone();
        assert_eq!(sheet_view.read(cx).table().read(cx).dump(cx).1.len(), 3, "the fixture's three records");
        assert!(sheet.read(cx).info().path.file_name().is_some_and(|n| n == "table.csv"));
        // Read-only: Ctrl+S says why with no raw text built, and a cell edit is refused with the same reason.
        let focus = sheet.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("ctrl-s", cx);
        assert_eq!(label(window, "sheet-message"), "read-only: an entry of bundle.zip; extract it to edit a copy");
        window.click(("cell", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The selection reaches the edit field after the update; then the edit itself.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let sheet_view = workspace.read(cx).panels(cx)[2].read(cx).sheet_view().unwrap().clone();
        let edit = sheet_view.read(cx).edit_input().clone();
        assert_eq!(edit.read(cx).value().as_ref(), "Rossi, Mario", "the clicked cell's text is in the edit field");
        edit.update(cx, |state, cx| state.set_value("X", window, cx));
        let focus = edit.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        let sheet = panels[2].clone();
        assert_eq!(label(window, "sheet-message"), "read-only: an entry of bundle.zip; extract it to edit a copy");
        assert!(!sheet.read(cx).is_dirty(cx), "the cell edit was refused");
        assert_eq!(std::fs::read(&sheet.read(cx).info().path).unwrap(), std::fs::read(fixture("table.csv")).unwrap(), "the entry's file is untouched");
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        // A double click opens too.
        window.double_click(("entry", 3usize * 8), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 4, "the double-clicked image opened");
        assert_eq!(panels[3].read(cx).tab_name(cx).map(|s| s.to_string()), Some("bundle.zip › data/dot.png".to_string()));
        assert!(panels[3].read(cx).image_view().is_some(), "a .png entry opens in the image viewer");
    })
    .unwrap();
    // The same path inside another archive is another entry, in a tab named after that archive, while the first
    // archive's tab of the same name is open.
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("bundle.7z")], window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let seven = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let panels = workspace.read(cx).panels(cx);
            assert_eq!(panels.len(), 5);
            let seven = panels[4].read(cx).archive_view().expect("the 7z opens in the archive viewer").clone();
            assert_eq!(names(&seven, cx)[0], "readme.md");
            press_enter_on(&seven, 0, window, cx);
            seven
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 6, "a tab of its own, not bundle.zip's readme.md");
        assert_eq!(panels[5].read(cx).tab_name(cx).map(|s| s.to_string()), Some("bundle.7z › readme.md".to_string()));
        assert_eq!(panels[5].read(cx).info().origin.as_ref().map(|o| o.archive.clone()), Some(fixture("bundle.7z")));
        assert_eq!(seven.read(cx).message(), Some("opening readme.md"));
        assert_eq!(workspace.read(cx).active_panel(cx).map(|p| p.entity_id()), Some(panels[5].entity_id()));
        // Closing the entry tab removes its folder.
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.remove_panel(entry_panel.clone(), window, cx));
    })
    .unwrap();
    drop(entry_panel);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 5);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!entry_file.exists(), "the entry's folder went with its tab: {}", entry_file.display());
    assert!(!entry_file.parent().unwrap().exists());
}

#[gpui_kit::test]
fn extract_all_and_selected_write_the_entries_into_the_chosen_folder(cx: &mut TestAppContext) {
    let path = fixture("bundle.zip");
    let (handle, _workspace, _panel, view) = open_archive(cx, no_tools(), path.clone());
    let all = scratch_dir("archive-extract-all");
    let one = scratch_dir("archive-extract-one");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Nothing selected: Extract selected says so and asks for no folder.
        window.click("archive-extract-selected", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), "select an entry first");
    })
    .unwrap();
    assert!(!cx.did_prompt_for_paths(), "nothing selected: no folder asked for");
    cx.update_window(handle, |_, window, cx| {
        window.click("archive-extract-all", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths(), "Extract all asks for a folder");
    let chosen = all.clone();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories && !options.files && !options.multiple);
        Some(vec![chosen])
    });
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), format!("5 entries extracted into {}", all.display()));
        assert_eq!(std::fs::read(all.join("readme.md")).unwrap(), b"# Bundle\n\nThree files and a folder.\n");
        assert!(all.join("data").is_dir());
        assert_eq!(std::fs::read(all.join("data").join("table.csv")).unwrap(), std::fs::read(fixture("table.csv")).unwrap());
        assert_eq!(std::fs::read(all.join("data").join("dot.png")).unwrap(), std::fs::read(fixture("dot.png")).unwrap());
        assert_eq!(std::fs::read(all.join("naïve.txt")).unwrap(), b"Unicode name\n");
        // One entry selected: that one alone, under its path.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_row(2, cx));
        window.render_frame(cx);
        assert_eq!(view.read(cx).selected_entry(cx), Some(2));
        window.click("archive-extract-selected", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    let chosen = one.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), format!("1 entry extracted into {}", one.display()));
        assert_eq!(std::fs::read(one.join("data").join("table.csv")).unwrap(), std::fs::read(fixture("table.csv")).unwrap());
        assert!(!one.join("readme.md").exists());
        assert!(!one.join("data").join("dot.png").exists());
        // A cancelled prompt extracts nothing and says nothing new.
        window.click("archive-extract-all", cx);
    })
    .unwrap();
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), format!("1 entry extracted into {}", one.display()));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&all);
    let _ = std::fs::remove_dir_all(&one);
    // An entry whose path climbs out of the folder is skipped and named; the rest is written.
    let (handle, _workspace, _panel, _view) = open_archive(cx, no_tools(), fixture("escape.zip"));
    let out = scratch_dir("archive-extract-escape");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&_view, cx), strings(&["../escape.txt", "ok.txt"]), "the list shows the entry as named");
        window.click("archive-extract-all", cx);
    })
    .unwrap();
    let chosen = out.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), format!("1 entry extracted into {}; skipped as outside it: ../escape.txt", out.display()));
        assert_eq!(std::fs::read_to_string(out.join("ok.txt")).unwrap(), "fine\n");
        assert!(!out.parent().unwrap().join("escape.txt").exists(), "nothing climbed out");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&out);
}

#[gpui_kit::test]
fn an_encrypted_seven_zip_asks_for_its_password(cx: &mut TestAppContext) {
    let path = fixture("secret.7z");
    let (handle, workspace, _panel, view) = open_archive(cx, no_tools(), path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.state_name(), "locked");
        assert_eq!(v.container(), Some(Container::SevenZ));
        assert_eq!(label(window, "archive-summary"), "7Z, encrypted");
        assert_eq!(label(window, "archive-message"), "the archive is encrypted: it needs its password");
        assert_eq!(label(window, "archive-hint"), "the archive is encrypted: it needs its password");
        assert!(window.try_find("archive-password").is_some(), "the password field is in the tools row");
        assert!(window.try_find("archive-rows").is_none());
        assert_eq!(rows(&view, cx).len(), 0);
        let password = v.password_input().clone();
        assert_eq!(window.focused(cx), Some(password.read(cx).focus_handle(cx)), "the password field has the keyboard");
        window.input("nope", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).state_name(), "locked");
        assert_eq!(label(window, "archive-message"), "wrong password (try 1)");
        let password = view.read(cx).password_input().clone();
        password.update(cx, |state, cx| state.set_value("still wrong", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), "wrong password (try 2)", "each wrong one is said anew");
        password.update(cx, |state, cx| state.set_value("sfoglia", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.state_name(), "listed");
        assert_eq!(names(&view, cx), strings(&["secret.txt"]));
        let cells = rows(&view, cx);
        assert_eq!(cells[0][0..2], strings(&["secret.txt", "25"]));
        assert_eq!(cells[0][3..5], strings(&[&archive::local_time(1_792_324_800), "LZMA2+AES"]), "a 7z's instant in this machine's zone");
        let summary = label(window, "archive-summary");
        assert!(summary.starts_with("7Z, 1 entry, ") && summary.ends_with(" packed from 25 B"), "{summary}");
        assert!(window.try_find("archive-password").is_some(), "the field stays: the entries are encrypted");
        assert_eq!(label(window, "archive-message"), "Enter opens an entry inside Sfoglia, Ctrl+Shift+F finds one, the arrow in a header sorts");
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 2);
        let entry = panels[1].clone();
        assert_eq!(entry.read(cx).tab_name(cx).map(|s| s.to_string()), Some("secret.7z › secret.txt".to_string()));
        let text = entry.read(cx).text_view().expect("a .txt entry opens in the text viewer").clone();
        assert_eq!(text.read(cx).editor().read(cx).value().to_string(), "The password is sfoglia.\n");
        assert!(!text.read(cx).is_editable());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_exotic_container_goes_through_seven_zip_or_says_what_to_install(cx: &mut TestAppContext) {
    let dir = scratch_dir("archive-seven-zip");
    let later = dir.join(if cfg!(windows) { "7z.cmd" } else { "7z" });
    let tools = ToolSet::stubbed([(Tool::SevenZip, Some(later.clone()))]);
    let path = fixture("fake.rar");
    let (handle, workspace, _panel, view) = open_archive(cx, tools, path.clone());
    let expected = "A RAR needs 7-Zip, and none is installed: install 7-Zip from https://www.7-zip.org, or set SFOGLIA_7ZIP to its path. The bytes show meanwhile.";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.state_name(), "missing");
        assert_eq!(v.container(), Some(Container::External));
        assert!(v.tool().is_none());
        assert_eq!(label(window, "archive-summary"), "RAR");
        assert_eq!(label(window, "archive-tool"), "no 7-Zip");
        assert_eq!(label(window, "archive-message"), expected);
        assert_eq!(label(window, "archive-status"), expected);
        assert!(window.try_find("archive-look-again").is_some());
        assert!(window.try_find("archive-rows").is_none());
        assert!(v.hex_view().is_some(), "the bytes show meanwhile");
        assert!(window.try_find("archive-hex").is_some());
        let row = window.find(("hex-row", 0usize)).label().unwrap_or_default().to_string();
        assert!(row.starts_with("00000000  52 61 72 21 1a 07 01 00"), "Rar!: {row}");
        // 7-Zip appears: Look again lists the archive through it (the stand-in refuses a call without -sccUTF-8).
        install_wrapper(&later, &stub("sevenzip"));
        window.click("archive-look-again-button", cx);
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.state_name(), "listed");
        assert_eq!(v.tool().map(|f| f.path.clone()), Some(later.clone()));
        assert_eq!(label(window, "archive-tool"), "7-Zip 24.08");
        assert!(window.try_find("archive-look-again").is_none());
        assert!(window.try_find("archive-hex").is_none(), "the listing replaces the bytes");
        assert_eq!(names(&view, cx), strings(&["docs/readme.md", "docs/", "big.bin", "../climb.txt"]));
        let cells = rows(&view, cx);
        assert_eq!(cells[0], strings(&["docs/readme.md", "20", "18", "2026-10-18 12:00", "RAR5:LZSS", "1A2B3C4D"]));
        assert_eq!(cells[1], strings(&["docs/", "", "", "2026-10-18 12:00", "", ""]));
        assert_eq!(cells[2], strings(&["big.bin", "1048576", "1000", "2026-10-18 12:00", "RAR5:LZSS", "00000000"]));
        assert_eq!(cells[3], strings(&["../climb.txt", "4", "4", "2026-10-18 12:00", "RAR5:LZSS", "5E8D2F51"]));
        assert_eq!(label(window, "archive-summary"), "RAR, 4 entries, 1022 B packed from 1.0 MB");
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let out = scratch_dir("archive-seven-zip-out");
    let one = scratch_dir("archive-seven-zip-one");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 2, "the entry came through 7-Zip's -so");
        let entry = panels[1].clone();
        assert_eq!(entry.read(cx).tab_name(cx).map(|s| s.to_string()), Some("fake.rar › docs/readme.md".to_string()));
        assert!(entry.read(cx).markdown_view().is_some());
        let bytes = std::fs::read(&entry.read(cx).info().path).unwrap();
        assert!(bytes.starts_with(b"# Readme from a rar"), "{}", String::from_utf8_lossy(&bytes));
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        window.click("archive-extract-all", cx);
    })
    .unwrap();
    let chosen = out.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        // Extract all names nothing to 7-Zip (it writes everything); the climbing entry is still skipped and named.
        assert_eq!(
            label(window, "archive-message"),
            format!("3 entries extracted into {}; skipped as outside it: ../climb.txt", out.display())
        );
        assert!(std::fs::read(out.join("docs").join("readme.md")).unwrap().starts_with(b"# Readme from a rar"));
        assert!(out.join("big.bin").is_file());
        assert!(!out.parent().unwrap().join("climb.txt").exists(), "nothing climbed out");
        assert!(!out.join("climb.txt").exists(), "7-Zip was never asked for the climbing entry (it would write it inside, the `..` stripped)");
        // Extract selected names the one entry: the stand-in writes that alone.
        let table = view.read(cx).table().clone();
        table.update(cx, |table, cx| table.set_selected_row(2, cx));
        window.render_frame(cx);
        window.click("archive-extract-selected", cx);
    })
    .unwrap();
    let chosen = one.clone();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), format!("1 entry extracted into {}", one.display()));
        assert!(one.join("big.bin").is_file());
        assert!(!one.join("docs").exists(), "only the selected entry was asked for");
        // An encrypted archive through 7-Zip: the stand-in refuses every password but the right one.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("fake-secret.rar")], window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let secret = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let panels = workspace.read(cx).panels(cx);
            assert_eq!(panels.len(), 3);
            let secret = panels[2].read(cx).archive_view().expect("a rar opens in the archive viewer").clone();
            let v = secret.read(cx);
            assert_eq!(v.state_name(), "locked");
            assert_eq!(label(window, "archive-summary"), "RAR, encrypted");
            assert_eq!(label(window, "archive-tool"), "7-Zip 24.08");
            assert_eq!(label(window, "archive-message"), "the archive is encrypted: it needs its password");
            assert!(window.try_find("archive-password").is_some());
            let password = v.password_input().clone();
            assert_eq!(window.focused(cx), Some(password.read(cx).focus_handle(cx)));
            window.input("nope", cx);
            window.press("enter", cx);
            secret
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(secret.read(cx).state_name(), "locked");
        assert_eq!(label(window, "archive-message"), "wrong password (try 1)");
        let password = secret.read(cx).password_input().clone();
        password.update(cx, |state, cx| state.set_value("sfoglia", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        assert_eq!(secret.read(cx).state_name(), "listed");
        assert_eq!(names(&secret, cx), strings(&["docs/readme.md", "docs/", "big.bin", "../climb.txt"]));
        assert_eq!(label(window, "archive-summary"), "RAR, 4 entries, 1022 B packed from 1.0 MB");
        assert!(window.try_find("archive-password").is_none(), "the entries list in the clear once the password is right");
        // An archive whose entries are encrypted under clear headers (a rar made with plain -p): listed at once,
        // the password field beside the list, a wrong password said on Unlock, the right one opens.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("fake-entries.rar")], window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    let locked = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let panels = workspace.read(cx).panels(cx);
            assert_eq!(panels.len(), 4);
            let locked = panels[3].read(cx).archive_view().expect("the archive viewer").clone();
            assert_eq!(locked.read(cx).state_name(), "listed");
            assert!(locked.read(cx).listing().is_some_and(|l| l.encrypted));
            assert_eq!(names(&locked, cx), strings(&["docs/readme.md", "docs/", "big.bin", "../climb.txt"]));
            assert_eq!(rows(&locked, cx)[0][4], "AES RAR5:LZSS");
            assert!(window.try_find("archive-password").is_some(), "the entries are encrypted: the field shows beside the list");
            press_enter_on(&locked, 0, window, cx);
            locked
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 4, "nothing opened without the password");
        assert_eq!(label(window, "archive-message"), "the archive is encrypted: it needs its password");
        let password = locked.read(cx).password_input().clone();
        password.update(cx, |state, cx| state.set_value("nope", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), "wrong password (try 1)");
        assert_eq!(locked.read(cx).state_name(), "listed");
        assert_eq!(names(&locked, cx).len(), 4, "the list stays");
        password.update(cx, |state, cx| state.set_value("sfoglia", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        assert_eq!(locked.read(cx).message(), None);
        press_enter_on(&locked, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 5);
        assert_eq!(panels[4].read(cx).tab_name(cx).map(|s| s.to_string()), Some("fake-entries.rar › docs/readme.md".to_string()));
        assert!(std::fs::read(&panels[4].read(cx).info().path).unwrap().starts_with(b"# Readme from a rar"));
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&one);
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn other_entry_kinds_open_in_their_viewers_and_every_column_sorts(cx: &mut TestAppContext) {
    // No tool of the machine's: the PDF entry shows its hint, the zip is read by the library.
    let tools = ToolSet::stubbed([
        (Tool::SevenZip, None),
        (Tool::SumatraPdf, None),
        (Tool::MuTool, None),
        (Tool::PdfToPpm, None),
        (Tool::PdfInfo, None),
    ]);
    let path = fixture("more.zip");
    let (handle, workspace, _panel, view) = open_archive(cx, tools, path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data.json", "three.pdf", "inner.zip", "noise.bin", "d/", "d/n.txt"]));
        let cells = rows(&view, cx);
        assert_eq!(cells[0], strings(&["data.json", "130", "106", "2026-10-18 12:00", "deflate", &format!("{:08X}", 359138865u32)]));
        assert_eq!(cells[4], strings(&["d/", "", "", "", "", ""]), "a folder the zip never recorded, filled in");
        assert_eq!(cells[5], strings(&["d/n.txt", "7", "9", "2026-10-18 12:04", "deflate", &format!("{:08X}", 3953890701u32)]));
        assert_eq!(label(window, "archive-summary"), "ZIP, 6 entries, 811 B packed from 1.6 KB");
        // Every column sorts, descending first; the folder with no size, date, method or CRC sorts last.
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["three.pdf", "inner.zip", "data.json", "noise.bin", "d/n.txt", "d/"]), "by size");
        window.click(("icon-sort", 2usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["three.pdf", "inner.zip", "data.json", "noise.bin", "d/n.txt", "d/"]), "by packed size");
        window.click(("icon-sort", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["d/n.txt", "noise.bin", "inner.zip", "three.pdf", "data.json", "d/"]), "by date");
        window.click(("icon-sort", 3usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["d/", "data.json", "three.pdf", "inner.zip", "noise.bin", "d/n.txt"]), "by date, ascending");
        window.click(("icon-sort", 4usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["noise.bin", "inner.zip", "data.json", "d/n.txt", "three.pdf", "d/"]), "by method, a tie by name");
        window.click(("icon-sort", 5usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["d/n.txt", "noise.bin", "three.pdf", "inner.zip", "data.json", "d/"]), "by CRC");
        window.click(("icon-sort", 5usize), cx);
        window.click(("icon-sort", 5usize), cx);
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["data.json", "three.pdf", "inner.zip", "noise.bin", "d/", "d/n.txt"]), "the archive's order again");
        // A structured-data entry opens in the data viewer, read-only: Ctrl+S says why with no text built.
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let reason = "read-only: an entry of more.zip; extract it to edit a copy";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 2);
        let data = panels[1].clone();
        assert_eq!(data.read(cx).tab_name(cx).map(|s| s.to_string()), Some("more.zip › data.json".to_string()));
        let data_view = data.read(cx).data_view().expect("a .json entry opens in the data viewer").clone();
        assert!(data_view.read(cx).text_view().is_none(), "no raw text built yet");
        let focus = data.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("ctrl-s", cx);
        assert_eq!(label(window, "data-message"), reason);
        assert!(!data.read(cx).is_dirty(cx));
        // The PDF entry opens in the PDF viewer (its hint, with no tool pinned), the nested zip in the archive viewer.
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        press_enter_on(&view, 1, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 3);
        assert_eq!(panels[2].read(cx).tab_name(cx).map(|s| s.to_string()), Some("more.zip › three.pdf".to_string()));
        assert!(panels[2].read(cx).pdf_view().is_some(), "a .pdf entry opens in the PDF viewer");
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        press_enter_on(&view, 2, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let inner = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let panels = workspace.read(cx).panels(cx);
            assert_eq!(panels.len(), 4);
            assert_eq!(panels[3].read(cx).tab_name(cx).map(|s| s.to_string()), Some("more.zip › inner.zip".to_string()));
            let inner = panels[3].read(cx).archive_view().expect("a .zip entry opens in the archive viewer").clone();
            assert_eq!(names(&inner, cx), strings(&["note.txt"]));
            assert_eq!(label(window, "archive-summary"), "ZIP, 1 entry, 26 B packed from 26 B");
            // Its own entry opens from it, named after the nested archive.
            press_enter_on(&inner, 0, window, cx);
            inner
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 5);
        assert_eq!(panels[4].read(cx).tab_name(cx).map(|s| s.to_string()), Some("inner.zip › note.txt".to_string()));
        let text = panels[4].read(cx).text_view().expect("a .txt entry opens in the text viewer").clone();
        assert_eq!(text.read(cx).editor().read(cx).value().to_string(), "A note, compressed alone.\n");
        assert!(!text.read(cx).is_editable());
        assert_eq!(inner.read(cx).message(), Some("opening note.txt"));
        // Bytes nobody claims open in the hex view.
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        press_enter_on(&view, 3, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 6);
        assert_eq!(panels[5].read(cx).tab_name(cx).map(|s| s.to_string()), Some("more.zip › noise.bin".to_string()));
        assert!(panels[5].read(cx).hex_view().is_some(), "a binary entry opens on its bytes");
        assert!(window.try_find("hex-status").is_some());
        // The filled-in folder opens nothing; the file inside it opens as text.
        let archive_panel = panels[0].clone();
        let id = gpui_kit::component::dock::PanelId::from(archive_panel.entity_id());
        workspace.read(cx).dock_area().clone().update(cx, |dock, cx| dock.select_panel(id, window, cx));
        window.render_frame(cx);
        press_enter_on(&view, 4, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 6);
        assert_eq!(label(window, "archive-message"), "d/ is a folder");
        press_enter_on(&view, 5, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 7);
        assert_eq!(panels[6].read(cx).tab_name(cx).map(|s| s.to_string()), Some("more.zip › d/n.txt".to_string()));
        let text = panels[6].read(cx).text_view().expect("a .txt entry opens in the text viewer").clone();
        assert_eq!(text.read(cx).editor().read(cx).value().to_string(), "nested\n");
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_zip_with_encrypted_entries_lists_and_checks_the_password_on_unlock(cx: &mut TestAppContext) {
    // An AES zip lists in the clear: the password field shows beside the list, a wrong one is said on Unlock and
    // on opening an entry, each counted, and the search and the sort survive every re-listing.
    let path = fixture("secret.zip");
    let (handle, workspace, _panel, view) = open_archive(cx, no_tools(), path.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.state_name(), "listed");
        assert!(v.listing().is_some_and(|l| l.encrypted));
        assert_eq!(names(&view, cx), strings(&["secret.txt"]));
        assert_eq!(rows(&view, cx)[0], strings(&["secret.txt", "25", "55", "2026-10-18 12:00", "deflate", &format!("{:08X}", 2629368947u32)]));
        assert_eq!(label(window, "archive-summary"), "ZIP, 1 entry, 55 B packed from 25 B");
        assert!(window.try_find("archive-password").is_some(), "the entries are encrypted: the field shows beside the list");
        assert!(window.try_find("archive-rows").is_some());
        assert_eq!(label(window, "archive-message"), "Enter opens an entry inside Sfoglia, Ctrl+Shift+F finds one, the arrow in a header sorts");
        // Enter with no password: the entry says what it needs.
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 1, "nothing opened");
        assert_eq!(label(window, "archive-message"), "the archive is encrypted: it needs its password");
        // A wrong password on Unlock: said and counted, the list still up.
        let password = view.read(cx).password_input().clone();
        let focus = password.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.input("nope", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).state_name(), "listed");
        assert_eq!(names(&view, cx), strings(&["secret.txt"]));
        assert_eq!(label(window, "archive-message"), "wrong password (try 1)");
        // Enter with the wrong password kept: counted again, nothing opened.
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).panels(cx).len(), 1);
        assert_eq!(label(window, "archive-message"), "wrong password (try 2)");
        // A search and a sort, then another wrong password: the re-listing keeps both.
        window.press("ctrl-shift-f", cx);
        window.input("sec", cx);
        window.click(("icon-sort", 1usize), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).table().read(cx).delegate().filter(), "sec");
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), Some((1, ColumnSort::Descending)));
        let password = view.read(cx).password_input().clone();
        password.update(cx, |state, cx| state.set_value("still wrong", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        assert_eq!(label(window, "archive-message"), "wrong password (try 3)");
        assert_eq!(view.read(cx).state_name(), "listed");
        assert_eq!(view.read(cx).table().read(cx).delegate().filter(), "sec", "the search survives the re-listing");
        assert_eq!(view.read(cx).table().read(cx).delegate().sort(), Some((1, ColumnSort::Descending)), "and so does the sort");
        assert_eq!(names(&view, cx), strings(&["secret.txt"]));
        // A search that keeps nothing survives too: the right password lists, the filter still hides the entry.
        let search = view.read(cx).search_input().clone();
        let focus = search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.press("ctrl-a", cx);
        window.input("zzz", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&view, cx), Vec::<String>::new());
        let password = view.read(cx).password_input().clone();
        password.update(cx, |state, cx| state.set_value("sfoglia", window, cx));
        window.click("archive-unlock", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).state_name(), "listed");
        assert_eq!(view.read(cx).message(), None, "the right password says nothing");
        assert_eq!(names(&view, cx), Vec::<String>::new(), "the search still hides the entry");
        assert_eq!(view.read(cx).table().read(cx).delegate().filter(), "zzz");
        let search = view.read(cx).search_input().clone();
        let focus = search.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&view, cx), strings(&["secret.txt"]));
        assert!(window.try_find("archive-password").is_some(), "the field stays: the entries are still encrypted");
        press_enter_on(&view, 0, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        assert_eq!(panels.len(), 2, "the right password opens the entry");
        assert_eq!(panels[1].read(cx).tab_name(cx).map(|s| s.to_string()), Some("secret.zip › secret.txt".to_string()));
        let text = panels[1].read(cx).text_view().expect("a .txt entry opens in the text viewer").clone();
        assert_eq!(text.read(cx).editor().read(cx).value().to_string(), "The password is sfoglia.\n");
        assert_eq!(view.read(cx).message(), Some("opening secret.txt"));
    })
    .unwrap();
}
