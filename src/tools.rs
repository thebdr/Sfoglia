//! Tools outside Sfoglia that some families need (C-014): where each is looked for, in what order, how its
//! version is read, and what to say when it is missing. A `ToolSet` answers on demand and remembers the
//! answer for the process; `refresh` looks again (the status row's `Look again`). An environment variable
//! `SFOGLIA_<TOOL>` set to a path wins over every search (a path to nothing switches the tool off), and tests
//! give a `ToolSet` their own overrides so no machine's installs reach them.

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// The tools Sfoglia can use when they are installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    /// The PDF viewer hosted in the pdf family's panel on Windows.
    SumatraPdf,
    /// MuPDF's `mutool`, the first PDF page renderer.
    MuTool,
    /// poppler's `pdftoppm`, the second PDF page renderer.
    PdfToPpm,
    /// poppler's `pdfinfo`, the page list for `pdftoppm`.
    PdfInfo,
    /// 7-Zip, for rar and the exotic archive containers.
    SevenZip,
    /// LibreOffice, for the fidelity rendering of office documents.
    LibreOffice,
    /// VLC, for audio and video.
    Vlc,
}

impl Tool {
    pub const ALL: [Tool; 7] =
        [Tool::SumatraPdf, Tool::MuTool, Tool::PdfToPpm, Tool::PdfInfo, Tool::SevenZip, Tool::LibreOffice, Tool::Vlc];

    /// The name a message uses.
    pub fn label(self) -> &'static str {
        match self {
            Tool::SumatraPdf => "SumatraPDF",
            Tool::MuTool => "mutool",
            Tool::PdfToPpm => "pdftoppm",
            Tool::PdfInfo => "pdfinfo",
            Tool::SevenZip => "7-Zip",
            Tool::LibreOffice => "LibreOffice",
            Tool::Vlc => "VLC",
        }
    }

    /// The environment variable that overrides the search.
    pub fn override_variable(self) -> &'static str {
        match self {
            Tool::SumatraPdf => "SFOGLIA_SUMATRAPDF",
            Tool::MuTool => "SFOGLIA_MUTOOL",
            Tool::PdfToPpm => "SFOGLIA_PDFTOPPM",
            Tool::PdfInfo => "SFOGLIA_PDFINFO",
            Tool::SevenZip => "SFOGLIA_7ZIP",
            Tool::LibreOffice => "SFOGLIA_LIBREOFFICE",
            Tool::Vlc => "SFOGLIA_VLC",
        }
    }

    /// Where the tool comes from: a site, or a package.
    pub fn source(self) -> &'static str {
        match self {
            Tool::SumatraPdf => "https://www.sumatrapdfreader.org",
            Tool::MuTool => {
                if cfg!(windows) { "https://mupdf.com (mutool.exe in the MuPDF release)" } else { "the package mupdf-tools" }
            }
            Tool::PdfToPpm | Tool::PdfInfo => {
                if cfg!(windows) {
                    "a poppler build for Windows (https://github.com/oschwartz10612/poppler-windows)"
                } else {
                    "the package poppler-utils"
                }
            }
            Tool::SevenZip => "https://www.7-zip.org",
            Tool::LibreOffice => "https://www.libreoffice.org",
            Tool::Vlc => "https://www.videolan.org",
        }
    }

    /// What to do about a missing tool: `install SumatraPDF from https://..., or set SFOGLIA_SUMATRAPDF to its path`.
    pub fn install_hint(self) -> String {
        format!("install {} from {}, or set {} to its path", self.label(), self.source(), self.override_variable())
    }

    /// The names the tool's executable goes by on the search path.
    fn executable_names(self) -> &'static [&'static str] {
        match self {
            Tool::SumatraPdf => &["SumatraPDF.exe"],
            Tool::MuTool => &["mutool.exe", "mutool"],
            Tool::PdfToPpm => &["pdftoppm.exe", "pdftoppm"],
            Tool::PdfInfo => &["pdfinfo.exe", "pdfinfo"],
            Tool::SevenZip => &["7z.exe", "7zz", "7z", "7za"],
            Tool::LibreOffice => &["soffice.exe", "soffice", "libreoffice"],
            Tool::Vlc => &["vlc.exe", "vlc"],
        }
    }

    /// The fixed places the tool installs to, tried before the search path: the handler registered for its
    /// file class, the Program Files folders and the local application data on Windows, `/usr/bin` and
    /// `/usr/local/bin` elsewhere.
    fn known_places(self) -> Vec<PathBuf> {
        let mut places = Vec::new();
        // The handler registered for the file class comes before the install folders.
        if self == Tool::SumatraPdf {
            places.extend(registered_handler("SumatraPDF.pdf"));
        }
        let roots = [
            std::env::var_os("ProgramFiles").map(PathBuf::from),
            std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        ];
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let mut under_roots = |rest: &str| {
            for root in roots.iter().flatten() {
                places.push(root.join(rest));
            }
        };
        match self {
            Tool::SumatraPdf => under_roots("SumatraPDF/SumatraPDF.exe"),
            Tool::SevenZip => under_roots("7-Zip/7z.exe"),
            Tool::LibreOffice => under_roots("LibreOffice/program/soffice.exe"),
            Tool::Vlc => under_roots("VideoLAN/VLC/vlc.exe"),
            Tool::MuTool | Tool::PdfToPpm | Tool::PdfInfo => {}
        }
        if self == Tool::SumatraPdf {
            places.extend(local.map(|l| l.join("SumatraPDF/SumatraPDF.exe")));
        }
        for name in self.executable_names().iter().filter(|n| !n.ends_with(".exe")) {
            places.push(PathBuf::from("/usr/bin").join(name));
            places.push(PathBuf::from("/usr/local/bin").join(name));
        }
        places
    }

    /// The arguments that make the tool print its version, when it has such.
    fn version_arguments(self) -> Option<&'static [&'static str]> {
        match self {
            Tool::SumatraPdf => None,
            Tool::MuTool | Tool::PdfToPpm | Tool::PdfInfo => Some(&["-v"]),
            Tool::SevenZip => Some(&[]),
            Tool::LibreOffice | Tool::Vlc => Some(&["--version"]),
        }
    }
}

/// A tool as found: where, and the version it reports (when it reports one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub version: Option<String>,
}

impl Found {
    /// `SumatraPDF 3.6`, or `SumatraPDF at C:\...` when no version is known.
    pub fn describe(&self, tool: Tool) -> String {
        match &self.version {
            Some(version) => format!("{} {version}", tool.label()),
            None => format!("{} at {}", tool.label(), self.path.display()),
        }
    }
}

/// What the search reads: the process's own environment, or one a test fixes.
enum Environment {
    Real,
    Fixed { variables: HashMap<String, OsString>, search_path: Option<OsString>, places: HashMap<Tool, Vec<PathBuf>> },
}

/// The tools as this process sees them.
pub struct ToolSet {
    overrides: HashMap<Tool, Option<PathBuf>>,
    environment: Environment,
    cache: Mutex<HashMap<Tool, Option<Found>>>,
}

impl ToolSet {
    /// The machine's tools: the override variables, the known places, the search path.
    pub fn from_env() -> Self {
        Self { overrides: HashMap::new(), environment: Environment::Real, cache: Mutex::new(HashMap::new()) }
    }

    /// A tool set with the given tools pinned to a path or to nothing (tests), the rest as the machine has them.
    pub fn stubbed(overrides: impl IntoIterator<Item = (Tool, Option<PathBuf>)>) -> Self {
        Self { overrides: overrides.into_iter().collect(), environment: Environment::Real, cache: Mutex::new(HashMap::new()) }
    }

    /// A tool set searching a fixed environment (tests): these override variables, this search path and
    /// these known places alone, nothing of the machine's.
    pub fn in_environment(
        variables: impl IntoIterator<Item = (String, OsString)>,
        search_path: Option<OsString>,
        places: impl IntoIterator<Item = (Tool, Vec<PathBuf>)>,
    ) -> Self {
        Self {
            overrides: HashMap::new(),
            environment: Environment::Fixed {
                variables: variables.into_iter().collect(),
                search_path,
                places: places.into_iter().collect(),
            },
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn variable(&self, name: &str) -> Option<OsString> {
        match &self.environment {
            Environment::Real => std::env::var_os(name),
            Environment::Fixed { variables, .. } => variables.get(name).cloned(),
        }
    }

    fn search_path(&self) -> Option<OsString> {
        match &self.environment {
            Environment::Real => std::env::var_os("PATH"),
            Environment::Fixed { search_path, .. } => search_path.clone(),
        }
    }

    fn places(&self, tool: Tool) -> Vec<PathBuf> {
        match &self.environment {
            Environment::Real => tool.known_places(),
            Environment::Fixed { places, .. } => places.get(&tool).cloned().unwrap_or_default(),
        }
    }

    /// Where the tool is, if it is installed: the override first (a path that is no file counts as not
    /// installed), then the known places, then the search path. Remembered until `refresh`.
    pub fn find(&self, tool: Tool) -> Option<Found> {
        if let Some(found) = self.cache.lock().unwrap().get(&tool) {
            return found.clone();
        }
        let found = self.search(tool);
        self.cache.lock().unwrap().insert(tool, found.clone());
        found
    }

    /// Look again, after an install or a change of the override.
    pub fn refresh(&self, tool: Tool) -> Option<Found> {
        self.cache.lock().unwrap().remove(&tool);
        self.find(tool)
    }

    /// The first of the tools that is installed, in the order given.
    pub fn first_found(&self, tools: &[Tool]) -> Option<(Tool, Found)> {
        tools.iter().find_map(|&tool| self.find(tool).map(|found| (tool, found)))
    }

    /// `SumatraPDF 3.6` / `mutool at /usr/bin/mutool` / `SumatraPDF is not installed: install ...`.
    pub fn status_line(&self, tool: Tool) -> String {
        match self.find(tool) {
            Some(found) => found.describe(tool),
            None => format!("{} is not installed: {}", tool.label(), tool.install_hint()),
        }
    }

    fn search(&self, tool: Tool) -> Option<Found> {
        let pinned = match self.overrides.get(&tool) {
            Some(pinned) => Some(pinned.clone()),
            None => self.variable(tool.override_variable()).map(|value| Some(PathBuf::from(value))),
        };
        if let Some(pinned) = pinned {
            let path = pinned?;
            return path.is_file().then(|| Found { version: version_of(tool, &path), path });
        }
        for place in self.places(tool) {
            if place.is_file() {
                return Some(Found { version: version_of(tool, &place), path: place });
            }
        }
        let search_path = self.search_path()?;
        for dir in std::env::split_paths(&search_path) {
            for name in tool.executable_names() {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(Found { version: version_of(tool, &candidate), path: candidate });
                }
            }
        }
        None
    }
}

/// The message for a family none of whose tools is installed: each tool named with what to do about it.
pub fn missing_message(what: &str, tools: &[Tool]) -> String {
    let names: Vec<&str> = tools.iter().map(|t| t.label()).collect();
    let hints: Vec<String> = tools.iter().map(|t| t.install_hint()).collect();
    format!("{what} needs {}, and none is installed: {}.", join_names(&names), hints.join("; "))
}

fn join_names(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => one.to_string(),
        [head @ .., last] => format!("{} or {last}", head.join(", ")),
    }
}

/// The version the tool reports: its version resource on Windows (an `.exe`), else the first number with a
/// dot in what it prints when asked.
pub fn version_of(tool: Tool, path: &Path) -> Option<String> {
    if let Some(version) = resource_version(path) {
        return Some(version);
    }
    let args = tool.version_arguments()?;
    let output = Command::new(path).args(args).output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    parse_version(&text)
}

/// `mutool version 1.24.9` → `1.24.9`; `7-Zip 24.08 (x64) : Copyright` → `24.08`; nothing → none.
pub fn parse_version(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|word| word.trim_end_matches([',', ';', ':']))
        .find(|word| {
            word.chars().next().is_some_and(|c| c.is_ascii_digit())
                && word.contains('.')
                && word.chars().all(|c| c.is_ascii_digit() || c == '.')
        })
        .map(str::to_string)
}

/// `3.6.0.0` → `3.6`, `1.24.9.0` → `1.24.9`: trailing zero parts dropped, two parts kept at least.
pub fn short_version(parts: [u32; 4]) -> String {
    let mut keep = 4;
    while keep > 2 && parts[keep - 1] == 0 {
        keep -= 1;
    }
    parts[..keep].iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}

/// The executable of a registered shell command: `"C:\Program Files\X\x.exe" "%1"` or `C:\x\x.exe "%1"`.
pub fn command_executable(command: &str) -> PathBuf {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        return PathBuf::from(rest.split('"').next().unwrap_or(""));
    }
    let lower = command.to_ascii_lowercase();
    match lower.find(".exe") {
        Some(end) => PathBuf::from(&command[..end + 4]),
        None => PathBuf::from(command.split_whitespace().next().unwrap_or("")),
    }
}

/// The product version in a Windows executable's version resource.
#[cfg(windows)]
fn resource_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VS_FIXEDFILEINFO, VerQueryValueW,
    };
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
        return None;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the strings are null-terminated, the buffer is as large as the system asked for, and the
    // pointer VerQueryValueW hands back points into that buffer for as long as it lives.
    unsafe {
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(wide.as_ptr(), &mut handle);
        if size == 0 {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, size, buffer.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let root: Vec<u16> = "\\".encode_utf16().chain(Some(0)).collect();
        let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(buffer.as_ptr().cast(), root.as_ptr(), &mut info, &mut len) == 0
            || info.is_null()
            || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return None;
        }
        let fixed = std::ptr::read_unaligned(info as *const VS_FIXEDFILEINFO);
        Some(short_version([
            fixed.dwProductVersionMS >> 16,
            fixed.dwProductVersionMS & 0xffff,
            fixed.dwProductVersionLS >> 16,
            fixed.dwProductVersionLS & 0xffff,
        ]))
    }
}

#[cfg(not(windows))]
fn resource_version(_: &Path) -> Option<String> {
    None
}

/// The executable registered for a file class (`HKCU` then `HKLM`), when it exists.
#[cfg(windows)]
fn registered_handler(class: &str) -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let sub: Vec<u16> =
        format!(r"Software\Classes\{class}\shell\open\command").encode_utf16().chain(Some(0)).collect();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut len = 0u32;
        // SAFETY: the key name is null-terminated and the buffer is as long as the first call asked for.
        unsafe {
            if RegGetValueW(root, sub.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut len)
                != 0
            {
                continue;
            }
            let mut buffer = vec![0u16; len as usize / 2 + 1];
            if RegGetValueW(root, sub.as_ptr(), std::ptr::null(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut len)
                != 0
            {
                continue;
            }
            let units = (len as usize / 2).min(buffer.len());
            let text = String::from_utf16_lossy(&buffer[..units]);
            let exe = command_executable(text.trim_end_matches('\0'));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn registered_handler(_: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_banners_and_resources() {
        assert_eq!(parse_version("mutool version 1.24.9\n").as_deref(), Some("1.24.9"));
        assert_eq!(
            parse_version("pdftoppm version 24.02.0\nCopyright 2005-2024 The Poppler Developers").as_deref(),
            Some("24.02.0")
        );
        assert_eq!(
            parse_version("\n7-Zip 24.08 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-08-11\n").as_deref(),
            Some("24.08")
        );
        assert_eq!(parse_version("LibreOffice 24.2.7.2 521c2ea5...").as_deref(), Some("24.2.7.2"));
        assert_eq!(
            parse_version("VLC media player 3.0.21 Vetinari (revision 3.0.21-0-gdd8bfdbabe8)").as_deref(),
            Some("3.0.21")
        );
        assert_eq!(parse_version("VLC version 3.0.21 Vetinari, 2024").as_deref(), Some("3.0.21"));
        assert_eq!(parse_version("mutool version 9.9-stub"), None, "a dash is no version");
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("no numbers here, 7 of them"), None);
        assert_eq!(short_version([3, 6, 0, 0]), "3.6");
        assert_eq!(short_version([1, 24, 9, 0]), "1.24.9");
        assert_eq!(short_version([24, 8, 0, 1]), "24.8.0.1");
        assert_eq!(short_version([0, 0, 0, 0]), "0.0");
    }

    #[test]
    fn registered_commands_give_their_executable() {
        assert_eq!(
            command_executable(r#""C:\Program Files\SumatraPDF\SumatraPDF.exe" "%1""#),
            PathBuf::from(r"C:\Program Files\SumatraPDF\SumatraPDF.exe")
        );
        assert_eq!(command_executable(r#"C:\Tools\SumatraPDF.exe "%1""#), PathBuf::from(r"C:\Tools\SumatraPDF.exe"));
        assert_eq!(command_executable(r"C:\Tools\viewer.EXE %1"), PathBuf::from(r"C:\Tools\viewer.EXE"));
        assert_eq!(command_executable("viewer %1"), PathBuf::from("viewer"));
        assert_eq!(command_executable(""), PathBuf::from(""));
    }

    #[test]
    fn hints_name_the_tool_its_source_and_the_variable() {
        assert_eq!(
            Tool::SumatraPdf.install_hint(),
            "install SumatraPDF from https://www.sumatrapdfreader.org, or set SFOGLIA_SUMATRAPDF to its path"
        );
        // The source is the platform's: a package on Linux, a site on Windows (where no package manager is assumed).
        if cfg!(windows) {
            assert_eq!(
                Tool::MuTool.install_hint(),
                "install mutool from https://mupdf.com (mutool.exe in the MuPDF release), or set SFOGLIA_MUTOOL to its path"
            );
            assert_eq!(
                missing_message("A PDF", &[Tool::MuTool, Tool::PdfToPpm]),
                "A PDF needs mutool or pdftoppm, and none is installed: install mutool from https://mupdf.com (mutool.exe in the MuPDF release), or set SFOGLIA_MUTOOL to its path; install pdftoppm from a poppler build for Windows (https://github.com/oschwartz10612/poppler-windows), or set SFOGLIA_PDFTOPPM to its path."
            );
        } else {
            assert_eq!(
                Tool::MuTool.install_hint(),
                "install mutool from the package mupdf-tools, or set SFOGLIA_MUTOOL to its path"
            );
            assert_eq!(
                missing_message("A PDF", &[Tool::MuTool, Tool::PdfToPpm]),
                "A PDF needs mutool or pdftoppm, and none is installed: install mutool from the package mupdf-tools, or set SFOGLIA_MUTOOL to its path; install pdftoppm from the package poppler-utils, or set SFOGLIA_PDFTOPPM to its path."
            );
        }
        assert_eq!(
            missing_message("X", &[Tool::Vlc]),
            "X needs VLC, and none is installed: install VLC from https://www.videolan.org, or set SFOGLIA_VLC to its path."
        );
        for tool in Tool::ALL {
            assert!(tool.override_variable().starts_with("SFOGLIA_"), "{tool:?}");
            assert!(!tool.executable_names().is_empty(), "{tool:?}");
            assert!(!tool.known_places().is_empty() || tool == Tool::SumatraPdf, "{tool:?} has places to look");
        }
    }

    #[test]
    fn a_pinned_tool_is_found_or_not_by_its_path_and_the_cache_forgets_on_refresh() {
        let dir = std::env::temp_dir().join("sfoglia-tools-test");
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join(if cfg!(windows) { "stub-tool.cmd" } else { "stub-tool.sh" });
        let body = if cfg!(windows) {
            "@echo off\r\necho mutool version 9.9\r\n"
        } else {
            "#!/bin/sh\necho mutool version 9.9\n"
        };
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let set = ToolSet::stubbed([
            (Tool::MuTool, Some(script.clone())),
            (Tool::SumatraPdf, None),
            (Tool::PdfToPpm, Some(dir.join("absent"))),
        ]);
        let found = set.find(Tool::MuTool).expect("the stub is a file");
        assert_eq!(found.path, script);
        assert_eq!(found.version.as_deref(), Some("9.9"));
        assert_eq!(set.status_line(Tool::MuTool), "mutool 9.9");
        assert_eq!(set.find(Tool::SumatraPdf), None);
        assert_eq!(set.find(Tool::PdfToPpm), None, "a path to nothing is not installed");
        assert_eq!(
            set.status_line(Tool::PdfToPpm),
            format!("pdftoppm is not installed: {}", Tool::PdfToPpm.install_hint())
        );
        assert_eq!(
            set.first_found(&[Tool::SumatraPdf, Tool::MuTool, Tool::PdfToPpm]).map(|(t, f)| (t, f.path)),
            Some((Tool::MuTool, script.clone()))
        );
        // The answer is remembered: the script gone, `find` still says found; `refresh` looks again.
        std::fs::remove_file(&script).unwrap();
        assert!(set.find(Tool::MuTool).is_some());
        assert_eq!(set.refresh(Tool::MuTool), None);
        assert_eq!(Found { path: PathBuf::from("/x/y"), version: None }.describe(Tool::Vlc), "VLC at /x/y");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stand-in tool at `path`: a script that prints a version.
    fn stub_at(path: &Path, banner: &str) {
        let body = if cfg!(windows) { format!("@echo off\r\necho {banner}\r\n") } else { format!("#!/bin/sh\necho {banner}\n") };
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn the_search_order_is_the_variable_then_the_known_places_then_the_search_path() {
        let dir = std::env::temp_dir().join("sfoglia-tools-order");
        let _ = std::fs::remove_dir_all(&dir);
        let name = if cfg!(windows) { "mutool.exe" } else { "mutool" };
        let on_path = dir.join("bin").join(name);
        let known = dir.join("known").join(name);
        let pinned = dir.join("pinned").join(if cfg!(windows) { "any.cmd" } else { "any.sh" });
        // On Windows a `.exe` stand-in has no version resource and is not run: the version comes out as none;
        // the paths tell the places apart.
        stub_at(&on_path, "mutool version 1.1");
        stub_at(&known, "mutool version 2.2");
        stub_at(&pinned, "mutool version 3.3");
        let path_var = Some(OsString::from(dir.join("bin")));
        let places = [(Tool::MuTool, vec![known.clone()])];

        // The search path alone finds the tool there.
        let set = ToolSet::in_environment([], path_var.clone(), []);
        assert_eq!(set.find(Tool::MuTool).map(|f| f.path), Some(on_path.clone()));
        // A known place wins over the search path.
        let set = ToolSet::in_environment([], path_var.clone(), places.clone());
        assert_eq!(set.find(Tool::MuTool).map(|f| f.path), Some(known.clone()));
        // The variable wins over both.
        let set = ToolSet::in_environment([("SFOGLIA_MUTOOL".to_string(), OsString::from(&pinned))], path_var.clone(), places.clone());
        let found = set.find(Tool::MuTool).unwrap();
        assert_eq!(found.path, pinned);
        assert_eq!(found.version.as_deref(), Some("3.3"), "a script stand-in is run for its banner");
        // The variable pointing at nothing switches the tool off, whatever the places and the path hold.
        let set = ToolSet::in_environment([("SFOGLIA_MUTOOL".to_string(), OsString::from(dir.join("gone")))], path_var.clone(), places.clone());
        assert_eq!(set.find(Tool::MuTool), None);
        // A known place that is no file is skipped for the next one, then the path.
        let set = ToolSet::in_environment([], path_var.clone(), [(Tool::MuTool, vec![dir.join("absent"), known.clone()])]);
        assert_eq!(set.find(Tool::MuTool).map(|f| f.path), Some(known.clone()));
        let set = ToolSet::in_environment([], path_var, [(Tool::MuTool, vec![dir.join("absent")])]);
        assert_eq!(set.find(Tool::MuTool).map(|f| f.path), Some(on_path.clone()));
        // Nothing anywhere: not installed, and the variable of another tool changes nothing.
        let set = ToolSet::in_environment([("SFOGLIA_PDFTOPPM".to_string(), OsString::from(&pinned))], None, []);
        assert_eq!(set.find(Tool::MuTool), None);
        assert_eq!(set.find(Tool::PdfToPpm).map(|f| f.path), Some(pinned.clone()));
        // The real machine's known places for SumatraPDF name the registered handler before the install folders.
        let places = Tool::SumatraPdf.known_places();
        if let Some(handler) = registered_handler("SumatraPDF.pdf") {
            assert_eq!(places.first(), Some(&handler));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
