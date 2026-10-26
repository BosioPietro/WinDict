//! "Start with Windows" via the per-user Run key.
//!
//! There's one entry for every copy, the installed one and those run from
//! a download: each sees and changes it only while it names that copy.

use std::path::{Path, PathBuf};

use windows::core::{w, Owned, HSTRING, PCWSTR};
use windows::Win32::System::Registry::*;

use crate::installed;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("WinDict");

/// Whether this copy starts with Windows.
pub fn is_enabled() -> bool {
    registered().is_some_and(|current| names_this_copy(&current))
}

pub fn set(enabled: bool) {
    unsafe {
        let mut key = HKEY::default();
        if RegCreateKeyW(HKEY_CURRENT_USER, RUN_KEY, &mut key).is_err() {
            return;
        }
        let key = Owned::new(key);
        if enabled {
            if let Some(command) = command() {
                let bytes = std::slice::from_raw_parts(
                    command.as_ptr() as *const u8,
                    (command.len() + 1) * 2,
                );
                let _ = RegSetValueExW(*key, VALUE, None, REG_SZ, Some(bytes));
            }
        } else if is_enabled() {
            let _ = RegDeleteValueW(*key, VALUE);
        }
    }
}

/// Points this copy's Run entry at the running executable. Release files
/// carry the version in their name, so after an update the entry still
/// names the old (possibly deleted) file.
pub fn sync() {
    let Some(command) = command() else { return };
    match registered() {
        Some(current) if current[..] != command[..] && names_this_copy(&current) => set(true),
        _ => {}
    }
}

fn command() -> Option<HSTRING> {
    Some(HSTRING::from(format!("\"{}\"", exe()?.display())))
}

/// What starts this copy. The installed copy starts through WinDict.exe,
/// even when it's the UI access build running: that one may not start
/// once UI access is off.
fn exe() -> Option<PathBuf> {
    match installed::dir() {
        Some(dir) => Some(dir.join(installed::LAUNCHER)),
        None => std::env::current_exe().ok(),
    }
}

/// Whether the Run entry's command line starts this copy: the installed
/// one names its launcher; one from a download, any WinDict executable in
/// its folder, as another version of it is named.
fn names_this_copy(command: &[u16]) -> bool {
    let command = String::from_utf16_lossy(command);
    let registered = Path::new(command.trim().trim_matches('"'));
    if let Some(dir) = installed::dir() {
        return same_path(registered, &dir.join(installed::LAUNCHER));
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let name = registered
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let folder = |p: &Path| p.parent().map(Path::to_path_buf);
    name.starts_with("windict")
        && name.ends_with(".exe")
        && folder(registered)
            .zip(folder(&exe))
            .is_some_and(|(a, b)| same_path(&a, &b))
}

/// Paths are compared as Windows does, ignoring case.
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
}

/// The Run entry's command line, if the entry exists.
fn registered() -> Option<Vec<u16>> {
    unsafe {
        let mut size = 0u32;
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
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
        Some(buf)
    }
}
