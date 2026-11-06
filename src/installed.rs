//! The installed edition: what installer/windict.iss puts in Program Files,
//! as opposed to an executable run from wherever it was downloaded.
//!
//! The installer puts two builds side by side: WinDict.exe, which the
//! shortcut and the Run entry start, and WinDict-uiaccess.exe, which can
//! show the card above Start and Search. That one only starts once
//! installer/uiaccess.ps1 has signed it, which also sets `UIAccess` under
//! `HKLM\Software\WinDict`; WinDict.exe then hands over to it.

use std::path::{Path, PathBuf};

use windows::core::{w, Owned, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

use crate::win;

/// Written by the installer, and only administrators can change it.
const KEY: PCWSTR = w!("Software\\WinDict");
/// What the shortcut and the Run entry start.
pub const LAUNCHER: &str = "WinDict.exe";
const UIACCESS: &str = "WinDict-uiaccess.exe";

/// The folder WinDict is installed in, if this is the installed copy.
pub fn dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    let installed = read_string(w!("InstallDir"))?;
    same_path(parent, Path::new(&installed)).then(|| parent.to_path_buf())
}

/// What starts this copy. The installed copy starts through WinDict.exe,
/// even when it's the UI access build running: that one may not start
/// once UI access is off. One from a download starts itself.
pub fn launcher() -> Option<PathBuf> {
    match dir() {
        Some(dir) => Some(dir.join(LAUNCHER)),
        None => std::env::current_exe().ok(),
    }
}

/// Whether `a` and `b` name the same place, as Windows reads them: case
/// aside, and a trailing backslash too.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
    a.trim_end_matches('\\').to_lowercase() == b.trim_end_matches('\\').to_lowercase()
}

/// Whether `path` names a WinDict executable: this one, or another version
/// of it, as the release files are named.
pub fn is_windict_exe(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    name.starts_with("windict") && name.ends_with(".exe")
}

/// Where the setup installed WinDict, whichever copy this is.
pub fn install_dir() -> Option<PathBuf> {
    read_string(w!("InstallDir")).map(PathBuf::from)
}

/// Whether UI access can be turned on: only the installed copy can have it.
pub fn ui_access_available() -> bool {
    dir().is_some()
}

/// Whether UI access is on.
pub fn ui_access_on() -> bool {
    read_dword(w!("UIAccess")) == Some(1)
}

/// Why UI access couldn't be turned on or off.
#[derive(Debug)]
pub enum UiAccessError {
    /// This isn't the installed copy.
    NotInstalled,
    /// The user said no when Windows asked.
    Cancelled,
    /// installer/uiaccess.ps1 failed, or couldn't be started.
    Failed(String),
}

/// Turns UI access on or off, with installer/uiaccess.ps1. It needs
/// administrator rights, so Windows asks the user first. On success the
/// script restarts WinDict, which ends this process; so this only returns
/// to say what went wrong, unless the restart is slow. It waits for the
/// script: call it off the UI thread.
pub fn set_ui_access(on: bool) -> Result<(), UiAccessError> {
    let dir = dir().ok_or(UiAccessError::NotInstalled)?;
    let powershell = HSTRING::from(
        crate::win::windows_dir()
            .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe")
            .as_os_str(),
    );
    let parameters = HSTRING::from(format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{}\" {} -Restart",
        dir.join("uiaccess.ps1").display(),
        if on { "-Enable" } else { "-Disable" },
    ));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(powershell.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            return Err(if e.code() == ERROR_CANCELLED.to_hresult() {
                UiAccessError::Cancelled
            } else {
                UiAccessError::Failed(e.message())
            });
        }
        // Started some other way, it can't be waited on.
        if info.hProcess.is_invalid() {
            return Err(UiAccessError::Failed(
                "uiaccess.ps1 couldn't be waited on".into(),
            ));
        }
        let process = Owned::new(info.hProcess);
        WaitForSingleObject(*process, INFINITE);
        let mut code = 0u32;
        GetExitCodeProcess(*process, &mut code).map_err(|e| UiAccessError::Failed(e.message()))?;
        match code {
            0 => Ok(()),
            code => Err(UiAccessError::Failed(format!(
                "uiaccess.ps1 exited with {code}"
            ))),
        }
    }
}

/// Starts WinDict-uiaccess.exe in this one's place, if this is the
/// installed WinDict.exe and UI access is on. Returns whether it started:
/// if it can't, because its signature is missing or no longer trusted, this
/// one runs instead. It's started with `switch`, if there's one.
pub fn hand_over(switch: Option<&str>) -> bool {
    if cfg!(feature = "uiaccess") || !ui_access_on() {
        return false;
    }
    let Some(dir) = dir() else {
        return false;
    };
    let exe = HSTRING::from(dir.join(UIACCESS).as_os_str());
    let parameters = HSTRING::from(switch.unwrap_or_default());
    // Only the shell can start an executable that asks for UI access:
    // CreateProcess refuses it.
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info).is_ok() }
}

fn read_dword(name: PCWSTR) -> Option<u32> {
    win::reg_dword(HKEY_LOCAL_MACHINE, KEY, name)
}

fn read_string(name: PCWSTR) -> Option<String> {
    win::reg_string(HKEY_LOCAL_MACHINE, KEY, name)
}
