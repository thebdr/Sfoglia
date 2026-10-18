//! The SumatraPDF host (C-014, Windows): one SumatraPDF process per PDF tab, started in its plugin mode with
//! Sfoglia's window as the parent, so its frame is a child window placed over the panel. The frame does not
//! follow the panel by itself: the view calls `place` on every paint and `show` when the tab goes inactive.
//! Commands reach that one instance through `WM_COPYDATA` carrying a DDE command text, never through the
//! DDE server every SumatraPDF registers under the same name.

use std::ffi::OsString;
use std::os::windows::io::AsRawHandle as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{HANDLE, HWND};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowThreadProcessId, MoveWindow, SW_HIDE, SW_SHOW, SendMessageW, ShowWindow, WM_COPYDATA,
};

/// The class of SumatraPDF's top window, a child of ours in plugin mode.
pub const FRAME_CLASS: &str = "SUMATRA_PDF_FRAME";
/// `WM_COPYDATA`'s tag for a DDE command in UTF-16 (`DdeW`), as SumatraPDF's frame expects it.
pub const DDE_WIDE: usize = 0x4464_6557;

/// Where the frame sits in the parent's client area, in device pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// What `WM_COPYDATA` carries: the command text in UTF-16 with its terminating zero (`cbData` is twice this length).
pub fn payload(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// What to do with a frame the moment it is found: SumatraPDF shows it, so a hidden tab hides it; a shown tab
/// leaves it be until the first paint places it.
pub fn command_on_found(shown: bool) -> Option<i32> {
    if shown { None } else { Some(SW_HIDE) }
}

/// The command line that starts SumatraPDF as a plugin of `parent` on `pdf`.
pub fn arguments(parent: usize, pdf: &Path) -> Vec<OsString> {
    vec![OsString::from("-plugin"), OsString::from(parent.to_string()), pdf.as_os_str().to_os_string()]
}

/// A DDE command as SumatraPDF reads it: `[GotoPage("C:\a.pdf", 3)]`. Windows paths carry no quotes.
pub fn dde_command(name: &str, pdf: &Path, rest: &[&str]) -> String {
    let mut text = format!("[{name}(\"{}\"", pdf.display());
    for argument in rest {
        text.push_str(", ");
        text.push_str(argument);
    }
    text.push_str(")]");
    text
}

pub struct SumatraHost {
    child: Child,
    parent: HWND,
    frame: Option<HWND>,
    placed: Option<Placement>,
    shown: bool,
    pdf: PathBuf,
}

impl SumatraHost {
    /// Start SumatraPDF on `pdf` as a child of `parent`.
    pub fn start(exe: &Path, pdf: &Path, parent: usize) -> std::io::Result<Self> {
        let child = Command::new(exe)
            .args(arguments(parent, pdf))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        join_job(&child);
        Ok(Self { child, parent: parent as HWND, frame: None, placed: None, shown: true, pdf: pdf.to_path_buf() })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the process is still there (a crashed or closed SumatraPDF leaves the panel empty).
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Whether the frame window has been found under the parent.
    pub fn is_embedded(&self) -> bool {
        self.frame.is_some()
    }

    /// The frame window of this process among the parent's children (several PDF tabs share the parent).
    fn find_frame(&mut self) -> Option<HWND> {
        if self.frame.is_some() {
            return self.frame;
        }
        let class: Vec<u16> = FRAME_CLASS.encode_utf16().chain(Some(0)).collect();
        let pid = self.child.id();
        let mut after: HWND = std::ptr::null_mut();
        loop {
            // SAFETY: the class name is null-terminated; the handles are only passed back to the system.
            let found = unsafe { FindWindowExW(self.parent, after, class.as_ptr(), std::ptr::null()) };
            if found.is_null() {
                return None;
            }
            let mut owner = 0u32;
            // SAFETY: a valid window handle and an out pointer to a u32.
            unsafe { GetWindowThreadProcessId(found, &mut owner) };
            if owner == pid {
                self.frame = Some(found);
                // A tab already inactive (or on its bytes) by the time the frame appears hides it at once.
                if let Some(command) = command_on_found(self.shown) {
                    // SAFETY: a window handle this process found under its own parent.
                    unsafe { ShowWindow(found, command) };
                }
                return self.frame;
            }
            after = found;
        }
    }

    /// Look for the frame now (the view polls after the start, since an inactive tab never paints); true once
    /// it is known.
    pub fn settle(&mut self) -> bool {
        self.find_frame().is_some()
    }

    /// Put the frame over the panel; nothing happens while the placement is unchanged.
    pub fn place(&mut self, placement: Placement) {
        let Some(frame) = self.find_frame() else { return };
        if self.placed == Some(placement) {
            return;
        }
        // SAFETY: a window handle this process found under its own parent.
        unsafe { MoveWindow(frame, placement.x, placement.y, placement.width, placement.height, 1) };
        self.placed = Some(placement);
        if !self.shown {
            // SAFETY: as above.
            unsafe { ShowWindow(frame, SW_HIDE) };
        }
    }

    /// Show or hide the frame (the tab inactive, the hex view on).
    pub fn show(&mut self, shown: bool) {
        self.shown = shown;
        if let Some(frame) = self.find_frame() {
            // SAFETY: as above.
            unsafe { ShowWindow(frame, if shown { SW_SHOW } else { SW_HIDE }) };
        }
    }

    /// Send a DDE command text to this instance; true when the frame took it.
    pub fn command(&mut self, text: &str) -> bool {
        let Some(frame) = self.find_frame() else { return false };
        let wide = payload(text);
        let data = COPYDATASTRUCT {
            dwData: DDE_WIDE,
            cbData: (wide.len() * 2) as u32,
            lpData: wide.as_ptr() as *mut core::ffi::c_void,
        };
        // SAFETY: `data` and the text it points to outlive the synchronous SendMessageW call.
        let result = unsafe {
            SendMessageW(frame, WM_COPYDATA, self.parent as usize, &data as *const COPYDATASTRUCT as isize)
        };
        result != 0
    }

    /// Go to a page, counted from 1.
    pub fn goto_page(&mut self, page: u32) -> bool {
        let command = dde_command("GotoPage", &self.pdf, &[&page.to_string()]);
        self.command(&command)
    }
}

impl Drop for SumatraHost {
    /// No SumatraPDF outlives its tab.
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The job every hosted SumatraPDF joins: when Sfoglia ends, however it ends, the system ends them with it, so
/// no instance outlives a crash or a kill either. Best effort: a system that refuses the assignment (a job
/// of its own that allows no nesting) keeps the explicit kill on drop alone.
fn job() -> HANDLE {
    static JOB: OnceLock<usize> = OnceLock::new();
    *JOB.get_or_init(|| {
        // SAFETY: a job created with default attributes and no name; the information struct is zeroed
        // apart from the one flag, and its size is passed along.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            job as usize
        }
    }) as HANDLE
}

fn join_job(child: &Child) {
    let job = job();
    if !job.is_null() {
        // SAFETY: both handles are valid for the call; the result is advisory.
        unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
    }
}

/// The parent window's handle as the command line needs it.
pub fn handle_of(window: &gpui_kit::Window) -> Option<usize> {
    use raw_window_handle::RawWindowHandle;
    // GPUI has a `window_handle` of its own: the trait method is named in full.
    match raw_window_handle::HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as usize),
        _ => None,
    }
}

/// Trim helper for the tests: the command line's second argument, the parent, as a number.
pub fn parent_in(arguments: &[OsString]) -> Option<usize> {
    arguments.get(1)?.to_str()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_and_the_dde_text_are_built_as_sumatra_reads_them() {
        let args = arguments(4328700, Path::new(r"C:\docs\three.pdf"));
        assert_eq!(args, vec![OsString::from("-plugin"), OsString::from("4328700"), OsString::from(r"C:\docs\three.pdf")]);
        assert_eq!(parent_in(&args), Some(4328700));
        assert_eq!(dde_command("GotoPage", Path::new(r"C:\docs\three.pdf"), &["3"]), r#"[GotoPage("C:\docs\three.pdf", 3)]"#);
        assert_eq!(
            dde_command("SetView", Path::new(r"C:\docs\a b.pdf"), &["\"continuous\"", "50"]),
            r#"[SetView("C:\docs\a b.pdf", "continuous", 50)]"#
        );
        assert_eq!(dde_command("CmdClose", Path::new(r"C:\x.pdf"), &[]), r#"[CmdClose("C:\x.pdf")]"#);
        assert_eq!(DDE_WIDE, 0x44646557, "the four bytes of DdeW");
        let wide = payload("[GotoPage(\"x\", 1)]");
        assert_eq!(wide.len(), 18 + 1, "the text and its terminating zero");
        assert_eq!(wide.last(), Some(&0));
        assert_eq!(String::from_utf16(&wide[..18]).unwrap(), "[GotoPage(\"x\", 1)]");
        assert_eq!(payload("é").len(), 2, "UTF-16 units, not bytes");
        assert_eq!(command_on_found(true), None, "a shown tab's frame stays as SumatraPDF shows it");
        assert_eq!(command_on_found(false), Some(SW_HIDE), "an inactive tab's frame is hidden the moment it is found");
    }
}
