//! The window and hook procedures: they turn Windows messages into calls
//! on the app state, and keep to what can't wait for it.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::*;
use crate::win;

/// Not in the windows crate's message set (it lives with the common controls).
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Posts `msg` to the main window, for the app state to handle once the
/// procedure that saw it has returned.
fn post(msg: u32, wparam: WPARAM, lparam: LPARAM) {
    unsafe {
        let _ = PostMessageW(Some(MSG_HWND.get()), msg, wparam, lparam);
    }
}

/// Answers WM_SETCURSOR over `hwnd` with the cursor `shape_at` picks for
/// where the pointer is, in client pixels. Returns whether it picked one;
/// if not, the window's own cursor is for Windows to show.
fn set_cursor(hwnd: HWND, shape_at: impl FnOnce(&App, POINT) -> Option<PCWSTR>) -> bool {
    let mut pt = POINT::default();
    let located =
        unsafe { GetCursorPos(&mut pt).is_ok() && ScreenToClient(hwnd, &mut pt).as_bool() };
    let shape = located
        .then(|| with_app(|app| shape_at(app, pt)).flatten())
        .flatten();
    let Some(shape) = shape else {
        return false;
    };
    unsafe {
        if let Ok(cursor) = LoadCursorW(None, shape) {
            SetCursor(Some(cursor));
        }
    }
    true
}

/// Opens Snipping Tool for Print Screen released in one of our windows, as
/// Windows would have, if it's set to. Windows watches for the key from a
/// process without UI access, which doesn't hear keys meant for windows of
/// one with it (the signed install). Returns whether it did.
fn snip(wparam: WPARAM) -> bool {
    let snips = VIRTUAL_KEY(wparam.0 as u16) == VK_SNAPSHOT
        && selection::has_ui_access()
        && print_screen_snips();
    snips && win::shell_open(&HSTRING::from("ms-screenclip:"))
}

/// Whether Print Screen opens Snipping Tool, as it does unless turned off.
fn print_screen_snips() -> bool {
    let mut value = 1u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Keyboard"),
            w!("PrintScreenKeyForSnippingEnabled"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
    }
    value != 0
}

/// Where `vk` is kept in a set of keys like [`SWALLOWED`]: which word, and
/// the bit in it.
fn key_bit(vk: u16) -> (usize, u64) {
    ((vk as usize / 64) % 4, 1u64 << (vk % 64))
}

pub(super) unsafe extern "system" fn main_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_LOOKUP => with_app(|app| app.on_hotkey(Shortcut::Lookup)),
                HOTKEY_TRANSLATE_SELECTION => with_app(|app| app.on_hotkey(Shortcut::Translate)),
                HOTKEY_ESCAPE => with_app(|app| app.escape()),
                HOTKEY_BACK | HOTKEY_BACK_ALT => with_app(|app| app.back()),
                HOTKEY_NEXT_LINK => with_app(|app| app.step_link(true)),
                HOTKEY_PREV_LINK => with_app(|app| app.step_link(false)),
                HOTKEY_OPEN_LINK => with_app(|app| app.open_focused_link()),
                HOTKEY_TRANSLATE => with_app(|app| app.toggle_translation()),
                HOTKEY_SCROLL_UP => with_app(|app| app.scroll_line(false)),
                HOTKEY_SCROLL_DOWN => with_app(|app| app.scroll_line(true)),
                HOTKEY_COPY => with_app(|app| app.copy()),
                HOTKEY_ZOOM_IN | HOTKEY_ZOOM_IN_PAD => with_app(|app| app.zoom(Some(1))),
                HOTKEY_ZOOM_OUT | HOTKEY_ZOOM_OUT_PAD => with_app(|app| app.zoom(Some(-1))),
                HOTKEY_ZOOM_RESET | HOTKEY_ZOOM_RESET_PAD => with_app(|app| app.zoom(None)),
                _ => None,
            };
            LRESULT(0)
        }
        WM_APP_EVENT => {
            with_app(|app| app.on_events());
            LRESULT(0)
        }
        WM_APP_DISMISS => {
            with_app(|app| app.dismiss());
            LRESULT(0)
        }
        // From a card's window: the window in `lparam`.
        WM_APP_WHEEL => {
            let card = HWND(lparam.0 as _);
            with_app(|app| app.with_card(card, |app| app.on_wheel(wparam.0 as i16 as i32)));
            LRESULT(0)
        }
        // Likewise, with Ctrl held.
        WM_APP_ZOOM => {
            let card = HWND(lparam.0 as _);
            with_app(|app| app.with_card(card, |app| app.zoom_by_wheel(wparam.0 as i16 as i32)));
            LRESULT(0)
        }
        // From a card's window: the window in `wparam`, the point in `lparam`.
        WM_APP_CLICK => {
            let (x, y) = win::lparam_point(lparam);
            with_app(|app| app.on_click(HWND(wparam.0 as _), x, y));
            LRESULT(0)
        }
        WM_APP_BACK => {
            let card = HWND(wparam.0 as _);
            with_app(|app| app.with_card(card, |app| app.back()));
            LRESULT(0)
        }
        WM_APP_PIN => {
            with_app(|app| app.pin_by_key(HWND(wparam.0 as _)));
            LRESULT(0)
        }
        WM_APP_CLOSE_CARD => {
            with_app(|app| app.close_card(HWND(wparam.0 as _)));
            LRESULT(0)
        }
        WM_APP_CLOSE_SETTINGS => {
            with_app(|app| app.close_settings());
            LRESULT(0)
        }
        WM_APP_CLOSE_INTRO => {
            with_app(|app| app.finish_onboarding());
            LRESULT(0)
        }
        WM_APP_RECORD_KEY => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            let modifiers = HOT_KEY_MODIFIERS((lparam.0 & 0xFFFF) as u32);
            let down = lparam.0 >> 16 & 1 == 1;
            with_app(|app| app.in_settings(|s| s.record_key(key, modifiers, down)));
            LRESULT(0)
        }
        WM_TIMER => {
            with_app(|app| app.on_timer(wparam.0));
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let event = (lparam.0 & 0xFFFF) as u32;
            match event {
                WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                    with_app(|app| app.on_tray_press());
                }
                // Either button opens the menu.
                WM_LBUTTONUP | WM_RBUTTONUP => {
                    with_app(|app| app.on_tray_release());
                }
                WM_CONTEXTMENU => {
                    with_app(|app| app.open_menu());
                }
                _ => {}
            }
            LRESULT(0)
        }
        _ => {
            let taskbar = with_app(|app| app.taskbar_created).unwrap_or(0);
            if msg == taskbar && taskbar != 0 {
                with_app(|app| app.tray.add());
                return LRESULT(0);
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

pub(super) unsafe extern "system" fn popup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Runs `f` on the card shown in this window.
    let card = |f: &mut dyn FnMut(&mut App)| with_app(|app| app.with_card(hwnd, |app| f(app)));
    let default = || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        // Never steal focus from the app the user is reading in, unless the
        // card's pinned, which takes the keyboard like any window.
        WM_MOUSEACTIVATE if win::activates(hwnd) => LRESULT(MA_ACTIVATE as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // A card is dragged by its heading.
        WM_NCHITTEST => {
            let hit = default();
            let (x, y) = win::lparam_point(lparam);
            let mut pt = POINT { x, y };
            let caption = hit.0 == HTCLIENT as isize
                && unsafe { ScreenToClient(hwnd, &mut pt).as_bool() }
                && with_app(|app| app.popup_of(hwnd).is_some_and(|p| p.caption_at(pt.x, pt.y)))
                    .unwrap_or(false);
            if caption {
                LRESULT(HTCAPTION as isize)
            } else {
                hit
            }
        }
        WM_EXITSIZEMOVE => {
            with_app(|app| app.card_moved(hwnd));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to drop the hover when the pointer goes.
            win::track_mouse_leave(hwnd);
            // Windows also sends one when the card changes under a pointer
            // that stays put: that mustn't pick the suggestion it lands on.
            let at = unsafe { GetMessagePos() };
            if POINTER_AT.replace(Some(at)) == Some(at) {
                return LRESULT(0);
            }
            let (x, y) = win::lparam_point(lparam);
            card(&mut |app| app.hover_card(Some((x, y))));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            POINTER_AT.set(None);
            card(&mut |app| app.hover_card(None));
            LRESULT(0)
        }
        // With Ctrl held, it zooms the card instead.
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) & 0xFFFF;
            let ctrl = wparam.0 as u32 & 0x0008 != 0; // MK_CONTROL
            let msg = if ctrl { WM_APP_ZOOM } else { WM_APP_WHEEL };
            post(msg, WPARAM(delta), LPARAM(hwnd.0 as isize));
            LRESULT(0)
        }
        // The scrollbar: its thumb follows the pointer, even off the card,
        // until the button comes up.
        WM_LBUTTONDOWN => {
            let (x, y) = win::lparam_point(lparam);
            let mut pressed = false;
            card(&mut |app| pressed = app.press_card(x, y));
            if pressed {
                unsafe { SetCapture(hwnd) };
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut released = false;
            card(&mut |app| released = app.release_card());
            if released {
                unsafe {
                    let _ = ReleaseCapture();
                }
            } else {
                post(WM_APP_CLICK, WPARAM(hwnd.0 as usize), lparam);
            }
            LRESULT(0)
        }
        // Another window took the mouse: the thumb lets go.
        WM_CAPTURECHANGED => {
            card(&mut |app| {
                app.release_card();
            });
            LRESULT(0)
        }
        // The mouse's back button.
        WM_XBUTTONUP => {
            if (wparam.0 >> 16) as u16 == XBUTTON1 {
                post(WM_APP_BACK, WPARAM(hwnd.0 as usize), LPARAM(0));
            }
            LRESULT(1)
        }
        // A hand over the links, a text cursor over the search field.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let shown = set_cursor(hwnd, |app, pt| {
                let popup = app.popup_of(hwnd)?;
                if popup.link_at(pt.x, pt.y).is_some() {
                    Some(IDC_HAND)
                } else if popup.search_at(pt.x, pt.y) {
                    Some(IDC_IBEAM)
                } else {
                    None
                }
            });
            if shown {
                LRESULT(1)
            } else {
                default()
            }
        }
        WM_KEYUP if snip(wparam) => LRESULT(0),
        // Picking a suggestion, or moving the caret, in the search box; or
        // any of a pinned card's keys.
        WM_KEYDOWN => {
            card(&mut |app| app.on_key(VIRTUAL_KEY(wparam.0 as u16)));
            LRESULT(0)
        }
        // Alt+Left goes back in a pinned card; Alt+F4 and Alt+Space work as
        // they do everywhere.
        WM_SYSKEYDOWN if VIRTUAL_KEY(wparam.0 as u16) == VK_LEFT => {
            card(&mut |app| {
                if app.card.pinned {
                    app.back();
                }
            });
            LRESULT(0)
        }
        // Alt on its own: there's no menu bar to go to. Nor does a pinned
        // card grow to fill the screen when its heading is double-clicked,
        // or go down to the taskbar when its button is clicked.
        WM_SYSCOMMAND
            if matches!((wparam.0 & 0xFFF0) as u32, SC_MAXIMIZE | SC_MINIMIZE)
                || ((wparam.0 & 0xFFF0) as u32 == SC_KEYMENU && lparam.0 == 0) =>
        {
            LRESULT(0)
        }
        // Typing in the search box.
        WM_CHAR => {
            card(&mut |app| app.on_char(wparam.0 as u16));
            LRESULT(0)
        }
        // Esc, Alt+F4, Ctrl+W, or Close on the taskbar: a pinned card goes; any
        // other closes, but its window is kept for next time.
        WM_CLOSE => {
            post(WM_APP_CLOSE_CARD, WPARAM(hwnd.0 as usize), LPARAM(0));
            LRESULT(0)
        }
        // The card's own: other timers on the window, its tooltips', aren't.
        WM_TIMER if matches!(wparam.0, TIMER_SETTLE | TIMER_TRANSLATING) => {
            with_app(|app| app.on_card_timer(hwnd, wparam.0));
            LRESULT(0)
        }
        // We size ourselves for the target monitor's DPI already.
        WM_DPICHANGED => LRESULT(0),
        _ => default(),
    }
}

pub(super) unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Mouse messages carry client coordinates.
    let (x, y) = win::lparam_point(lparam);
    match msg {
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to drop the highlight when the pointer goes.
            win::track_mouse_leave(hwnd);
            with_app(|app| app.menu_hover(Some((x, y))));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| app.menu_hover(None));
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP => {
            with_app(|app| app.menu_click(x, y));
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_KEYDOWN => {
            with_app(|app| app.menu_key(VIRTUAL_KEY(wparam.0 as u16)));
            LRESULT(0)
        }
        // We size ourselves for the target monitor's DPI already.
        WM_DPICHANGED => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub(super) unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Mouse messages carry client coordinates (screen ones for WM_NCHITTEST).
    let (x, y) = win::lparam_point(lparam);
    let recording =
        || with_app(|app| app.settings.as_ref().is_some_and(|s| s.recording())).unwrap_or(false);
    let default = || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        // The header drags the window.
        WM_NCHITTEST => {
            let hit = default();
            let mut pt = POINT { x, y };
            let caption = hit.0 == HTCLIENT as isize
                && unsafe { ScreenToClient(hwnd, &mut pt).as_bool() }
                && with_app(|app| {
                    app.settings
                        .as_ref()
                        .is_some_and(|s| s.caption_at(pt.x, pt.y))
                })
                .unwrap_or(false);
            if caption {
                LRESULT(HTCAPTION as isize)
            } else {
                hit
            }
        }
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to put out the hover when the pointer goes.
            win::track_mouse_leave(hwnd);
            with_app(|app| app.in_settings(|s| s.hover(Some((x, y))).map(|_| None)));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| app.in_settings(|s| s.hover(None).map(|_| None)));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            unsafe { SetCapture(hwnd) };
            with_app(|app| app.in_settings(|s| s.press(x, y)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            // Before the capture goes, which would let go of the thumb.
            with_app(|app| app.in_settings(|s| s.release(x, y)));
            unsafe {
                let _ = ReleaseCapture();
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            with_app(|app| app.in_settings(|s| s.lose_capture().map(|_| None)));
            LRESULT(0)
        }
        // With Ctrl held, it zooms the window instead.
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16 as i32;
            if wparam.0 as u32 & 0x0008 != 0 {
                // MK_CONTROL
                with_app(|app| app.zoom_settings_by_wheel(delta));
            } else {
                // The pointer's in screen coordinates here.
                let mut pt = POINT { x, y };
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut pt);
                }
                let at = (pt.x, pt.y);
                with_app(|app| app.in_settings(|s| s.wheel(delta, at).map(|_| None)));
            }
            LRESULT(0)
        }
        // The card has shrunk: the window follows.
        WM_TIMER => {
            with_app(|app| app.in_settings(|s| s.settle().map(|_| None)));
            LRESULT(0)
        }
        // A hand over the link, and an I-beam over the text field.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let shown = set_cursor(hwnd, |app, pt| {
                let s = app.settings.as_ref()?;
                if s.link_at(pt.x, pt.y) {
                    Some(IDC_HAND)
                } else if s.field_at(pt.x, pt.y) {
                    Some(IDC_IBEAM)
                } else {
                    None
                }
            });
            if shown {
                LRESULT(1)
            } else {
                default()
            }
        }
        // While a shortcut is recorded every key is for it, Alt ones too;
        // otherwise Alt+F4 and Alt+Space work as they do everywhere.
        WM_KEYDOWN | WM_SYSKEYDOWN if msg == WM_KEYDOWN || recording() => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            with_app(|app| app.in_settings(|s| s.key(key)));
            LRESULT(0)
        }
        WM_KEYUP if !recording() && snip(wparam) => LRESULT(0),
        WM_KEYUP | WM_SYSKEYUP if recording() => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            with_app(|app| app.in_settings(|s| s.key_up(key).map(|_| None)));
            LRESULT(0)
        }
        WM_CHAR => {
            let unit = wparam.0 as u16;
            with_app(|app| app.in_settings(|s| s.char(unit)));
            LRESULT(0)
        }
        WM_SYSCHAR if recording() => LRESULT(0),
        // Alt on its own: there's no menu bar to go to.
        WM_SYSCOMMAND if (wparam.0 & 0xFFF0) as u32 == SC_KEYMENU && lparam.0 == 0 => LRESULT(0),
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                with_app(|app| app.in_settings(|s| s.deactivate()));
            } else {
                with_app(|app| app.in_settings(|s| s.activated().map(|_| None)));
            }
            default()
        }
        // Alt+F4, or Close on the taskbar: the window fades out, and only
        // then goes.
        WM_CLOSE => {
            post(WM_APP_CLOSE_SETTINGS, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        // Dragged to a monitor with another scale: redrawn for it.
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            let dpi = (wparam.0 & 0xFFFF) as u32;
            with_app(|app| app.in_settings(|s| s.rescale(dpi, suggested).map(|_| None)));
            LRESULT(0)
        }
        // Light or dark mode, or the accent color, changed.
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
            let colors = msg == WM_DWMCOLORIZATIONCOLORCHANGED
                || (lparam.0 != 0
                    && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }
                        .is_ok_and(|area| area == "ImmersiveColorSet"));
            if colors {
                with_app(|app| app.on_colors_changed());
            }
            if msg == WM_SETTINGCHANGE && wparam.0 == SPI_SETCLIENTAREAANIMATION.0 as usize {
                with_app(|app| app.on_motion_changed());
            }
            default()
        }
        _ => default(),
    }
}

pub(super) unsafe extern "system" fn intro_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Mouse messages carry client coordinates (screen ones for WM_NCHITTEST).
    let (x, y) = win::lparam_point(lparam);
    let default = || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        // The header drags the window.
        WM_NCHITTEST => {
            let hit = default();
            let mut pt = POINT { x, y };
            let caption = hit.0 == HTCLIENT as isize
                && unsafe { ScreenToClient(hwnd, &mut pt).as_bool() }
                && with_app(|app| app.intro.as_ref().is_some_and(|i| i.caption_at(pt.x, pt.y)))
                    .unwrap_or(false);
            if caption {
                LRESULT(HTCAPTION as isize)
            } else {
                hit
            }
        }
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to put out the hover when the pointer goes.
            win::track_mouse_leave(hwnd);
            with_app(|app| app.in_intro(|i| i.hover(Some((x, y))).map(|_| None)));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| app.in_intro(|i| i.hover(None).map(|_| None)));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            unsafe { SetCapture(hwnd) };
            with_app(|app| app.in_intro(|i| i.press(x, y).map(|_| None)));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = ReleaseCapture();
            }
            with_app(|app| app.in_intro(|i| i.release(x, y)));
            LRESULT(0)
        }
        // What slid or faded away is gone.
        WM_TIMER => {
            with_app(|app| app.in_intro(|i| i.on_timer().map(|_| None)));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            with_app(|app| app.in_intro(|i| i.key(key)));
            LRESULT(0)
        }
        WM_KEYUP if snip(wparam) => LRESULT(0),
        // Alt on its own: there's no menu bar to go to.
        WM_SYSCOMMAND if (wparam.0 & 0xFFF0) as u32 == SC_KEYMENU && lparam.0 == 0 => LRESULT(0),
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                with_app(|app| app.in_intro(|i| i.deactivate().map(|_| None)));
            }
            default()
        }
        // Alt+F4, or Close on the taskbar: the window fades out, and only
        // then goes.
        WM_CLOSE => {
            post(WM_APP_CLOSE_INTRO, WPARAM(0), LPARAM(0));
            LRESULT(0)
        }
        // Dragged to a monitor with another scale: redrawn for it.
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            let dpi = (wparam.0 & 0xFFFF) as u32;
            with_app(|app| app.in_intro(|i| i.rescale(dpi, suggested).map(|_| None)));
            LRESULT(0)
        }
        // Light or dark mode, or the accent color, changed.
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
            let colors = msg == WM_DWMCOLORIZATIONCOLORCHANGED
                || (lparam.0 != 0
                    && unsafe { PCWSTR(lparam.0 as *const u16).to_string() }
                        .is_ok_and(|area| area == "ImmersiveColorSet"));
            if colors {
                with_app(|app| app.on_colors_changed());
            }
            default()
        }
        _ => default(),
    }
}

pub(super) unsafe extern "system" fn mouse_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let button_down = matches!(
            wparam.0 as u32,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        );
        if button_down {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let r = CARD_RECT.get();
            let inside = info.pt.x >= r.left
                && info.pt.x < r.right
                && info.pt.y >= r.top
                && info.pt.y < r.bottom;
            if !inside {
                post(WM_APP_DISMISS, WPARAM(0), LPARAM(0));
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// While a shortcut is recorded, every key goes to it before Windows can act
/// on it: Win+Alt+D would open the calendar, and Win alone Start, so the
/// settings window would never see them. Like the mouse hook, it doesn't
/// touch APP, and posts what it caught instead.
pub(super) unsafe extern "system" fn keyboard_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let next = || unsafe { CallNextHookEx(None, code, wparam, lparam) };
    if code < 0 {
        return next();
    }
    let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
    let vk = info.vkCode as u16;
    let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
    let target = RECORDING_IN.get();
    let recording = !target.is_invalid() && unsafe { GetForegroundWindow() } == target;
    let mut swallowed = SWALLOWED.get();
    let (slot, bit) = key_bit(vk);
    let kept = swallowed[slot] & bit != 0;
    if !recording && !kept {
        return next();
    }
    if down {
        swallowed[slot] |= bit;
    } else {
        swallowed[slot] &= !bit;
    }
    SWALLOWED.set(swallowed);
    if recording {
        let modifiers = held_after(&swallowed, vk, down);
        let state = modifiers.0 as isize | (down as isize) << 16;
        post(WM_APP_RECORD_KEY, WPARAM(vk as usize), LPARAM(state));
    }
    // A key that went down before recording started goes back up as usual.
    if down || kept {
        LRESULT(1)
    } else {
        next()
    }
}

/// The modifiers held once `vk` has gone `down` or up: those the hook kept
/// from Windows, and those it didn't, as Windows has them.
fn held_after(swallowed: &[u64; 4], vk: u16, down: bool) -> HOT_KEY_MODIFIERS {
    let held = |key: VIRTUAL_KEY| {
        if key.0 == vk {
            return down;
        }
        let (slot, bit) = key_bit(key.0);
        swallowed[slot] & bit != 0 || unsafe { GetAsyncKeyState(key.0 as i32) } < 0
    };
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    for (keys, modifier) in [
        ([VK_LCONTROL, VK_RCONTROL], MOD_CONTROL),
        ([VK_LMENU, VK_RMENU], MOD_ALT),
        ([VK_LSHIFT, VK_RSHIFT], MOD_SHIFT),
        ([VK_LWIN, VK_RWIN], MOD_WIN),
    ] {
        if keys.into_iter().any(held) {
            modifiers |= modifier;
        }
    }
    modifiers
}

pub(super) unsafe extern "system" fn focus_hook(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    // Ignore the foreground change from closing Start/Search just before showing.
    if win::now_ms().saturating_sub(SHOWN_AT.get()) < FOREGROUND_GRACE_MS {
        return;
    }
    // Nor the card itself taking the keyboard for the search box; but a
    // pinned card is another window, as another app's would be.
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let pinned = win::activates(hwnd) && win::class_name(hwnd) == "WinDict.Popup";
    if pid == unsafe { GetCurrentProcessId() } && !pinned {
        return;
    }
    post(WM_APP_DISMISS, WPARAM(0), LPARAM(0));
}
