//! Notification-area icon and its context menu.

use windows::core::{w, PCSTR, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ID_AUTOSTART: u32 = 10;
pub const ID_SETTINGS: u32 = 11;
pub const ID_RELOAD: u32 = 12;
pub const ID_QUIT: u32 = 13;

const ICON_ID: u32 = 1;

pub struct Tray {
    hwnd: HWND,
    callback: u32,
    icon: HICON,
}

fn copy_into(dst: &mut [u16], text: &str) {
    let max = dst.len() - 1;
    let mut n = 0;
    for (d, s) in dst.iter_mut().zip(text.encode_utf16()).take(max) {
        *d = s;
        n += 1;
    }
    dst[n] = 0;
}

impl Tray {
    pub fn new(hwnd: HWND, callback: u32) -> Tray {
        let icon = unsafe {
            let module = GetModuleHandleW(None).unwrap_or_default();
            LoadImageW(
                Some(HINSTANCE(module.0)),
                PCWSTR(ICON_ID as usize as *const u16),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )
            .map(|h| HICON(h.0))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
        };
        let tray = Tray {
            hwnd,
            callback,
            icon,
        };
        tray.add();
        tray
    }

    /// Another handle to the same icon (the icon itself is shared).
    pub fn clone_handle(&self) -> Tray {
        Tray {
            hwnd: self.hwnd,
            callback: self.callback,
            icon: self.icon,
        }
    }

    fn data(&self) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        data.Anonymous.uVersion = NOTIFYICON_VERSION;
        data
    }

    /// (Re-)adds the icon; also used after Explorer restarts.
    pub fn add(&self) {
        let mut data = self.data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = self.callback;
        data.hIcon = self.icon;
        copy_into(&mut data.szTip, "WinDict");
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
        }
    }

    pub fn set_tip(&self, tip: &str) {
        let mut data = self.data();
        data.uFlags = NIF_TIP;
        copy_into(&mut data.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn notify(&self, title: &str, text: &str) {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        data.hBalloonIcon = self.icon;
        copy_into(&mut data.szInfoTitle, title);
        copy_into(&mut data.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn remove(&self) {
        let data = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }

    /// Shows the context menu and returns the chosen command, if any.
    pub fn menu(&self, hotkey: &str, autostart: bool) -> Option<u32> {
        unsafe {
            let menu = CreatePopupMenu().ok()?;
            let title: Vec<u16> = format!("WinDict  \u{2014}  {hotkey}\0")
                .encode_utf16()
                .collect();
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, PCWSTR(title.as_ptr()));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let check = if autostart { MF_CHECKED } else { MF_UNCHECKED };
            let _ = AppendMenuW(
                menu,
                MF_STRING | check,
                ID_AUTOSTART as usize,
                w!("Start with Windows"),
            );
            let _ = AppendMenuW(
                menu,
                MF_STRING,
                ID_SETTINGS as usize,
                w!("Edit settings\u{2026}"),
            );
            let _ = AppendMenuW(menu, MF_STRING, ID_RELOAD as usize, w!("Reload settings"));
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            let _ = AppendMenuW(menu, MF_STRING, ID_QUIT as usize, w!("Quit"));

            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            // Required so the menu closes when clicking elsewhere.
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RIGHTALIGN,
                pt.x,
                pt.y,
                None,
                self.hwnd,
                None,
            );
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            let _ = DestroyMenu(menu);
            (cmd.0 != 0).then_some(cmd.0 as u32)
        }
    }
}

/// Opts the process into dark context menus when Windows is in dark mode.
/// Uses uxtheme's unnamed SetPreferredAppMode export (ordinal 135, Windows 10 1903+).
pub fn allow_dark_menus() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
        else {
            return;
        };
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(135usize as *const u8)) {
            let set_preferred_app_mode: extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_preferred_app_mode(1); // AllowDark
        }
        if let Some(f) = GetProcAddress(uxtheme, PCSTR(136usize as *const u8)) {
            let flush_menu_themes: extern "system" fn() = std::mem::transmute(f);
            flush_menu_themes();
        }
    }
}
