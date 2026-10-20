//! Notification-area icon. Its menu is drawn by `menu.rs`.

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ICON_ID: u32 = 1;

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
}
