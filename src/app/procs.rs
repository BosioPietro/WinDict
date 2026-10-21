//! The window and hook procedures: they turn Windows messages into calls
//! on the app state, and keep to what can't wait for it.

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::*;
use crate::win;

/// Not in the windows crate's message set (it lives with the common controls).
const WM_MOUSELEAVE: u32 = 0x02A3;

pub(super) unsafe extern "system" fn main_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_LOOKUP => with_app(|app| app.on_hotkey()),
                HOTKEY_ESCAPE => with_app(|app| app.dismiss()),
                HOTKEY_BACK | HOTKEY_BACK_ALT => with_app(|app| app.back()),
                HOTKEY_NEXT_LINK => with_app(|app| app.step_link(true)),
                HOTKEY_PREV_LINK => with_app(|app| app.step_link(false)),
                HOTKEY_OPEN_LINK => with_app(|app| app.open_focused_link()),
                HOTKEY_TRANSLATE => with_app(|app| app.toggle_translation()),
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
        WM_APP_WHEEL => {
            with_app(|app| {
                app.popup
                    .as_mut()
                    .map(|p| p.scroll_by(wparam.0 as i16 as i32))
            });
            LRESULT(0)
        }
        WM_APP_CLICK => {
            let (x, y) = win::lparam_point(lparam);
            with_app(|app| app.on_click(x, y));
            LRESULT(0)
        }
        WM_APP_BACK => {
            with_app(|app| app.back());
            LRESULT(0)
        }
        WM_APP_CLOSE_SETTINGS => {
            with_app(|app| app.close_settings());
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
    match msg {
        // Never steal focus from the app the user is reading in.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to drop the hover when the pointer goes.
            win::track_mouse_leave(hwnd);
            let (x, y) = win::lparam_point(lparam);
            with_app(|app| app.popup.as_mut().map(|p| p.hover(Some((x, y)))));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| app.popup.as_mut().map(|p| p.hover(None)));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) & 0xFFFF;
            unsafe {
                let _ = PostMessageW(Some(MSG_HWND.get()), WM_APP_WHEEL, WPARAM(delta), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = PostMessageW(Some(MSG_HWND.get()), WM_APP_CLICK, WPARAM(0), lparam);
            }
            LRESULT(0)
        }
        // The mouse's back button.
        WM_XBUTTONUP => {
            if (wparam.0 >> 16) as u16 == XBUTTON1 {
                unsafe {
                    let _ = PostMessageW(Some(MSG_HWND.get()), WM_APP_BACK, WPARAM(0), LPARAM(0));
                }
            }
            LRESULT(1)
        }
        // A hand over the links, a text cursor over the search field.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let mut pt = POINT::default();
            let located =
                unsafe { GetCursorPos(&mut pt).is_ok() && ScreenToClient(hwnd, &mut pt).as_bool() };
            let cursor = located
                .then(|| {
                    with_app(|app| {
                        let popup = app.popup.as_ref()?;
                        if popup.link_at(pt.x, pt.y).is_some() {
                            Some(IDC_HAND)
                        } else if popup.search_at(pt.x, pt.y) {
                            Some(IDC_IBEAM)
                        } else {
                            None
                        }
                    })
                    .flatten()
                })
                .flatten();
            match cursor {
                Some(cursor) => {
                    unsafe {
                        if let Ok(cursor) = LoadCursorW(None, cursor) {
                            SetCursor(Some(cursor));
                        }
                    }
                    LRESULT(1)
                }
                None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        // Picking a suggestion, or moving the caret, in the search box.
        WM_KEYDOWN => {
            with_app(|app| app.on_key(VIRTUAL_KEY(wparam.0 as u16)));
            LRESULT(0)
        }
        // Typing in the search box.
        WM_CHAR => {
            with_app(|app| app.on_char(wparam.0 as u16));
            LRESULT(0)
        }
        // Alt+F4 while the search box has the keyboard: close the card, but
        // keep the window for next time.
        WM_CLOSE => {
            unsafe {
                let _ = PostMessageW(Some(MSG_HWND.get()), WM_APP_DISMISS, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        // We size ourselves for the target monitor's DPI already.
        WM_DPICHANGED => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
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
            unsafe {
                let _ = ReleaseCapture();
            }
            with_app(|app| app.in_settings(|s| s.release(x, y)));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = (wparam.0 >> 16) as u16 as i16 as i32;
            with_app(|app| app.in_settings(|s| s.wheel(delta).map(|_| None)));
            LRESULT(0)
        }
        // The card has shrunk: the window follows.
        WM_TIMER => {
            with_app(|app| app.in_settings(|s| s.settle().map(|_| None)));
            LRESULT(0)
        }
        // A hand over the link, and an I-beam over the text field.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let mut pt = POINT::default();
            let located =
                unsafe { GetCursorPos(&mut pt).is_ok() && ScreenToClient(hwnd, &mut pt).as_bool() };
            let shape = located
                .then(|| {
                    with_app(|app| {
                        let s = app.settings.as_ref()?;
                        if s.link_at(pt.x, pt.y) {
                            Some(IDC_HAND)
                        } else if s.field_at(pt.x, pt.y) {
                            Some(IDC_IBEAM)
                        } else {
                            None
                        }
                    })
                    .flatten()
                })
                .flatten();
            let Some(shape) = shape else {
                return default();
            };
            unsafe {
                if let Ok(cursor) = LoadCursorW(None, shape) {
                    SetCursor(Some(cursor));
                }
            }
            LRESULT(1)
        }
        // While a shortcut is recorded every key is for it, Alt ones too;
        // otherwise Alt+F4 and Alt+Space work as they do everywhere.
        WM_KEYDOWN | WM_SYSKEYDOWN if msg == WM_KEYDOWN || recording() => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            with_app(|app| app.in_settings(|s| s.key(key)));
            LRESULT(0)
        }
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
            unsafe {
                let _ = PostMessageW(
                    Some(MSG_HWND.get()),
                    WM_APP_CLOSE_SETTINGS,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
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
                unsafe {
                    let _ =
                        PostMessageW(Some(MSG_HWND.get()), WM_APP_DISMISS, WPARAM(0), LPARAM(0));
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
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
    // Nor the card itself taking the keyboard for the search box.
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == unsafe { GetCurrentProcessId() } {
        return;
    }
    unsafe {
        let _ = PostMessageW(Some(MSG_HWND.get()), WM_APP_DISMISS, WPARAM(0), LPARAM(0));
    }
}
