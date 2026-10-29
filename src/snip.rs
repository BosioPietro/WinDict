//! Picking an area of the screen to read text from, as Snipping Tool does:
//! the screen, as it was, fills a window over every monitor, dimmed but
//! for the area being dragged out over it.

use std::cell::RefCell;
use std::time::Instant;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::i18n::tr;
use crate::ocr::Picture;
use crate::selection;
use crate::ui;
use crate::win::{self, Window};

const CLASS: PCWSTR = w!("WinDict.Snip");
/// How bright the screen is around the area, out of 256.
const DIM: u32 = 140;
/// The area's border, in pixels: outside it, so it hides none of it.
const BORDER: i32 = 2;
/// Smaller than this each way, in pixels, a drag is taken for a slip.
const MIN_SIZE: i32 = 4;
/// The hint's text size, how far it's in from its edges, and how far down
/// from the top of the screen it is, in DIPs.
const HINT_TEXT: f32 = 14.0;
const HINT_PAD: (f32, f32) = (16.0, 9.0);
const HINT_TOP: f32 = 24.0;
/// How long the hint takes to fade out of the pointer's way, or back.
const HINT_FADE_MS: f32 = 150.0;
/// The timer that fades it, and how often it ticks.
const FADE_TIMER: usize = 1;
const FADE_TICK_MS: u32 = 10;

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

/// The window an area is picked in. It tells `notify` with `msg` once one
/// is, with a WPARAM of 1, or once it's given up on, with 0; it goes when
/// dropped.
pub struct Snip {
    _window: Window,
}

impl Snip {
    /// Shows the screen as it is now to pick an area of, the area's border
    /// in `accent` (red, green, blue), and what to do at the top of the
    /// monitor `near` is on. None, if it can't.
    pub fn open(notify: HWND, msg: u32, accent: (u8, u8, u8), near: POINT) -> Option<Snip> {
        let (x, y, w, h) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        };
        if w <= 0 || h <= 0 {
            return None;
        }
        let bright = Surface::new(w, h)?;
        let dim = Surface::new(w, h)?;
        let back = Surface::new(w, h)?;
        unsafe {
            let screen = GetDC(None);
            let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
            let copied = BitBlt(bright.dc, 0, 0, w, h, Some(screen), x, y, rop);
            ReleaseDC(None, screen);
            copied.ok()?;
            let _ = GdiFlush();
        }
        let len = (w * h * 4) as usize;
        let (from, to) = unsafe {
            (
                std::slice::from_raw_parts(bright.bits, len),
                std::slice::from_raw_parts_mut(dim.bits, len),
            )
        };
        for (d, s) in to.iter_mut().zip(from) {
            *d = ((*s as u32 * DIM) >> 8) as u8;
        }
        let hint = hint(&dim, (x, y), near);
        register();
        let window = Window::create(
            CLASS,
            w!("WinDict"),
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            WS_POPUP,
        )
        .ok()?;
        let hwnd = window.hwnd();
        let (r, g, b) = accent;
        OVERLAY.set(Some(Overlay {
            notify,
            msg,
            size: (w, h),
            origin: (x, y),
            bright,
            dim,
            back,
            hint,
            accent: COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16),
            from: None,
            to: (0, 0),
            picked: None,
            done: false,
        }));
        // The pointer may be on the hint already.
        let mut pointer = POINT::default();
        if unsafe { GetCursorPos(&mut pointer) }.is_ok() {
            with(|o| {
                hover(hwnd, o, (pointer.x - x, pointer.y - y));
                // Under the pointer from the start, it's never there.
                if let Some(hint) = o.hint.as_mut().filter(|h| h.hidden) {
                    hint.shown = 0.0;
                }
            });
        }
        unsafe {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        }
        selection::take_foreground(hwnd);
        Some(Snip { _window: window })
    }

    /// The area picked, if one was: what it showed, and where it is on the
    /// screen, in pixels.
    pub fn picked(&self) -> Option<(Picture, RECT)> {
        with(|o| {
            let area = o.picked?;
            let (w, h) = (area.right - area.left, area.bottom - area.top);
            let all = unsafe {
                std::slice::from_raw_parts(o.bright.bits, (o.size.0 * o.size.1 * 4) as usize)
            };
            let mut pixels = Vec::with_capacity((w * h * 4) as usize);
            for y in area.top..area.bottom {
                let start = ((y * o.size.0 + area.left) * 4) as usize;
                pixels.extend_from_slice(&all[start..start + (w * 4) as usize]);
            }
            let picture = Picture {
                pixels,
                width: w as u32,
                height: h as u32,
            };
            let (x, y) = o.origin;
            let on_screen = RECT {
                left: area.left + x,
                top: area.top + y,
                right: area.right + x,
                bottom: area.bottom + y,
            };
            Some((picture, on_screen))
        })
        .flatten()
    }
}

impl Drop for Snip {
    fn drop(&mut self) {
        // What it showed goes before its window does.
        OVERLAY.take();
    }
}

/// What the window shows, and the area being dragged out on it.
struct Overlay {
    notify: HWND,
    msg: u32,
    size: (i32, i32),
    /// Where the window's top-left is on the screen.
    origin: (i32, i32),
    /// The screen, as it was; dimmed; and what's drawn before it's shown.
    bright: Surface,
    dim: Surface,
    back: Surface,
    hint: Option<Hint>,
    accent: COLORREF,
    /// Where the drag started, while there's one, and where it's got to,
    /// in window pixels.
    from: Option<(i32, i32)>,
    to: (i32, i32),
    picked: Option<RECT>,
    /// Whether `notify` has been told.
    done: bool,
}

impl Overlay {
    /// The area dragged out so far, in window pixels.
    fn area(&self) -> Option<RECT> {
        let (from, to) = (self.from?, self.to);
        let clamp = |v: i32, most: i32| v.clamp(0, most);
        Some(RECT {
            left: clamp(from.0.min(to.0), self.size.0),
            top: clamp(from.1.min(to.1), self.size.1),
            right: clamp(from.0.max(to.0), self.size.0),
            bottom: clamp(from.1.max(to.1), self.size.1),
        })
    }

    /// Tells `notify` the area picked, or that none was; once.
    fn finish(&mut self, picked: Option<RECT>) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        self.picked = picked;
        let wparam = WPARAM(picked.is_some() as usize);
        unsafe {
            let _ = PostMessageW(Some(self.notify), self.msg, wparam, LPARAM(0));
        }
    }
}

/// The pill saying what to do, over the dimmed screen.
struct Hint {
    /// It, with the dimmed screen past its ends.
    pill: Surface,
    /// Where it is, in window pixels.
    at: RECT,
    /// Whether it's made way for the pointer.
    hidden: bool,
    /// How much of it shows, from 0 to 1, as it fades, and when that was.
    shown: f32,
    at_time: Instant,
}

/// A bitmap of the screen's size, selected into a DC of its own, and its
/// pixels: blue, green, red and an unused byte each, rows from the top.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
}

impl Surface {
    fn new(w: i32, h: i32) -> Option<Surface> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let dc = CreateCompatibleDC(None);
            let mut bits = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, bitmap.into());
            Some(Surface {
                dc,
                bitmap,
                old,
                bits: bits.cast(),
            })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Runs `f` on the overlay, unless there's none, or it's in use: some
/// calls send the window messages before they return.
fn with<R>(f: impl FnOnce(&mut Overlay) -> R) -> Option<R> {
    OVERLAY.with(|o| o.try_borrow_mut().ok()?.as_mut().map(f))
}

fn register() {
    unsafe {
        let instance = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
        let cursor = LoadCursorW(None, IDC_CROSS).unwrap_or_default();
        // Fails, harmlessly, once it's registered.
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(snip_proc),
            hInstance: instance.unwrap_or_default(),
            hCursor: cursor,
            lpszClassName: CLASS,
            ..Default::default()
        });
    }
}

/// The area that changes as the drag goes from `before` to `after`, with
/// their borders.
fn changed(hwnd: HWND, before: Option<RECT>, after: Option<RECT>) {
    for area in [before, after].into_iter().flatten() {
        let mut r = area;
        unsafe {
            let _ = InflateRect(&mut r, BORDER + 1, BORDER + 1);
            let _ = InvalidateRect(Some(hwnd), Some(&r), false);
        }
    }
}

/// What to do, in a pill at the top of the monitor `near` is on, over
/// the dimmed screen `dim`, whose top-left is at `origin` on the screen.
fn hint(dim: &Surface, origin: (i32, i32), near: POINT) -> Option<Hint> {
    let (work, scale) = ui::monitor_at(near);
    let px = |dips: f32| (dips * scale).round() as i32;
    let mut text: Vec<u16> = tr().scan_hint.encode_utf16().collect();
    unsafe {
        let font = CreateFontW(
            -px(HINT_TEXT),
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old = SelectObject(dim.dc, font.into());
        let mut size = RECT::default();
        DrawTextW(
            dim.dc,
            &mut text,
            &mut size,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(dim.dc, old);
        let (pad_x, pad_y) = (px(HINT_PAD.0), px(HINT_PAD.1));
        let (w, h) = (size.right + 2 * pad_x, size.bottom + 2 * pad_y);
        let Some(pill) = Surface::new(w, h) else {
            let _ = DeleteObject(font.into());
            return None;
        };
        let left = (work.left + work.right - w) / 2 - origin.0;
        let top = work.top + px(HINT_TOP) - origin.1;
        // The dimmed screen shows past its rounded ends.
        let _ = BitBlt(pill.dc, 0, 0, w, h, Some(dim.dc), left, top, SRCCOPY);
        let brush = CreateSolidBrush(COLORREF(0x202020));
        let old_brush = SelectObject(pill.dc, brush.into());
        let old_pen = SelectObject(pill.dc, GetStockObject(NULL_PEN));
        let _ = RoundRect(pill.dc, 0, 0, w + 1, h + 1, h, h);
        SelectObject(pill.dc, old_pen);
        SelectObject(pill.dc, old_brush);
        let _ = DeleteObject(brush.into());
        let old = SelectObject(pill.dc, font.into());
        SetBkMode(pill.dc, TRANSPARENT);
        SetTextColor(pill.dc, COLORREF(0xFFFFFF));
        let mut all = RECT {
            right: w,
            bottom: h,
            ..Default::default()
        };
        DrawTextW(
            pill.dc,
            &mut text,
            &mut all,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(pill.dc, old);
        let _ = DeleteObject(font.into());
        let _ = GdiFlush();
        Some(Hint {
            pill,
            at: RECT {
                left,
                top,
                right: left + w,
                bottom: top + h,
            },
            hidden: false,
            shown: 1.0,
            at_time: Instant::now(),
        })
    }
}

/// Over the hint, it makes way, for what's under it to be picked.
fn hover(hwnd: HWND, o: &mut Overlay, at: (i32, i32)) {
    let Some(hint) = o.hint.as_mut() else {
        return;
    };
    let over = unsafe { PtInRect(&hint.at, POINT { x: at.0, y: at.1 }).as_bool() };
    if std::mem::replace(&mut hint.hidden, over) != over {
        hint.at_time = Instant::now();
        win::set_timer(hwnd, FADE_TIMER, FADE_TICK_MS);
    }
}

/// Fades the hint on, towards hidden or shown. Returns whether it's
/// still fading.
fn fade(hwnd: HWND, o: &mut Overlay) -> bool {
    let Some(hint) = o.hint.as_mut() else {
        return false;
    };
    let now = Instant::now();
    let step = now.duration_since(hint.at_time).as_secs_f32() * 1000.0 / HINT_FADE_MS;
    hint.at_time = now;
    let to = if hint.hidden { 0.0 } else { 1.0 };
    hint.shown = if hint.shown < to {
        (hint.shown + step).min(to)
    } else {
        (hint.shown - step).max(to)
    };
    unsafe {
        let _ = InvalidateRect(Some(hwnd), Some(&hint.at), false);
    }
    hint.shown != to
}

unsafe extern "system" fn snip_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            let at = win::lparam_point(lparam);
            let areas = with(|o| {
                let before = o.area();
                (o.from, o.to) = (Some(at), at);
                (before, o.area())
            });
            if let Some((before, after)) = areas {
                SetCapture(hwnd);
                changed(hwnd, before, after);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let at = win::lparam_point(lparam);
            let areas = with(|o| {
                hover(hwnd, o, at);
                let before = o.area()?;
                o.to = at;
                Some((before, o.area()))
            })
            .flatten();
            if let Some((before, after)) = areas {
                changed(hwnd, Some(before), after);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let at = win::lparam_point(lparam);
            let slip = with(|o| {
                o.to = at;
                let area = o.area()?;
                let big = area.right - area.left >= MIN_SIZE && area.bottom - area.top >= MIN_SIZE;
                if big {
                    o.finish(Some(area));
                    return None;
                }
                o.from = None;
                Some(area)
            })
            .flatten();
            let _ = ReleaseCapture();
            changed(hwnd, slip, None);
            LRESULT(0)
        }
        // Another window took the mouse: the drag's off.
        WM_CAPTURECHANGED => {
            let dropped = with(|o| {
                let area = o.area().filter(|_| !o.done);
                o.from = None;
                area
            })
            .flatten();
            changed(hwnd, dropped, None);
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            with(|o| o.finish(None));
            LRESULT(0)
        }
        WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
            with(|o| o.finish(None));
            LRESULT(0)
        }
        // Switching to another app gives up on it.
        WM_ACTIVATE if wparam.0 & 0xFFFF == WA_INACTIVE as usize => {
            with(|o| o.finish(None));
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == FADE_TIMER => {
            if !with(|o| fade(hwnd, o)).unwrap_or(false) {
                win::kill_timer(hwnd, FADE_TIMER);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // It covers every monitor whatever their scale: it stays as it is.
        WM_DPICHANGED => LRESULT(0),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            with(|o| paint(o, hdc, ps.rcPaint));
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Draws `r` of the window: the dimmed screen, the hint on it, the area
/// bright, and its border around it.
fn paint(o: &Overlay, hdc: HDC, r: RECT) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    unsafe {
        let _ = BitBlt(
            o.back.dc,
            r.left,
            r.top,
            w,
            h,
            Some(o.dim.dc),
            r.left,
            r.top,
            SRCCOPY,
        );
        if let Some(hint) = o.hint.as_ref().filter(|h| h.shown > 0.0) {
            let mut on = RECT::default();
            if IntersectRect(&mut on, &hint.at, &r).as_bool() {
                let (x, y) = (on.left, on.top);
                let (from_x, from_y) = (x - hint.at.left, y - hint.at.top);
                let (w, h) = (on.right - on.left, on.bottom - on.top);
                // Eased, as it fades.
                let t = hint.shown;
                let eased = t * t * (3.0 - 2.0 * t);
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    SourceConstantAlpha: (eased * 255.0).round() as u8,
                    ..Default::default()
                };
                let pill = hint.pill.dc;
                let _ = AlphaBlend(o.back.dc, x, y, w, h, pill, from_x, from_y, w, h, blend);
            }
        }
        if let Some(area) = o.area() {
            let mut lit = RECT::default();
            if IntersectRect(&mut lit, &area, &r).as_bool() {
                let (lw, lh) = (lit.right - lit.left, lit.bottom - lit.top);
                let (x, y) = (lit.left, lit.top);
                let _ = BitBlt(o.back.dc, x, y, lw, lh, Some(o.bright.dc), x, y, SRCCOPY);
            }
            let brush = CreateSolidBrush(o.accent);
            let mut frame = area;
            for _ in 0..BORDER {
                let _ = InflateRect(&mut frame, 1, 1);
                FrameRect(o.back.dc, &frame, brush);
            }
            let _ = DeleteObject(brush.into());
        }
        let _ = BitBlt(
            hdc,
            r.left,
            r.top,
            w,
            h,
            Some(o.back.dc),
            r.left,
            r.top,
            SRCCOPY,
        );
    }
}
