//! The installed edition: what installer/windict.iss puts in Program Files,
//! as opposed to an executable run from wherever it was downloaded.
//!
//! The installer puts two builds side by side: WinDict.exe, which the
//! shortcut and the Run entry start, and WinDict-uiaccess.exe, which can
//! show the card above Start and Search. That one only starts once
//! installer/uiaccess.ps1 has signed it, which also sets `UIAccess` under
//! `HKLM\Software\WinDict`; WinDict.exe then hands over to it.

use std::path::PathBuf;

use windows::core::{w, Owned, HSTRING, PCWSTR};
use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

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
    let here = parent.to_string_lossy();
    here.trim_end_matches('\\')
        .eq_ignore_ascii_case(installed.trim_end_matches('\\'))
        .then(|| parent.to_path_buf())
}

/// Whether UI access can be turned on: only the installed copy can have it.
#[allow(dead_code)] // For the onboarding, which doesn't exist yet.
pub fn ui_access_available() -> bool {
    dir().is_some()
}

/// Whether UI access is on.
#[allow(dead_code)]
pub fn ui_access_on() -> bool {
    read_dword(w!("UIAccess")) == Some(1)
}

/// Why UI access couldn't be turned on or off.
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn set_ui_access(on: bool) -> Result<(), UiAccessError> {
    let dir = dir().ok_or(UiAccessError::NotInstalled)?;
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let powershell = HSTRING::from(
        PathBuf::from(system)
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
/// one runs instead.
pub fn hand_over() -> bool {
    if cfg!(feature = "uiaccess") || read_dword(w!("UIAccess")) != Some(1) {
        return false;
    }
    let Some(dir) = dir() else {
        return false;
    };
    let exe = HSTRING::from(dir.join(UIACCESS).as_os_str());
    // Only the shell can start an executable that asks for UI access:
    // CreateProcess refuses it.
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(exe.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info).is_ok() }
}

fn read_dword(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(value)
}

fn read_string(name: PCWSTR) -> Option<String> {
    unsafe {
        let mut size = 0u32;
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        buf.truncate(size as usize / 2);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Some(String::from_utf16_lossy(&buf))
    }
}
