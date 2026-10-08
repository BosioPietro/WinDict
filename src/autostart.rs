//! "Start with Windows" via the per-user Run key.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::System::Registry::*;

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
        if enabled {
            if let Ok(exe) = std::env::current_exe() {
                let command = HSTRING::from(format!("\"{}\"", exe.display()));
                let bytes = std::slice::from_raw_parts(
                    command.as_ptr() as *const u8,
                    (command.len() + 1) * 2,
                );
                let _ = RegSetValueExW(key, VALUE, None, REG_SZ, Some(bytes));
            }
        } else {
            let _ = RegDeleteValueW(key, VALUE);
        }
        let _ = RegCloseKey(key);
    }
}
