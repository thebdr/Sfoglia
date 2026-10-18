//! Headless tests of the PDF viewer and the tools outside Sfoglia (C-014): a real gpui-kit window opens a
//! PDF through a stand-in renderer into a page column with its zoom, shows the hint and the bytes when no
//! tool is installed until `Look again` finds one, reports a renderer's complaint, and, on Windows, chooses
//! SumatraPDF first and waits for a window to host it in.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::component::dock::{BasePanel as _, ClosePanel};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, Focusable as _, TestAppContext};
use sfoglia::detect::{Route, detect};
use sfoglia::generated::families::Family;
use sfoglia::pdf_pages::PageSize;
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
            // A quarter in: the summary follows at once, the drawings at the new dpi come later.
            view.update(cx, |v, cx| v.zoom_in(cx));
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
        // The keys: a quarter out again, then fit.
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.press("-", cx);
        window.render_frame(cx);
        let zoom = view.read(cx).zoom().expect("still chosen");
        assert!((zoom - fit).abs() < 1e-5, "{zoom} vs {fit}");
        window.press("0", cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).zoom(), None);
        assert_eq!(label(window, "pdf-summary"), format!("PDF, 2 pages · page 1 · {} %", (fit * 100.0).round()));
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
                "A PDF needs SumatraPDF, mutool or pdftoppm, and none is installed: install SumatraPDF from https://www.sumatrapdfreader.org, or set SFOGLIA_SUMATRAPDF to its path; install mutool from the package mupdf-tools, or set SFOGLIA_MUTOOL to its path; install pdftoppm from the package poppler-utils, or set SFOGLIA_PDFTOPPM to its path. The bytes show meanwhile."
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
        assert!(window.try_find("pdf-look-again").is_none(), "the tool is there; its complaint is the news");
        assert!(window.try_find("hex-status").is_none());
    })
    .unwrap();

    // Other tool sets, through panels built directly: pdftoppm without pdfinfo, pdftoppm on an XPS, nothing on an OXPS.
    let xps = dir.join("pages.xps");
    std::fs::copy(fixture("three.pdf"), &xps).unwrap();
    let oxps = dir.join("pages.oxps");
    std::fs::copy(fixture("three.pdf"), &oxps).unwrap();
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
            assert_eq!(
                view.message(),
                "pdftoppm needs pdfinfo from the same package to list the pages: install pdfinfo from the package poppler-utils, or set SFOGLIA_PDFINFO to its path."
            );
            let view = on_xps.read(cx).pdf_view().unwrap().read(cx);
            assert_eq!(view.kind(), "XPS");
            assert_eq!(view.tool_label(), "pdftoppm 9.9");
            assert_eq!(
                view.message(),
                "pdftoppm reads PDF alone: for XPS install mutool from the package mupdf-tools, or set SFOGLIA_MUTOOL to its path"
            );
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
        // The listing ran through the stub, which knows no `-f`: the complaint names the stub as run.
        let view = no_info.read(cx).pdf_view().unwrap().read(cx);
        assert_eq!(view.failure(), Some("mutool: unknown command -f"));
        assert_eq!(view.summary(), "PDF, mutool: unknown command -f");
        let view = on_xps.read(cx).pdf_view().unwrap().read(cx);
        assert_eq!(view.failure(), Some("mutool: unknown command -f"));
        assert_eq!(on_oxps.read(cx).pdf_view().unwrap().read(cx).failure(), None);
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
    let (handle, _workspace, panel, view) = open_pdf(cx, tools, fixture("three.pdf"));
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
            assert_eq!(found.path.file_name(), exe.file_name());
            assert!(found.path.is_file());
            let version = found.version.clone().expect("the version resource");
            assert!(version.starts_with("3."), "{version}");
            assert_eq!(version.split('.').count(), 2, "3.6, not 3.6.0.0: {version}");
            assert_eq!(set.status_line(Tool::SumatraPdf), format!("SumatraPDF {version}"));
            assert_eq!(tools::version_of(Tool::SumatraPdf, &exe), Some(version));
        }
        None => assert!(found.is_none() || found.as_ref().is_some_and(|f| f.path.is_file()), "{found:?}"),
    }
}
