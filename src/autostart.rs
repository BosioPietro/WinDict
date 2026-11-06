//! "Start with Windows" via the per-user Run key.
//!
//! There's one entry for every copy, the installed one and those run from
//! a download: each sees and changes it only while it names that copy.

use std::path::Path;

use windows::core::{w, Owned, HSTRING, PCWSTR};
use windows::Win32::System::Registry::*;

use crate::installed::{self, same_path};
use crate::win;

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
                let command = HSTRING::from(&command);
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
    if registered().is_some_and(|current| current != command && names_this_copy(&current)) {
        set(true);
    }
}

fn command() -> Option<String> {
    Some(format!("\"{}\"", installed::launcher()?.display()))
}

/// Whether the Run entry's command line starts this copy: the installed
/// one names its launcher; one from a download, any WinDict executable in
/// its folder, as another version of it is named.
fn names_this_copy(command: &str) -> bool {
    let registered = Path::new(command.trim().trim_matches('"'));
    if let Some(dir) = installed::dir() {
        return same_path(registered, &dir.join(installed::LAUNCHER));
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    installed::is_windict_exe(registered)
        && registered
            .parent()
            .zip(exe.parent())
            .is_some_and(|(a, b)| same_path(a, b))
}

/// The Run entry's command line, if the entry exists.
fn registered() -> Option<String> {
    win::reg_string(HKEY_CURRENT_USER, RUN_KEY, VALUE)
}
