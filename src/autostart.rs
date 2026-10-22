//! "Start with Windows" via the per-user Run key.

use windows::core::{w, Owned, HSTRING, PCWSTR};
use windows::Win32::System::Registry::*;

use crate::installed;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("WinDict");

pub fn is_enabled() -> bool {
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
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
        } else {
            let _ = RegDeleteValueW(*key, VALUE);
        }
    }
}

/// Points an enabled Run entry at the running executable. Release files carry
/// the version in their name, so after an update the entry still names the
/// old (possibly deleted) file.
pub fn sync() {
    let Some(command) = command() else { return };
    match registered() {
        Some(current) if current[..] != command[..] => set(true),
        _ => {}
    }
}

fn command() -> Option<HSTRING> {
    // The installed copy starts through WinDict.exe, even when it's the UI
    // access build running: that one may not start once UI access is off.
    let exe = match installed::dir() {
        Some(dir) => dir.join(installed::LAUNCHER),
        None => std::env::current_exe().ok()?,
    };
    Some(HSTRING::from(format!("\"{}\"", exe.display())))
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
