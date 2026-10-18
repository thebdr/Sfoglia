//! Headless tests of the PDF viewer and the tools outside Sfoglia (C-014): a real gpui-kit window opens a
//! PDF through a stand-in renderer into a page column with its zoom, shows the hint and the bytes when no
//! tool is installed until `Look again` finds one, reports a renderer's complaint, and, on Windows, chooses
//! SumatraPDF first and waits for a window to host it in.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::Placement;
use gpui_kit::component::dock::{BasePanel as _, ClosePanel, DockPlacement, InsertTarget, NodeId, PaneNode, PaneRef, PanelId};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext, point, px};
use sfoglia::detect::{Route, detect};
use sfoglia::generated::families::Family;
use sfoglia::pdf_pages::{Backend, PageSize, Renderer};
use sfoglia::tools::{self, Tool, ToolSet};
use sfoglia::viewers::PdfView;
use sfoglia::viewers::pdf_view::{self, candidates};
use sfoglia::workspace::{FileInfo, FilePanel};
use sfoglia::{Workspace, window_options};

fn open_workspace(cx: &mut TestAppContext, tools: ToolSet) -> (AnyWindowHandle, Entity<Workspace>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let rules_path = std::env::temp_dir().join(format!("sfoglia-pdf-rules-{}-{n}.toml", std::process::id()));
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

/// Open `path` in a fresh workspace with `tools` and hand back its panel and PDF view, the page list settled.
fn open_pdf(
    cx: &mut TestAppContext,
    tools: ToolSet,
    path: PathBuf,
) -> (AnyWindowHandle, Entity<Workspace>, Entity<FilePanel>, Entity<PdfView>) {
    let (handle, workspace) = open_workspace(cx, tools);
    let (panel, view) = cx
        .update_window(handle, |_, window, cx| {
            workspace.update(cx, |w, cx| w.open_paths(&[path], window, cx));
            window.render_frame(cx);
            let panel = workspace.read(cx).panels(cx)[0].clone();
            let view = panel.read(cx).pdf_view().expect("the file opens in the PDF viewer").clone();
            (panel, view)
        })
        .unwrap();
    cx.run_until_parked();
    (handle, workspace, panel, view)
}

fn pages(sizes: &[(f32, f32)]) -> Vec<PageSize> {
    sizes.iter().map(|&(width_pt, height_pt)| PageSize { width_pt, height_pt }).collect()
}

#[test]
fn the_pdf_extensions_map_to_their_family() {
    let header: &[u8] = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n1 0 obj\n";
    for name in ["a.pdf", "A.PDF", "a.xps", "a.oxps"] {
        let d = detect(name, header);
        assert_eq!(
            (d.family, d.route, d.route.clause()),
            (Some(Family::PdfXps), Route::Family(Family::PdfXps), "C-014"),
            "{name}"
        );
    }
    // The PDF header line is a definitive signature: a misleading extension still comes here.
    assert_eq!(detect("report.txt", header).family, Some(Family::PdfXps));
    // An XPS is a ZIP inside: the container never overrides its extension.
    assert_eq!(detect("pages.xps", b"PK\x03\x04\x14\x00\x00\x00\x08\x00").family, Some(Family::PdfXps));
    assert_eq!(Family::PdfXps.slug(), "pdf-xps");
    let expected: &[Tool] =
        if cfg!(windows) { &[Tool::SumatraPdf, Tool::MuTool, Tool::PdfToPpm] } else { &[Tool::MuTool, Tool::PdfToPpm] };
    assert_eq!(candidates(), expected);
    assert_eq!(pdf_view::article_for("PDF"), "A");
    assert_eq!(pdf_view::article_for("XPS"), "An");
    assert_eq!(pdf_view::article_for("OXPS"), "An");
}

#[test]
fn the_frame_poll_never_gives_up_while_the_process_lives() {
    use std::time::Duration;
    assert_eq!(pdf_view::poll_wait(0), Duration::from_millis(100));
    assert_eq!(pdf_view::poll_wait(49), Duration::from_millis(100));
    assert_eq!(pdf_view::poll_wait(50), Duration::from_millis(500), "after five seconds, every half second");
    assert_eq!(pdf_view::poll_wait(1_000), Duration::from_millis(500));
    assert_eq!(pdf_view::poll_wait(u32::MAX), Duration::from_millis(500), "there is no last try");
    // The poll itself: a thousand looks that find nothing, then the frame; or the process gone at the seven
    // hundredth look, with its reason.
    let mut looks = 0u32;
    let settled = futures::executor::block_on(pdf_view::poll_frame(|wait| {
        looks += 1;
        let outcome = if looks <= 1_000 { Ok(false) } else { Ok(true) };
        assert_eq!(wait, pdf_view::poll_wait(looks - 1));
        async move { outcome }
    }));
    assert_eq!(settled, Ok(()));
    assert_eq!(looks, 1_001, "every look until the frame, none after");
    let mut looks = 0u32;
    let gone = futures::executor::block_on(pdf_view::poll_frame(|_| {
        looks += 1;
        let outcome = if looks < 700 { Ok(false) } else { Err("ended (exit code: 3)".to_string()) };
        async move { outcome }
    }));
    assert_eq!(gone, Err("ended (exit code: 3)".to_string()));
    assert_eq!(looks, 700, "a process gone ends the poll with its reason");
}

#[gpui_kit::test]
fn a_pdf_opens_through_the_stub_renderer_as_a_page_column_with_zoom(cx: &mut TestAppContext) {
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, _workspace, panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    let fit = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let v = view.read(cx);
            assert_eq!(v.mode_name(), "pages");
            assert!(!v.is_listing(), "the stub listed the pages before the window parked");
            assert_eq!(v.failure(), None);
            assert_eq!(v.page_sizes(), pages(&[(612.0, 792.0), (400.0, 200.0)]));
            assert_eq!(v.tool().map(|(t, f)| (t, f.path.clone())), Some((Tool::MuTool, stub("mutool"))));
            assert_eq!(v.tool_label(), "mutool 9.9");
            assert_eq!(label(window, "pdf-tool"), "mutool 9.9");
            assert_eq!(v.zoom(), None, "fit to the width by default");
            let fit = v.effective_zoom();
            assert!(fit > 0.05 && fit < 16.0, "{fit}");
            // Fit to the width: the widest page (612 pt) fills the column but a 12 px gap each side.
            let column = f32::from(window.find("pdf-scroll").bounds().size.width);
            let expected_fit = (column - 24.0) / (612.0 * 96.0 / 72.0);
            assert!((fit - expected_fit).abs() < 1e-4, "fit {fit} in a {column} px column, expected {expected_fit}");
            let page_width = f32::from(window.find(("pdf-page", 1usize)).bounds().size.width);
            assert!((page_width - (column - 24.0)).abs() < 0.5, "page 1 is {page_width} px wide in {column}");
            let second = f32::from(window.find(("pdf-page", 2usize)).bounds().size.width);
            assert!((second - 400.0 * 96.0 / 72.0 * fit).abs() < 0.5, "page 2 keeps its own width: {second}");
            assert_eq!(v.current_page(), Some(1));
            assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page 1 · {} %", (fit * 100.0).round()));
            assert_eq!(label(window, "pdf-message"), "+ and - zoom, 0 fits the width");
            assert!(window.try_find("pdf-look-again").is_none(), "a tool is in use");
            assert!(window.try_find("pdf-hex").is_none());
            assert!(window.try_find("pdf-scroll").is_some());
            assert!(label(window, ("pdf-page", 1usize)).starts_with("page 1: 612 × 792 pt"));
            assert!(label(window, ("pdf-page", 2usize)).starts_with("page 2: 400 × 200 pt"));
            assert!(panel.read(cx).pdf_view().is_some());
            assert!(!panel.read(cx).is_dirty(cx));
            fit
        })
        .unwrap();
    cx.run_until_parked();
    let scale = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let scale = window.scale_factor();
            let dpi = (fit * 96.0 * scale).round() as u32;
            let v = view.read(cx);
            assert_eq!(v.drawn_pages(), vec![(1, dpi), (2, dpi)], "both pages drawn at the fit zoom's dpi");
            assert_eq!(label(window, ("pdf-page", 1usize)), "page 1: 612 × 792 pt, drawn");
            assert_eq!(label(window, ("pdf-page", 2usize)), "page 2: 400 × 200 pt, drawn");
            let dir = v.scratch_dir().expect("a folder for the drawings");
            assert!(dir.join(format!("page-1-{dpi}.png")).is_file(), "the stub's PNG for page 1");
            assert!(dir.join(format!("page-2-{dpi}.png")).is_file());
            // A quarter in by the key: the summary follows at once, the drawings at the new dpi come later.
            let focus = view.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            window.render_frame(cx);
            window.press("+", cx);
            window.render_frame(cx);
            let v = view.read(cx);
            let zoom = v.zoom().expect("a chosen zoom");
            assert!((zoom - fit * 1.25).abs() < 1e-5, "{zoom} vs {fit}");
            assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page 1 · {} %", (fit * 1.25 * 100.0).round()));
            assert_eq!(v.drawn_pages(), vec![(1, dpi), (2, dpi)], "the old drawings stay until the new ones land");
            scale
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let dpi = (fit * 1.25 * 96.0 * scale).round() as u32;
        assert_eq!(view.read(cx).drawn_pages(), vec![(1, dpi), (2, dpi)]);
        // The keys: `=` is another quarter in, `-` twice brings the fit zoom back, `0` fits again.
        window.press("=", cx);
        window.render_frame(cx);
        let zoom = view.read(cx).zoom().expect("chosen");
        assert!((zoom - fit * 1.25 * 1.25).abs() < 1e-5, "{zoom} vs {fit}");
        window.press("-", cx);
        window.press("-", cx);
        window.render_frame(cx);
        let zoom = view.read(cx).zoom().expect("still chosen");
        assert!((zoom - fit).abs() < 1e-4, "{zoom} vs {fit}");
        window.press("0", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).zoom(), None);
        assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page 1 · {} %", (fit * 100.0).round()));
        // The page at the top follows the scroll. The second page is short, so the column cannot scroll page 1
        // out of view: at the very bottom page 1 is still the page at the top, by the column's own geometry
        // (the eight-page test scrolls far enough for the page to change).
        let page_1_height = 792.0 * 96.0 / 72.0 * fit;
        let page_2_height = 200.0 * 96.0 / 72.0 * fit;
        let viewport = f32::from(window.find("pdf-scroll").bounds().size.height);
        let content = 12.0 + page_1_height + 12.0 + page_2_height + 12.0;
        let scroll = view.read(cx).scroll_handle().unwrap().clone();
        scroll.set_offset(point(px(0.0), px(-(content - viewport).max(0.0))));
        window.render_frame(cx);
        let top = -f32::from(scroll.offset().y);
        let expected = if 12.0 + page_1_height > top + 1.0 { 1 } else { 2 };
        assert_eq!(view.read(cx).current_page(), Some(expected), "top {top}, page 1 ends at {}", 12.0 + page_1_height);
        assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page {expected} · {} %", (fit * 100.0).round()));
        scroll.set_offset(point(px(0.0), px(0.0)));
        window.render_frame(cx);
        assert_eq!(view.read(cx).current_page(), Some(1));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_drawings_folder_goes_with_the_tab(cx: &mut TestAppContext) {
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    let dir = cx.update_window(handle, |_, _, cx| view.read(cx).scratch_dir().unwrap()).unwrap();
    assert!(dir.is_dir());
    // The test holds no handle of its own: the dock and the workspace are the only owners.
    drop(view);
    drop(panel);
    cx.update_window(handle, |_, window, cx| {
        // A second tab, so the first can close; the PDF tab is made active again and closed.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("hello.txt"), fixture("three.pdf")], window, cx));
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("three.pdf".into()));
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let focus = panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.dispatch_action(Box::new(ClosePanel), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["hello.txt"]);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!dir.exists(), "the drawings folder is removed with the view: {}", dir.display());
}

#[gpui_kit::test]
fn without_a_tool_the_hint_names_each_tool_and_the_bytes_show_until_look_again(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-missing");
    let later = dir.join(if cfg!(windows) { "mutool.cmd" } else { "mutool.sh" });
    let tools = ToolSet::stubbed([
        (Tool::SumatraPdf, None),
        (Tool::MuTool, Some(later.clone())),
        (Tool::PdfToPpm, None),
        (Tool::PdfInfo, None),
    ]);
    let (handle, _workspace, panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    let expected = format!("{} The bytes show meanwhile.", tools::missing_message("A PDF", candidates()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.mode_name(), "missing");
        assert_eq!(v.tool().map(|(t, _)| t), None);
        assert_eq!(label(window, "pdf-message"), expected);
        assert_eq!(label(window, "pdf-status"), expected);
        if cfg!(windows) {
            assert_eq!(
                expected,
                "A PDF needs SumatraPDF, mutool or pdftoppm, and none is installed: install SumatraPDF from https://www.sumatrapdfreader.org, or set SFOGLIA_SUMATRAPDF to its path; install mutool from https://mupdf.com (mutool.exe in the MuPDF release), or set SFOGLIA_MUTOOL to its path; install pdftoppm from a poppler build for Windows (https://github.com/oschwartz10612/poppler-windows), or set SFOGLIA_PDFTOPPM to its path. The bytes show meanwhile."
            );
        } else {
            assert_eq!(
                expected,
                "A PDF needs mutool or pdftoppm, and none is installed: install mutool from the package mupdf-tools, or set SFOGLIA_MUTOOL to its path; install pdftoppm from the package poppler-utils, or set SFOGLIA_PDFTOPPM to its path. The bytes show meanwhile."
            );
        }
        assert_eq!(label(window, "pdf-tool"), "no tool");
        assert_eq!(label(window, "pdf-summary"), "PDF");
        assert!(window.try_find("pdf-look-again").is_some());
        assert!(v.hex_view().is_some());
        assert!(window.try_find("pdf-hex").is_some());
        assert!(window.try_find("hex-status").is_some(), "the bytes show under the hint");
        let row = window.find(("hex-row", 0usize)).label().unwrap_or_default().to_string();
        assert!(row.starts_with("00000000  25 50 44 46 2d 31 2e 34"), "%PDF-1.4: {row}");
        assert!(panel.read(cx).pdf_view().is_some(), "the panel still shows the PDF view, which holds the bytes");
        assert!(window.try_find("pdf-scroll").is_none());
        // The hex view's fields are the hex view's: digits and signs typed there are not the page column's zoom keys.
        let goto = v.hex_view().unwrap().read(cx).goto_input().clone();
        let focus = goto.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("0x10-2+0=", cx);
        window.render_frame(cx);
        assert_eq!(goto.read(cx).value().as_ref(), "0x10-2+0=", "every key reached the offset field");
        assert_eq!(view.read(cx).zoom(), None);
    })
    .unwrap();
    // The tool arrives at the pinned path (a wrapper around the stand-in, which finds its fixtures next to
    // itself): Look again finds it, and the pages replace the bytes.
    install_wrapper(&later, &stub("mutool"));
    cx.update_window(handle, |_, window, cx| {
        window.click("pdf-look-again-button", cx);
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.mode_name(), "pages");
        assert!(v.is_listing());
        assert_eq!(v.tool().map(|(t, f)| (t, f.path.clone())), Some((Tool::MuTool, later.clone())));
        assert_eq!(label(window, "pdf-tool"), "mutool 9.9");
        assert_eq!(label(window, "pdf-summary"), "PDF, listing the pages");
        assert!(window.try_find("hex-status").is_none());
        assert!(window.try_find("pdf-look-again").is_none());
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page_sizes(), pages(&[(612.0, 792.0), (400.0, 200.0)]));
        assert!(label(window, "pdf-summary").starts_with("PDF, 2 pages · page 1 · "));
        // The focus was in the hint's offset field: it is the page column's now, so the zoom keys work at once.
        assert!(view.read(cx).focus_handle(cx).is_focused(window), "the view holds the focus after Look again");
        let fit = view.read(cx).effective_zoom();
        window.press("+", cx);
        window.render_frame(cx);
        let zoom = view.read(cx).zoom().expect("the key reached the page column");
        assert!((zoom - fit * 1.25).abs() < 1e-5, "{zoom} vs {fit}");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_renderer_that_fails_reports_its_complaint(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-failures");
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool-broken"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, _panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    let expected_failure = format!("mutool-broken: error: cannot open document");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.mode_name(), "pages");
        assert_eq!(v.tool_label(), "mutool 9.9", "the broken stub still reports a version");
        assert_eq!(v.failure(), Some(expected_failure.as_str()));
        assert_eq!(v.page_sizes(), vec![]);
        assert_eq!(label(window, "pdf-summary"), format!("PDF, {expected_failure}"));
        assert!(window.try_find("pdf-look-again").is_some(), "a listing that failed offers to look again");
        assert_eq!(view.read(cx).draw_failure(), None);
        assert!(window.try_find("hex-status").is_none());
    })
    .unwrap();

    // Other tool sets, through panels built directly: pdftoppm without pdfinfo, pdftoppm on an XPS, nothing on an OXPS.
    // An XPS and an OXPS are ZIP files: a PDF copied under those names would be a PDF by its header.
    let xps = dir.join("pages.xps");
    std::fs::write(&xps, b"PK\x03\x04\x14\x00\x00\x00\x08\x00not an xps inside").unwrap();
    let oxps = dir.join("pages.oxps");
    std::fs::write(&oxps, b"PK\x03\x04\x14\x00\x00\x00\x08\x00not an oxps inside").unwrap();
    let without_info = Arc::new(ToolSet::stubbed([
        (Tool::SumatraPdf, None),
        (Tool::MuTool, None),
        (Tool::PdfToPpm, Some(stub("mutool"))),
        (Tool::PdfInfo, None),
    ]));
    let poppler = Arc::new(ToolSet::stubbed([
        (Tool::SumatraPdf, None),
        (Tool::MuTool, None),
        (Tool::PdfToPpm, Some(stub("mutool"))),
        (Tool::PdfInfo, Some(stub("mutool"))),
    ]));
    let nothing = Arc::new(ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, None), (Tool::PdfToPpm, None)]));
    let (no_info, on_xps, on_oxps) = cx
        .update_window(handle, |_, window, cx| {
            let rules = workspace.read(cx).rules().clone();
            let no_info = cx.new(|cx| FilePanel::new(FileInfo::probe(&fixture("three.pdf")), &rules, &without_info, window, cx));
            let on_xps = cx.new(|cx| FilePanel::new(FileInfo::probe(&xps), &rules, &poppler, window, cx));
            let on_oxps = cx.new(|cx| FilePanel::new(FileInfo::probe(&oxps), &rules, &nothing, window, cx));
            let view = no_info.read(cx).pdf_view().unwrap().read(cx);
            assert_eq!(view.mode_name(), "pages");
            assert_eq!(view.tool_label(), "pdftoppm 9.9", "the stub stands in for pdftoppm here, with its banner's version");
            let view = on_xps.read(cx).pdf_view().unwrap().read(cx);
            assert_eq!(view.kind(), "XPS");
            assert_eq!(view.tool_label(), "pdftoppm 9.9");
            assert_eq!(view.message(), format!("pdftoppm reads PDF alone: for XPS {}", Tool::MuTool.install_hint()));
            assert!(view.message().contains("SFOGLIA_MUTOOL"));
            let view = on_oxps.read(cx).pdf_view().unwrap().read(cx);
            assert_eq!(view.mode_name(), "missing");
            assert_eq!(view.kind(), "OXPS");
            assert_eq!(view.summary(), "OXPS");
            assert!(view.message().starts_with("An OXPS needs "), "{}", view.message());
            assert!(view.hex_view().is_some());
            (no_info, on_xps, on_oxps)
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, _, cx| {
        // Without pdfinfo nothing lists the pages and nothing is run: the hint is the complaint.
        let view = no_info.read(cx).pdf_view().unwrap().read(cx);
        let hint = format!("pdftoppm needs pdfinfo from the same package to list the pages: {}", Tool::PdfInfo.install_hint());
        assert_eq!(view.failure(), Some(hint.as_str()));
        assert_eq!(view.summary(), format!("PDF, {hint}"));
        assert!(view.wants_look_again());
        assert!(!view.scratch_dir().unwrap().join("calls.log").exists(), "the stub was never run");
        // With a pdfinfo that knows no `-f` (the mutool stub), the complaint names the stub as run.
        let view = on_xps.read(cx).pdf_view().unwrap().read(cx);
        assert_eq!(view.failure(), Some("mutool: unknown command -f"));
        assert_eq!(on_oxps.read(cx).pdf_view().unwrap().read(cx).failure(), None);
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pages the stand-in drew, from the log it keeps beside the drawings.
fn draw_calls(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default().lines().map(|l| l.trim().to_string()).collect()
}

/// The pages (from 1) whose box meets a viewport of `height` px scrolled to `top` px, in a column of
/// `count` pages each `page_height` px tall with the 12 px gaps.
fn pages_in_view(count: usize, page_height: f32, top: f32, height: f32) -> Vec<usize> {
    let mut seen = Vec::new();
    let mut y = 12.0f32;
    for page in 1..=count {
        if y + page_height >= top && y <= top + height {
            seen.push(page);
        }
        y += page_height + 12.0;
    }
    seen
}

#[gpui_kit::test]
fn drawings_follow_the_scroll_around_the_visible_pages(cx: &mut TestAppContext) {
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool-many"))), (Tool::PdfToPpm, None)]);
    let (handle, _workspace, _panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    let (dir, page_height, height, at_top) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let v = view.read(cx);
            assert_eq!(v.page_sizes().len(), 8);
            let fit = v.effective_zoom();
            let page_height = 792.0 * 96.0 / 72.0 * fit;
            let height = f32::from(window.find("pdf-scroll").bounds().size.height);
            let visible = pages_in_view(8, page_height, 0.0, height);
            let last = *visible.last().unwrap();
            assert!(last < 7, "the viewport ({height} px) must not reach the last pages ({page_height} px each) for this test to mean anything");
            let expected: Vec<usize> = (1..=last + 1).collect();
            assert_eq!(v.drawn_pages().iter().map(|(p, _)| *p).collect::<Vec<_>>(), expected, "the visible pages and the next one");
            let dir = v.scratch_dir().unwrap();
            assert_eq!(draw_calls(&dir).len(), expected.len(), "one drawing each, nothing beyond: {:?}", draw_calls(&dir));
            (dir, page_height, height, expected)
        })
        .unwrap();
    // Scrolled to the end: the last pages and the one before them are drawn too; the first ones stay. The
    // scroll lands during the next layout, and the frame after that asks for the drawings.
    cx.update_window(handle, |_, window, cx| {
        view.read(cx).scroll_handle().unwrap().scroll_to_bottom();
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        let total = 12.0 + 8.0 * (page_height + 12.0);
        let bottom = pages_in_view(8, page_height, total - height, height);
        let first = *bottom.first().unwrap();
        let mut expected = at_top.clone();
        expected.extend((first - 1)..=8);
        expected.sort();
        expected.dedup();
        assert_eq!(v.drawn_pages().iter().map(|(p, _)| *p).collect::<Vec<_>>(), expected, "scrolled to the end");
        assert_eq!(v.current_page(), Some(first));
        assert_eq!(label(window, "pdf-summary"), format!("PDF, 8 pages · page {first} · {} %", (v.effective_zoom() * 100.0).round()));
        assert_eq!(draw_calls(&dir).len(), expected.len());
        assert_eq!(v.draw_failure(), None);
        assert!(!v.wants_look_again());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_page_that_fails_to_draw_is_asked_again_only_after_a_zoom_change(cx: &mut TestAppContext) {
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool-page2"))), (Tool::PdfToPpm, None)]);
    let (handle, _workspace, _panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    let complaint = "mutool-page2: error: cannot draw page 2";
    let (dir, fit) = cx
        .update_window(handle, |_, window, cx| {
            // A few more frames: the failed page is not asked for again.
            for _ in 0..3 {
                window.render_frame(cx);
            }
            let v = view.read(cx);
            let fit = v.effective_zoom();
            let dpi = (fit * 96.0 * window.scale_factor()).round() as u32;
            assert_eq!(v.page_sizes().len(), 2);
            assert_eq!(v.drawn_pages(), vec![(1, dpi)], "page 1 drawn, page 2 not");
            assert_eq!(v.failure(), None, "the listing itself went fine");
            assert_eq!(v.draw_failure(), Some(complaint));
            assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page 1 · {} % · {complaint}", (fit * 100.0).round()));
            assert!(window.try_find("pdf-look-again").is_none(), "the tool is there and listed the pages");
            assert_eq!(label(window, ("pdf-page", 2usize)), "page 2: 612 × 792 pt");
            let dir = v.scratch_dir().unwrap();
            (dir, fit)
        })
        .unwrap();
    cx.run_until_parked();
    let calls = draw_calls(&dir);
    assert_eq!(calls.iter().filter(|c| c.starts_with("draw 2 ")).count(), 1, "one try for page 2: {calls:?}");
    assert_eq!(calls.len(), 2);
    // A zoom change asks again at the new dpi; the complaint stays until the page draws (the stand-in fails
    // page 2 the first time alone), then goes, and the summary is whole again.
    cx.update_window(handle, |_, window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("+", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).draw_failure(), Some(complaint), "the complaint stays until the page draws");
        assert!(label(window, "pdf-summary").ends_with(&format!(" · {complaint}")));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        let dpi = (fit * 1.25 * 96.0 * window.scale_factor()).round() as u32;
        assert_eq!(v.drawn_pages(), vec![(1, dpi), (2, dpi)], "the second try drew page 2");
        assert_eq!(v.draw_failure(), None, "gone when the page draws");
        assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page 1 · {} %", (fit * 1.25 * 100.0).round()));
    })
    .unwrap();
    let calls = draw_calls(&dir);
    assert_eq!(calls.iter().filter(|c| c.starts_with("draw 2 ")).count(), 2, "page 2 tried once per zoom: {calls:?}");
    assert_eq!(calls.len(), 4);
}

#[gpui_kit::test]
fn pdftoppm_lists_pages_through_pdfinfo_and_look_again_finds_it_later(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-poppler");
    let later = dir.join(if cfg!(windows) { "pdfinfo.cmd" } else { "pdfinfo.sh" });
    let tools = ToolSet::stubbed([
        (Tool::SumatraPdf, None),
        (Tool::MuTool, None),
        (Tool::PdfToPpm, Some(stub("pdftoppm"))),
        (Tool::PdfInfo, Some(later.clone())),
    ]);
    let (handle, workspace, _panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    let hint = format!("pdftoppm needs pdfinfo from the same package to list the pages: {}", Tool::PdfInfo.install_hint());
    let drawings = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let v = view.read(cx);
            assert_eq!(v.mode_name(), "pages");
            assert_eq!(v.tool().map(|(t, f)| (t, f.path.clone())), Some((Tool::PdfToPpm, stub("pdftoppm"))));
            assert_eq!(v.tool_label(), "pdftoppm 24.02.0", "the version from the banner on stderr");
            assert_eq!(v.failure(), Some(hint.as_str()));
            assert_eq!(label(window, "pdf-summary"), format!("PDF, {hint}"));
            assert!(window.try_find("pdf-look-again").is_some());
            assert_eq!(v.page_sizes(), vec![]);
            let drawings = v.scratch_dir().unwrap();
            assert!(!drawings.join("calls.log").exists(), "pdftoppm was not run for a listing it cannot do");
            drawings
        })
        .unwrap();
    // pdfinfo arrives at its pinned path: Look again lists the pages, pdftoppm draws them. The focus sits on
    // the workspace itself meanwhile, and stays there: Look again moves no focus from outside the view.
    install_wrapper(&later, &stub("pdfinfo"));
    let elsewhere = cx.update_window(handle, |_, window, cx| {
        let elsewhere = workspace.read(cx).own_focus_handle();
        window.focus(&elsewhere, cx);
        window.render_frame(cx);
        assert!(elsewhere.is_focused(window));
        window.click("pdf-look-again-button", cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_listing());
        assert!(elsewhere.is_focused(window), "a focus outside the view is left alone");
        assert!(!view.read(cx).focus_handle(cx).is_focused(window));
        elsewhere
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(elsewhere.is_focused(window), "still there once the pages are listed");
        let v = view.read(cx);
        assert_eq!(v.page_sizes(), pages(&[(612.0, 792.0), (792.0, 612.0)]), "the second page's quarter turn swaps its sides");
        assert_eq!(v.failure(), None);
        assert!(window.try_find("pdf-look-again").is_none());
        assert!(label(window, "pdf-summary").starts_with("PDF, 2 pages · page 1 · "));
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        let dpi = (v.effective_zoom() * 96.0 * window.scale_factor()).round() as u32;
        assert_eq!(v.drawn_pages(), vec![(1, dpi), (2, dpi)]);
        let drawings = v.scratch_dir().unwrap();
        assert_eq!(drawings, drawings.clone());
        let calls = draw_calls(&drawings);
        assert_eq!(calls, vec![format!("draw 1 at {dpi}"), format!("draw 2 at {dpi}")].into_iter().filter(|c| calls.contains(c)).collect::<Vec<_>>(), "{calls:?}");
        assert_eq!(calls.len(), 2);
        assert!(drawings.join(format!("page-2-{dpi}.png")).is_file(), "pdftoppm's -singlefile output under the prefix");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = drawings;
}

#[gpui_kit::test]
fn a_pdf_under_another_name_is_still_a_pdf(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-named");
    let report = dir.join("report.txt");
    std::fs::copy(fixture("three.pdf"), &report).unwrap();
    let tools = ToolSet::stubbed([
        (Tool::SumatraPdf, None),
        (Tool::MuTool, None),
        (Tool::PdfToPpm, Some(stub("pdftoppm"))),
        (Tool::PdfInfo, Some(stub("pdfinfo"))),
    ]);
    let (handle, workspace, _panel, view) = open_pdf(cx, tools, report.clone());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.kind(), "PDF", "the header says PDF, whatever the name");
        assert_eq!(v.failure(), None);
        assert!(label(window, "pdf-summary").starts_with("PDF, 2 pages · page 1 · "), "{}", label(window, "pdf-summary"));
        assert_eq!(label(window, "pdf-message"), "+ and - zoom, 0 fits the width", "pdftoppm reads it: it is a PDF");
    })
    .unwrap();
    // Even under an XPS name a PDF header makes a PDF, and pdftoppm reads it without complaint.
    let doc = dir.join("doc.xps");
    std::fs::copy(fixture("three.pdf"), &doc).unwrap();
    let poppler = Arc::new(ToolSet::stubbed([
        (Tool::SumatraPdf, None),
        (Tool::MuTool, None),
        (Tool::PdfToPpm, Some(stub("pdftoppm"))),
        (Tool::PdfInfo, Some(stub("pdfinfo"))),
    ]));
    cx.update_window(handle, |_, window, cx| {
        let rules = workspace.read(cx).rules().clone();
        let named = cx.new(|cx| FilePanel::new(FileInfo::probe(&doc), &rules, &poppler, window, cx));
        let view = named.read(cx).pdf_view().unwrap().read(cx);
        assert_eq!(view.kind(), "PDF", "a PDF header under an XPS name");
        assert_eq!(view.message(), "+ and - zoom, 0 fits the width");
        assert_eq!(view.summary(), "PDF, listing the pages");
    })
    .unwrap();
    // With nothing installed the hint names a PDF too; an XPS by name with a ZIP inside stays an XPS.
    let xps = dir.join("pages.xps");
    std::fs::write(&xps, b"PK\x03\x04\x14\x00\x00\x00\x08\x00not really an xps").unwrap();
    let nothing = Arc::new(ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, None), (Tool::PdfToPpm, None)]));
    cx.update_window(handle, |_, window, cx| {
        let rules = workspace.read(cx).rules().clone();
        let named = cx.new(|cx| FilePanel::new(FileInfo::probe(&report), &rules, &nothing, window, cx));
        let view = named.read(cx).pdf_view().unwrap().read(cx);
        assert_eq!(view.kind(), "PDF");
        assert!(view.message().starts_with("A PDF needs "), "{}", view.message());
        let zipped = cx.new(|cx| FilePanel::new(FileInfo::probe(&xps), &rules, &nothing, window, cx));
        let view = zipped.read(cx).pdf_view().unwrap().read(cx);
        assert_eq!(view.kind(), "XPS");
        assert_eq!(view.summary(), "XPS");
        assert!(view.message().starts_with("An XPS needs "), "{}", view.message());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn a_closed_tab_leaves_no_drawings_even_mid_flight(cx: &mut TestAppContext) {
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    let dir = cx.update_window(handle, |_, _, cx| view.read(cx).scratch_dir().unwrap()).unwrap();
    assert!(dir.is_dir());
    let weak = view.downgrade();
    drop(view);
    drop(panel);
    cx.update_window(handle, |_, window, cx| {
        // A second tab so the first can close; then, with the PDF tab active, a zoom change asks for new
        // drawings, and the tab closes while they are in flight.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("hello.txt"), fixture("three.pdf")], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[0].clone();
        let focus = panel.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("+", cx);
        window.render_frame(cx);
        assert_eq!(weak.upgrade().map(|v| v.read(cx).drawn_pages().len()), Some(2), "the old drawings stay while the new ones run");
        window.dispatch_action(Box::new(ClosePanel), cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["hello.txt"]);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| weak.upgrade().map(|v| v.read(cx).kind())), None, "the closed tab's view is dropped");
    assert!(!dir.exists(), "the drawings folder is gone even though drawings were in flight: {}", dir.display());
}

#[gpui_kit::test]
fn a_tab_behind_another_tells_its_view_so_a_hosted_window_hides(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-behind");
    let second = dir.join("second.pdf");
    std::fs::copy(fixture("three.pdf"), &second).unwrap();
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, first_panel, first) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(first.read(cx).shown());
        assert!(first_panel.read(cx).shown_by_dock());
        // A second PDF opens in front: the first tab's view hears it went behind, at the next render.
        workspace.update(cx, |w, cx| w.open_paths(&[second.clone()], window, cx));
        window.render_frame(cx);
        let second_panel = workspace.read(cx).panels(cx)[1].clone();
        let second_view = second_panel.read(cx).pdf_view().unwrap().clone();
        assert_eq!(workspace.read(cx).visible_panels(cx).len(), 1, "one tab group, one visible tab");
        assert!(!first_panel.read(cx).shown_by_dock());
        assert!(!first.read(cx).shown(), "the tab behind tells its view");
        assert!(second_panel.read(cx).shown_by_dock());
        assert!(second_view.read(cx).shown());
        // Back to the first: it shows, the second hides.
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("three.pdf")], window, cx));
        window.render_frame(cx);
        assert!(first.read(cx).shown());
        assert!(!second_view.read(cx).shown());
        // The hex view on the shown tab hides its view too; off again, it shows.
        first_panel.update(cx, |p, cx| p.toggle_hex(window, cx));
        window.render_frame(cx);
        assert!(!first.read(cx).shown());
        assert!(first_panel.read(cx).shown_by_dock(), "the dock still shows the tab, the bytes are in front");
        first_panel.update(cx, |p, cx| p.toggle_hex(window, cx));
        window.render_frame(cx);
        assert!(first.read(cx).shown());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_drawing_landing_after_its_pages_are_gone_removes_the_folder() {
    use std::sync::atomic::AtomicBool;
    let renderer = Renderer { backend: Backend::MuTool, exe: stub("mutool"), info_exe: None };
    let dir = scratch_dir("pdf-landing");
    // Alive: the drawing is decoded and the folder stays with its PNG.
    let alive = AtomicBool::new(true);
    let picture = pdf_view::draw_page(&renderer, &fixture("three.pdf"), 1, 96, &dir, &alive).expect("gradient.png decodes");
    assert_eq!((picture.width, picture.height), (320, 200), "the stand-in's gradient fixture");
    assert!(dir.join("page-1-96.png").is_file());
    // Gone meanwhile: the drawing still lands, and the folder it wrote into is removed.
    alive.store(false, std::sync::atomic::Ordering::SeqCst);
    let picture = pdf_view::draw_page(&renderer, &fixture("three.pdf"), 2, 96, &dir, &alive).expect("dot.png decodes");
    assert_eq!((picture.width, picture.height), (1, 1));
    assert!(!dir.exists(), "the folder is gone: {}", dir.display());
    // A failed drawing with the pages gone removes the folder too.
    let broken = Renderer { backend: Backend::MuTool, exe: stub("mutool-broken"), info_exe: None };
    std::fs::create_dir_all(&dir).unwrap();
    let failed = pdf_view::draw_page(&broken, &fixture("three.pdf"), 1, 96, &dir, &alive);
    assert_eq!(failed.err().as_deref(), Some("mutool-broken: error: cannot open document"));
    assert!(!dir.exists());
}

#[gpui_kit::test]
fn a_complaint_stays_until_its_own_page_draws(cx: &mut TestAppContext) {
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool-many2"))), (Tool::PdfToPpm, None)]);
    let (handle, _workspace, _panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| window.render_frame(cx)).unwrap();
    cx.run_until_parked();
    let complaint = "mutool-many2: error: cannot draw page 2";
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.page_sizes().len(), 8);
        assert_eq!(v.draw_failure(), Some(complaint));
        assert!(v.drawn_pages().iter().any(|(p, _)| *p == 1));
        assert!(!v.drawn_pages().iter().any(|(p, _)| *p == 2));
        // Scrolled to the end: other pages draw, the complaint about page 2 stays.
        v.scroll_handle().unwrap().scroll_to_bottom();
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert!(v.drawn_pages().iter().any(|(p, _)| *p == 8), "the last page drew");
        assert_eq!(v.draw_failure(), Some(complaint), "another page drawing does not clear it");
        assert!(label(window, "pdf-summary").ends_with(&format!(" · {complaint}")));
        // A zoom change with page 2 out of view: still not drawn, still the complaint.
        let focus = v.focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("+", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert!(v.zoom().is_some());
        assert!(!v.drawn_pages().iter().any(|(p, _)| *p == 2));
        assert_eq!(v.draw_failure(), Some(complaint), "the complaint is about page 2, which never drew");
        assert!(label(window, "pdf-summary").ends_with(&format!(" · {complaint}")));
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_hosted_window_hides_behind_the_closing_dialog_and_a_tab_closes_with_ctrl_w(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-dialog");
    let hello = dir.join("hello.txt");
    std::fs::copy(fixture("hello.txt"), &hello).unwrap();
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, _panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    // A modified text tab behind, so the window asks before closing; the PDF tab in front.
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[hello.clone()], window, cx));
        window.render_frame(cx);
        let text = workspace.read(cx).panels(cx)[1].read(cx).text_view().expect("a text tab").clone();
        let focus = text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Q", cx);
        window.render_frame(cx);
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("three.pdf")], window, cx));
        window.render_frame(cx);
        assert!(view.read(cx).shown(), "the PDF tab is in front");
    })
    .unwrap();
    cx.run_until_parked();
    let mut vcx = VisualTestContext::from_window(handle, cx);
    assert!(!vcx.simulate_close(), "a modified tab holds the window: the dialog opens");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        std::thread::sleep(std::time::Duration::from_millis(300));
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_some(), "the closing dialog shows");
        assert!(!view.read(cx).shown(), "a hosted window would cover the dialog: it hides while the dialog is up");
        window.click("close-cancel", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
        assert!(view.read(cx).shown(), "shown again once the dialog is gone");
        // Ctrl+W closes the active tab, the one the hosted window belongs to, with the focus wherever the
        // cancelled dialog left it.
        assert!(view.read(cx).focus_handle(cx).is_focused(window), "the dialog left the focus nowhere; the active tab took it back");
        window.press("ctrl-w", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let names: Vec<String> = workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["hello.txt"], "the PDF tab closed with Ctrl+W");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn ctrl_w_closes_the_active_tab_wherever_the_focus_sits(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-ctrl-w");
    let second = dir.join("second.pdf");
    std::fs::copy(fixture("three.pdf"), &second).unwrap();
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, _panel, first) = open_pdf(cx, tools, fixture("three.pdf"));
    let names = |workspace: &Entity<Workspace>, cx: &gpui_kit::App| -> Vec<String> {
        workspace.read(cx).open_files(cx).into_iter().map(|f| f.name).collect()
    };
    cx.update_window(handle, |_, window, cx| {
        // A second PDF, then a text file in front: opening a text tab moves no focus, so the focus stays on the
        // second PDF's view, a tab no longer drawn.
        workspace.update(cx, |w, cx| w.open_paths(&[second.clone()], window, cx));
        window.render_frame(cx);
        workspace.update(cx, |w, cx| w.open_paths(&[fixture("hello.txt")], window, cx));
        window.render_frame(cx);
        assert_eq!(names(&workspace, cx), ["three.pdf", "second.pdf", "hello.txt"]);
        assert_eq!(workspace.read(cx).active_file(cx).map(|f| f.name), Some("hello.txt".to_string()));
        // The second PDF's view had the focus; the workspace moved it to the tab in front, so the chord lands.
        let hidden = workspace.read(cx).panels(cx)[1].read(cx).pdf_view().unwrap().clone();
        assert!(!hidden.read(cx).focus_handle(cx).is_focused(window), "a focus on a tab behind is moved along");
        let front = workspace.read(cx).panels(cx)[2].read(cx).focus_handle(cx);
        assert!(front.contains_focused(window, cx), "the tab in front holds it");
        window.press("ctrl-w", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&workspace, cx), ["three.pdf", "second.pdf"], "the active tab closed, not the focused one behind");
        window.press("ctrl-w", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&workspace, cx), ["three.pdf"], "and the next active tab");
        assert!(first.read(cx).shown());
        // The last tab of the only group stays, as the dock has it.
        window.press("ctrl-w", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&workspace, cx), ["three.pdf"]);
    })
    .unwrap();
    // A modified tab stays: Ctrl+W throws no edit away, and opens no dialog either.
    let draft = dir.join("draft.txt");
    std::fs::copy(fixture("hello.txt"), &draft).unwrap();
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[draft.clone()], window, cx));
        window.render_frame(cx);
        let panel = workspace.read(cx).panels(cx)[1].clone();
        let text = panel.read(cx).text_view().expect("a text tab").clone();
        let focus = text.read(cx).editor().read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.input("Q", cx);
        window.render_frame(cx);
    })
    .unwrap();
    // The editor's change reaches the tab after the update: the edit counts once the window has parked.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(workspace.read(cx).panels(cx)[1].read(cx).is_dirty(cx), "the tab is modified");
        window.press("ctrl-w", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(names(&workspace, cx), ["three.pdf", "draft.txt"], "a modified tab stays open");
        assert!(workspace.read(cx).panels(cx)[1].read(cx).is_dirty(cx), "its edit is kept");
        assert!(window.try_find("dialog").is_none(), "and no dialog asks");
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[gpui_kit::test]
fn each_tab_group_shows_its_own_active_tab(cx: &mut TestAppContext) {
    let dir = scratch_dir("pdf-split");
    let second = dir.join("second.pdf");
    std::fs::copy(fixture("three.pdf"), &second).unwrap();
    let tools = ToolSet::stubbed([(Tool::SumatraPdf, None), (Tool::MuTool, Some(stub("mutool"))), (Tool::PdfToPpm, None)]);
    let (handle, workspace, _first_panel, first) = open_pdf(cx, tools, fixture("three.pdf"));
    fn first_tabs(node: &PaneNode) -> Option<NodeId> {
        match node.kind() {
            PaneRef::Tabs { .. } => Some(node.id()),
            PaneRef::Split { children, .. } => children.iter().find_map(first_tabs),
        }
    }
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.open_paths(&[second.clone()], window, cx));
        window.render_frame(cx);
        let panels = workspace.read(cx).panels(cx);
        let second_view = panels[1].read(cx).pdf_view().unwrap().clone();
        assert_eq!(workspace.read(cx).visible_panels(cx).len(), 1);
        assert!(!first.read(cx).shown());
        // The second tab split off to the right: two groups, each showing its tab.
        let dock = workspace.read(cx).dock_area().clone();
        let node = first_tabs(dock.read(cx).layout(DockPlacement::Center).expect("the centre layout").root()).expect("a tab group");
        dock.update(cx, |dock, cx| {
            dock.move_panel(
                PanelId::from(panels[1].entity_id()),
                InsertTarget::Split { node, placement: Placement::Right, size: None },
                window,
                cx,
            )
        });
        window.render_frame(cx);
        let visible = workspace.read(cx).visible_panels(cx);
        assert_eq!(visible.len(), 2, "one visible tab per group");
        assert!(visible.contains(&PanelId::from(panels[0].entity_id())));
        assert!(visible.contains(&PanelId::from(panels[1].entity_id())));
        assert!(first.read(cx).shown(), "the first tab is its group's only tab again");
        assert!(second_view.read(cx).shown());
    })
    .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[gpui_kit::test]
fn the_sumatra_host_is_chosen_first_on_windows_and_waits_for_a_window(cx: &mut TestAppContext) {
    use sfoglia::viewers::pdf_view::HostState;
    let tools = ToolSet::stubbed([
        (Tool::SumatraPdf, Some(stub("sumatra"))),
        (Tool::MuTool, Some(stub("mutool"))),
        (Tool::PdfToPpm, None),
    ]);
    let (handle, workspace, panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.mode_name(), "host", "SumatraPDF comes before mutool");
        assert_eq!(v.host_state(), HostState::NoWindow, "a headless window has no handle to host in");
        assert_eq!(v.tool().map(|(t, f)| (t, f.path.clone())), Some((Tool::SumatraPdf, stub("sumatra"))));
        assert_eq!(v.tool_label(), format!("SumatraPDF at {}", stub("sumatra").display()), "a batch file has no version resource");
        assert_eq!(label(window, "pdf-summary"), "PDF");
        assert_eq!(label(window, "pdf-message"), "the page, find, zoom, rotation and view controls are SumatraPDF's own");
        assert_eq!(label(window, "pdf-host"), "SumatraPDF needs a window to show the document in");
        assert!(window.try_find("pdf-look-again").is_none());
        assert!(window.try_find("pdf-scroll").is_none());
        assert_eq!(v.page_sizes(), vec![]);
        assert_eq!(v.zoom(), None);
        // The hex view replaces the host and hides it; the host returns with the toggle.
        panel.update(cx, |p, cx| p.toggle_hex(window, cx));
        window.render_frame(cx);
        assert!(panel.read(cx).pdf_view().is_none());
        assert!(panel.read(cx).hex_view().is_some());
        assert!(window.try_find("pdf-host").is_none());
        panel.update(cx, |p, cx| p.toggle_hex(window, cx));
        window.render_frame(cx);
        assert!(panel.read(cx).pdf_view().is_some());
        assert!(window.try_find("pdf-host").is_some());
        // The dock's active hook reaches the view without a host to show or hide.
        panel.update(cx, |p, cx| {
            p.set_active(false, window, cx);
            p.set_active(true, window, cx);
        });
        assert_eq!(view.read(cx).host_state(), HostState::NoWindow);
        // A SumatraPDF that ends before it shows a window (what the frame poll reports in the real window) counts
        // as absent: mutool serves next, the reason stays in the status row.
        let reason = "SumatraPDF ended before it showed a window (exit code: 1).".to_string();
        view.update(cx, |v, cx| v.host_failed(reason.clone(), window, cx));
        window.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.host_state(), HostState::Failed);
        assert_eq!(v.mode_name(), "pages");
        assert_eq!(v.tool().map(|(t, f)| (t, f.path.clone())), Some((Tool::MuTool, stub("mutool"))));
        assert_eq!(label(window, "pdf-tool"), "mutool 9.9");
        assert_eq!(label(window, "pdf-message"), format!("{reason} + and - zoom, 0 fits the width"), "the reason joins the column's own text");
        assert_eq!(label(window, "pdf-summary"), "PDF, listing the pages");
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).page_sizes().len(), 2, "the pages listed through mutool");
        assert!(label(window, "pdf-summary").starts_with("PDF, 2 pages · page 1 · "));
        let reason = "SumatraPDF ended before it showed a window (exit code: 1).";
        assert_eq!(label(window, "pdf-message"), format!("{reason} + and - zoom, 0 fits the width"), "the reason joins the column's own text");
    })
    .unwrap();
    // With both renderers there, mutool serves first; with none, the hint joins the reason, no tool is named,
    // the bytes show and Look again is offered.
    let both = Arc::new(ToolSet::stubbed([
        (Tool::SumatraPdf, Some(stub("sumatra"))),
        (Tool::MuTool, Some(stub("mutool"))),
        (Tool::PdfToPpm, Some(stub("pdftoppm"))),
        (Tool::PdfInfo, Some(stub("pdfinfo"))),
    ]));
    let none = Arc::new(ToolSet::stubbed([(Tool::SumatraPdf, Some(stub("sumatra"))), (Tool::MuTool, None), (Tool::PdfToPpm, None)]));
    cx.update_window(handle, |_, window, cx| {
        let rules = workspace.read(cx).rules().clone();
        let reason = "SumatraPDF could not start: a made-up reason.".to_string();
        let with_both = cx.new(|cx| FilePanel::new(FileInfo::probe(&fixture("three.pdf")), &rules, &both, window, cx));
        let view = with_both.read(cx).pdf_view().unwrap().clone();
        view.update(cx, |v, cx| v.host_failed(reason.clone(), window, cx));
        assert_eq!(view.read(cx).tool().map(|(t, _)| t), Some(Tool::MuTool), "mutool before pdftoppm, as among the candidates");
        let with_none = cx.new(|cx| FilePanel::new(FileInfo::probe(&fixture("three.pdf")), &rules, &none, window, cx));
        let view = with_none.read(cx).pdf_view().unwrap().clone();
        view.update(cx, |v, cx| v.host_failed(reason.clone(), window, cx));
        let v = view.read(cx);
        assert_eq!(v.mode_name(), "missing");
        assert_eq!(v.tool().map(|(t, _)| t), None, "a SumatraPDF that counts as absent is not named as the tool");
        assert_eq!(v.tool_label(), "no tool");
        assert_eq!(v.message(), format!("{reason} {} The bytes show meanwhile.", tools::missing_message("A PDF", candidates())));
        assert!(v.hex_view().is_some());
        assert!(v.wants_look_again());
        // Look again asks afresh: SumatraPDF (the stand-in) is found again and hosted, and the old reason is
        // gone from the status row.
        view.update(cx, |v, cx| v.look_again(window, cx));
        let v = view.read(cx);
        assert_eq!(v.mode_name(), "host");
        assert_eq!(v.host_state(), HostState::NoWindow);
        assert_eq!(v.tool().map(|(t, _)| t), Some(Tool::SumatraPdf));
        assert_eq!(v.message(), "the page, find, zoom, rotation and view controls are SumatraPDF's own", "the failed attempt's reason does not linger");
        assert!(!v.wants_look_again());
    })
    .unwrap();
}

#[cfg(windows)]
#[test]
fn the_installed_sumatra_is_found_with_its_version() {
    let installed = std::env::var_os("ProgramFiles")
        .map(|p| PathBuf::from(p).join("SumatraPDF").join("SumatraPDF.exe"))
        .filter(|p| p.is_file());
    let set = ToolSet::from_env();
    let found = set.find(Tool::SumatraPdf);
    match installed {
        Some(exe) => {
            let found = found.expect("SumatraPDF is installed under Program Files");
            assert_eq!(found.path.to_string_lossy().to_lowercase(), exe.to_string_lossy().to_lowercase());
            let version = found.version.clone().expect("the version resource");
            assert!(version.starts_with("3."), "{version}");
            assert_eq!(version.split('.').count(), 2, "3.6, not 3.6.0.0: {version}");
            assert_eq!(set.status_line(Tool::SumatraPdf), format!("SumatraPDF {version}"));
            assert_eq!(tools::version_of(Tool::SumatraPdf, &exe), Some(version));
        }
        None => assert!(found.is_none() || found.as_ref().is_some_and(|f| f.path.is_file()), "{found:?}"),
    }
}
