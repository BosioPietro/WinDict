//! Notification-area icon. Its menu is drawn by `menu.rs`.

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ICON_ID: u32 = 1;

/// WinDict's icon, from the resources of the executable `instance` is, as
/// windows show it in the taskbar; none if it isn't there.
pub fn window_icon(instance: HINSTANCE) -> HICON {
    unsafe { LoadIconW(Some(instance), PCWSTR(ICON_ID as usize as *const u16)) }.unwrap_or_default()
}

pub struct Tray {
    hwnd: HWND,
    callback: u32,
    icon: HICON,
    /// Notifications' icon: with `NIIF_LARGE_ICON`, Windows refuses any
    /// smaller than `SM_CXICON`.
    large_icon: HICON,
    /// The icon with a dot on it, while there's an update.
    badged: Option<HICON>,
    /// Shown over the icon, kept to show again once Explorer restarts.
    tip: String,
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
        let load = |cx, cy| unsafe {
            let module = GetModuleHandleW(None).unwrap_or_default();
            LoadImageW(
                Some(HINSTANCE(module.0)),
                PCWSTR(ICON_ID as usize as *const u16),
                IMAGE_ICON,
                GetSystemMetrics(cx),
                GetSystemMetrics(cy),
                LR_DEFAULTCOLOR,
            )
            .map(|h| HICON(h.0))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default()
        };
        let tray = Tray {
            hwnd,
            callback,
            icon: load(SM_CXSMICON, SM_CYSMICON),
            large_icon: load(SM_CXICON, SM_CYICON),
            badged: None,
            tip: "WinDict".into(),
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
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        data
    }

    /// (Re-)adds the icon; also used after Explorer restarts.
    ///
    /// It speaks version 4: the keyboard can open its menu, and the
    /// callback's `wParam` says where, the pointer or the icon.
    pub fn add(&self) {
        let mut data = self.data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = self.callback;
        data.hIcon = self.badged.unwrap_or(self.icon);
        copy_into(&mut data.szTip, &self.tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &data);
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &self.data());
        }
    }

    pub fn set_tip(&mut self, tip: &str) {
        tip.clone_into(&mut self.tip);
        let mut data = self.data();
        data.uFlags = NIF_TIP | NIF_SHOWTIP;
        copy_into(&mut data.szTip, tip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// Puts a dot of `color` (red, green, blue) on the icon, or takes it off.
    pub fn set_badge(&mut self, color: Option<(u8, u8, u8)>) {
        if let Some(old) = self.badged.take() {
            unsafe {
                let _ = DestroyIcon(old);
            }
        }
        self.badged = color.and_then(|c| unsafe { badged(self.icon, c) });
        let mut data = self.data();
        data.uFlags = NIF_ICON;
        data.hIcon = self.badged.unwrap_or(self.icon);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn notify(&self, title: &str, text: &str) {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
        data.hBalloonIcon = self.large_icon;
        copy_into(&mut data.szInfoTitle, title);
        copy_into(&mut data.szInfo, text);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    pub fn remove(&mut self) {
        self.set_badge(None);
        let data = self.data();
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

/// A copy of `icon` with a dot of `color` in its top right corner.
unsafe fn badged(icon: HICON, color: (u8, u8, u8)) -> Option<HICON> {
    let mut info = ICONINFO::default();
    GetIconInfo(icon, &mut info).ok()?;
    let mut bitmap = BITMAP::default();
    let read = GetObjectW(
        info.hbmColor.into(),
        std::mem::size_of::<BITMAP>() as i32,
        Some(&mut bitmap as *mut BITMAP as *mut _),
    );
    let (w, h) = (bitmap.bmWidth, bitmap.bmHeight);
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            // Top-down.
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u32; (w.max(0) * h.max(0)) as usize];
    let dc = GetDC(None);
    let copied = read != 0
        && !pixels.is_empty()
        && GetDIBits(
            dc,
            info.hbmColor,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        ) == h;
    let mut bits = std::ptr::null_mut();
    let dib = if copied {
        paint_dot(&mut pixels, w as usize, color);
        CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()
    } else {
        None
    };
    ReleaseDC(None, dc);
    let _ = DeleteObject(info.hbmColor.into());
    let _ = DeleteObject(info.hbmMask.into());
    let dib = dib.filter(|_| !bits.is_null())?;
    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
    // All clear: the alpha channel says what's drawn. Its rows are whole
    // words; without bits to start from, it would hold whatever was there.
    let clear = vec![0u8; ((w + 15) / 16 * 2 * h) as usize];
    let mask = CreateBitmap(w, h, 1, 1, Some(clear.as_ptr().cast()));
    let badged = CreateIconIndirect(&ICONINFO {
        fIcon: TRUE,
        hbmMask: mask,
        hbmColor: dib,
        ..Default::default()
    })
    .ok();
    let _ = DeleteObject(dib.into());
    let _ = DeleteObject(mask.into());
    badged
}

/// Paints a dot of `color` in the top right corner of a square of
/// `width` BGRA pixels with straight alpha, with a clear ring around it so
/// it stands out from the icon.
fn paint_dot(pixels: &mut [u32], width: usize, (r, g, b): (u8, u8, u8)) {
    let size = width as f32;
    let radius = size * 0.19;
    let gap = (size * 0.06).max(1.0);
    let inset = radius + size * 0.02;
    let (cx, cy) = (size - inset, inset);
    for (i, pixel) in pixels.iter_mut().enumerate() {
        let (x, y) = ((i % width) as f32 + 0.5, (i / width) as f32 + 0.5);
        let distance = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        // How much of the pixel each circle covers, with a pixel of blur.
        let cover = |r: f32| (r - distance + 0.5).clamp(0.0, 1.0);
        let (dot, ring) = (cover(radius), cover(radius + gap));
        if ring == 0.0 {
            continue;
        }
        let [pb, pg, pr, pa] = pixel.to_le_bytes();
        let under = pa as f32 / 255.0 * (1.0 - ring);
        let alpha = dot + under * (1.0 - dot);
        let mix = |top: u8, bottom: u8| {
            if alpha == 0.0 {
                return 0;
            }
            ((top as f32 * dot + bottom as f32 * under * (1.0 - dot)) / alpha).round() as u8
        };
        *pixel = u32::from_le_bytes([
            mix(b, pb),
            mix(g, pg),
            mix(r, pr),
            (alpha * 255.0).round() as u8,
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_a_dot_in_the_corner() {
        let white = u32::from_le_bytes([255, 255, 255, 255]);
        let mut pixels = vec![white; 32 * 32];
        paint_dot(&mut pixels, 32, (200, 0, 0));
        let at = |x: usize, y: usize| pixels[y * 32 + x].to_le_bytes();
        // The middle of the dot is its color; the icon shows through elsewhere.
        assert_eq!(at(25, 6), [0, 0, 200, 255]);
        assert_eq!(at(4, 4), [255, 255, 255, 255]);
        assert_eq!(at(25, 25), [255, 255, 255, 255]);
        // Around it, a clear ring.
        assert_eq!(at(25, 13)[3], 0);
    }
}
