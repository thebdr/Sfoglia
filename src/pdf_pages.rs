//! PDF pages through an installed renderer (C-014): `mutool` (MuPDF), or `pdftoppm` with `pdfinfo` from the
//! same poppler package. The renderer lists the pages with their sizes first, then draws one page at a time
//! into a PNG the view decodes. Everything here is a command line and a parser; the view runs them off the
//! main thread.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A page's size in PDF points, after the page's own rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width_pt: f32,
    pub height_pt: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    MuTool,
    PdfToPpm,
}

/// The renderer as found on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renderer {
    pub backend: Backend,
    pub exe: PathBuf,
    /// `pdfinfo`, for the poppler backend's page list.
    pub info_exe: Option<PathBuf>,
}

impl Renderer {
    /// The command that lists the pages; pdftoppm has none of its own, so without pdfinfo there is no listing
    /// and nothing is run.
    pub fn pages_command(&self, pdf: &Path) -> Result<Command, String> {
        match self.backend {
            Backend::MuTool => {
                let mut command = Command::new(&self.exe);
                command.arg("pages").arg(pdf);
                Ok(command)
            }
            Backend::PdfToPpm => {
                let info = self.info_exe.as_deref().ok_or_else(|| {
                    format!(
                        "pdftoppm needs pdfinfo from the same package to list the pages: {}",
                        crate::tools::Tool::PdfInfo.install_hint()
                    )
                })?;
                let mut command = Command::new(info);
                command.args(["-f", "1", "-l", "1000000"]).arg(pdf);
                Ok(command)
            }
        }
    }

    /// The command that draws page `page` (from 1) at `dpi` into `dir`, and the PNG it leaves there.
    pub fn render_command(&self, pdf: &Path, page: usize, dpi: u32, dir: &Path) -> (Command, PathBuf) {
        let stem = format!("page-{page}-{dpi}");
        match self.backend {
            Backend::MuTool => {
                let out = dir.join(format!("{stem}.png"));
                let mut command = Command::new(&self.exe);
                command.args(["draw", "-F", "png", "-r", &dpi.to_string(), "-o"]).arg(&out).arg(pdf).arg(page.to_string());
                (command, out)
            }
            Backend::PdfToPpm => {
                let prefix = dir.join(&stem);
                let mut command = Command::new(&self.exe);
                let n = page.to_string();
                command.args(["-png", "-r", &dpi.to_string(), "-f", &n, "-l", &n, "-singlefile"]).arg(pdf).arg(&prefix);
                (command, dir.join(format!("{stem}.png")))
            }
        }
    }

    /// The pages of `pdf`, or the tool's own complaint.
    pub fn pages(&self, pdf: &Path) -> Result<Vec<PageSize>, String> {
        let output = run(self.pages_command(pdf)?)?;
        match self.backend {
            Backend::MuTool => parse_mutool_pages(&output),
            Backend::PdfToPpm => parse_pdfinfo(&output),
        }
    }

    /// Draw one page; the PNG's path.
    pub fn render(&self, pdf: &Path, page: usize, dpi: u32, dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let (command, out) = self.render_command(pdf, page, dpi, dir);
        run(command)?;
        if out.is_file() { Ok(out) } else { Err(format!("the renderer left no {}", out.file_name().unwrap_or_default().to_string_lossy())) }
    }
}

/// Run a tool; its standard output, or its first line of complaint on failure.
fn run(mut command: Command) -> Result<String, String> {
    let output = command.output().map_err(|e| format!("cannot run {}: {e}", command.get_program().to_string_lossy()))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let complaint = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let program = Path::new(command.get_program()).file_stem().unwrap_or_default().to_string_lossy().into_owned();
    Err(match output.status.code() {
        _ if !complaint.is_empty() => format!("{program}: {complaint}"),
        Some(code) => format!("{program} failed with exit code {code}"),
        None => format!("{program} was stopped"),
    })
}

/// `mutool pages` output: `<page pagenum="1">`, `<MediaBox l="0" b="0" r="612" t="792" />`, `<Rotate v="90" />`,
/// `</page>` on lines of their own (or together); a quarter turn swaps the sides.
pub fn parse_mutool_pages(text: &str) -> Result<Vec<PageSize>, String> {
    let mut pages = Vec::new();
    let mut current: Option<(f32, f32, i32)> = None;
    for piece in text.split('<').skip(1) {
        let piece = piece.trim_start();
        if piece.starts_with("page ") || piece.starts_with("page>") {
            current = Some((612.0, 792.0, 0));
        } else if let Some(rest) = piece.strip_prefix("MediaBox") {
            let value = |name: &str| attribute(rest, name).and_then(|v| v.parse::<f32>().ok());
            if let (Some(l), Some(b), Some(r), Some(t)) = (value("l"), value("b"), value("r"), value("t")) {
                if let Some(page) = current.as_mut() {
                    page.0 = (r - l).abs();
                    page.1 = (t - b).abs();
                }
            }
        } else if let Some(rest) = piece.strip_prefix("Rotate") {
            if let (Some(page), Some(v)) = (current.as_mut(), attribute(rest, "v").and_then(|v| v.parse::<i32>().ok())) {
                page.2 = v;
            }
        } else if piece.starts_with("/page") {
            if let Some((w, h, rotate)) = current.take() {
                pages.push(sized(w, h, rotate));
            }
        }
    }
    if let Some((w, h, rotate)) = current.take() {
        pages.push(sized(w, h, rotate));
    }
    if pages.is_empty() { Err("mutool listed no pages".to_string()) } else { Ok(pages) }
}

fn sized(w: f32, h: f32, rotate: i32) -> PageSize {
    if rotate.rem_euclid(180) == 90 { PageSize { width_pt: h, height_pt: w } } else { PageSize { width_pt: w, height_pt: h } }
}

/// `name="value"` inside a tag's attributes.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let after = &rest[at + name.len()..];
        let boundary = at == 0 || !rest[..at].ends_with(|c: char| c.is_ascii_alphanumeric());
        if boundary && let Some(value) = after.strip_prefix("=\"") {
            return value.split('"').next();
        }
        rest = after;
    }
    None
}

/// `pdfinfo -f 1 -l N` output: `Pages: 3`, then `Page    2 size: 612 x 792 pts (letter)` and `Page    2 rot: 90`
/// per page; an older poppler prints one `Page size:` for the document.
pub fn parse_pdfinfo(text: &str) -> Result<Vec<PageSize>, String> {
    let mut count: Option<usize> = None;
    let mut whole: Option<(f32, f32)> = None;
    let mut sizes: Vec<(usize, f32, f32)> = Vec::new();
    let mut rotations: Vec<(usize, i32)> = Vec::new();
    for line in text.lines() {
        let (key, value) = match line.split_once(':') {
            Some(pair) => pair,
            None => continue,
        };
        let key = key.trim();
        let value = value.trim();
        if key == "Pages" {
            count = value.parse().ok();
        } else if key == "Page size" {
            whole = points(value);
        } else if let Some(rest) = key.strip_prefix("Page ") {
            let rest = rest.trim();
            if let Some(n) = rest.strip_suffix(" size").and_then(|n| n.trim().parse::<usize>().ok()) {
                if let Some((w, h)) = points(value) {
                    sizes.push((n, w, h));
                }
            } else if let Some(n) = rest.strip_suffix(" rot").and_then(|n| n.trim().parse::<usize>().ok()) {
                if let Ok(r) = value.parse::<i32>() {
                    rotations.push((n, r));
                }
            }
        }
    }
    let count = count.ok_or_else(|| "pdfinfo gave no page count".to_string())?;
    let mut pages = Vec::with_capacity(count);
    for n in 1..=count {
        let (w, h) = sizes
            .iter()
            .find(|(p, _, _)| *p == n)
            .map(|(_, w, h)| (*w, *h))
            .or(whole)
            .ok_or_else(|| format!("pdfinfo gave no size for page {n}"))?;
        let rotate = rotations.iter().find(|(p, _)| *p == n).map_or(0, |(_, r)| *r);
        pages.push(sized(w, h, rotate));
    }
    Ok(pages)
}

/// `612 x 792 pts (letter)` → (612, 792).
fn points(value: &str) -> Option<(f32, f32)> {
    let mut parts = value.split_whitespace();
    let w = parts.next()?.parse().ok()?;
    if parts.next()? != "x" {
        return None;
    }
    let h = parts.next()?.parse().ok()?;
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutool_page_lists_parse_with_their_boxes_and_rotations() {
        let text = "x.pdf:\n<page pagenum=\"1\">\n<MediaBox l=\"0\" b=\"0\" r=\"612\" t=\"792\" />\n<Rotate v=\"0\" />\n</page>\n<page pagenum=\"2\">\n<MediaBox l=\"0\" b=\"0\" r=\"400\" t=\"200\" />\n<Rotate v=\"90\" />\n</page>\n<page pagenum=\"3\">\n<MediaBox l=\"10\" b=\"20\" r=\"310\" t=\"420\" />\n</page>\n";
        assert_eq!(
            parse_mutool_pages(text).unwrap(),
            vec![
                PageSize { width_pt: 612.0, height_pt: 792.0 },
                PageSize { width_pt: 200.0, height_pt: 400.0 },
                PageSize { width_pt: 300.0, height_pt: 400.0 },
            ]
        );
        // One line per page, as a newer mutool prints it, and a flipped box.
        let one_line = "<page pagenum=\"1\"><MediaBox l=\"0\" b=\"792\" r=\"612\" t=\"0\" /><Rotate v=\"270\" /></page>";
        assert_eq!(parse_mutool_pages(one_line).unwrap(), vec![PageSize { width_pt: 792.0, height_pt: 612.0 }]);
        assert_eq!(parse_mutool_pages("x.pdf:\n"), Err("mutool listed no pages".to_string()));
        assert_eq!(attribute(" l=\"1\" bl=\"2\" b=\"3\"", "b"), Some("3"), "a name inside another name is skipped");
    }

    #[test]
    fn pdfinfo_output_parses_per_page_or_for_the_whole_document() {
        let text = "Title:          x\nPages:          3\nPage size:      612 x 792 pts (letter)\nPage    1 size: 612 x 792 pts (letter)\nPage    1 rot:  0\nPage    2 size: 400 x 200 pts\nPage    2 rot:  90\nPage    3 size: 300 x 400 pts\nPage    3 rot:  180\n";
        assert_eq!(
            parse_pdfinfo(text).unwrap(),
            vec![
                PageSize { width_pt: 612.0, height_pt: 792.0 },
                PageSize { width_pt: 200.0, height_pt: 400.0 },
                PageSize { width_pt: 300.0, height_pt: 400.0 },
            ]
        );
        let old = "Pages:          2\nPage size:      595.276 x 841.89 pts (A4)\n";
        assert_eq!(parse_pdfinfo(old).unwrap(), vec![PageSize { width_pt: 595.276, height_pt: 841.89 }; 2]);
        assert_eq!(parse_pdfinfo("Title: x\n"), Err("pdfinfo gave no page count".to_string()));
        assert_eq!(parse_pdfinfo("Pages: 1\n"), Err("pdfinfo gave no size for page 1".to_string()));
    }

    #[test]
    fn the_command_lines_name_the_tool_the_page_and_the_output() {
        let mutool = Renderer { backend: Backend::MuTool, exe: PathBuf::from("/usr/bin/mutool"), info_exe: None };
        let (command, out) = mutool.render_command(Path::new("/d/x.pdf"), 3, 150, Path::new("/tmp/p"));
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["draw", "-F", "png", "-r", "150", "-o", &out.to_string_lossy(), "/d/x.pdf", "3"]);
        assert_eq!(out, Path::new("/tmp/p").join("page-3-150.png"));
        let pages: Vec<String> = mutool.pages_command(Path::new("/d/x.pdf")).unwrap().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(pages, ["pages", "/d/x.pdf"]);

        let poppler = Renderer { backend: Backend::PdfToPpm, exe: PathBuf::from("/usr/bin/pdftoppm"), info_exe: Some(PathBuf::from("/usr/bin/pdfinfo")) };
        let (command, out) = poppler.render_command(Path::new("/d/x.pdf"), 2, 96, Path::new("/tmp/p"));
        let args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-png", "-r", "96", "-f", "2", "-l", "2", "-singlefile", "/d/x.pdf", &Path::new("/tmp/p").join("page-2-96").to_string_lossy()]);
        assert_eq!(out, Path::new("/tmp/p").join("page-2-96.png"));
        let info = poppler.pages_command(Path::new("/d/x.pdf")).unwrap();
        assert_eq!(info.get_program(), "/usr/bin/pdfinfo");
        let args: Vec<String> = info.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, ["-f", "1", "-l", "1000000", "/d/x.pdf"]);
        // Without pdfinfo nothing lists the pages and nothing is run: the hint names the package.
        let alone = Renderer { backend: Backend::PdfToPpm, exe: PathBuf::from("/nowhere/pdftoppm"), info_exe: None };
        let expected = "pdftoppm needs pdfinfo from the same package to list the pages: install pdfinfo from the package poppler-utils, or set SFOGLIA_PDFINFO to its path";
        assert_eq!(alone.pages_command(Path::new("/d/x.pdf")).err().as_deref(), Some(expected));
        assert_eq!(alone.pages(Path::new("/d/x.pdf")), Err(expected.to_string()), "the exe does not exist: had it run, the error would say so");
    }
}
