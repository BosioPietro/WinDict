//! Safe wrappers for the Win32 calls several parts of the app make, so
//! they're written, and their errors handled, in one place.

use windows::core::{w, Result, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Accessibility::{
    SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK, WINEVENTPROC,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, TOOLTIPS_CLASSW, TTF_SUBCLASS,
    TTM_ADDTOOLW, TTM_NEWTOOLRECTW, TTM_POP, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP, TTS_NOPREFIX,
    TTTOOLINFOW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, TrackMouseEvent, UnregisterHotKey, HOT_KEY_MODIFIERS, TME_LEAVE,
    TRACKMOUSEEVENT,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClassNameW, GetCursorPos, GetWindowLongPtrW, KillTimer,
    SendMessageW, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, SetWindowsHookExW,
    ShowWindow, UnhookWindowsHookEx, CW_USEDEFAULT, GWL_EXSTYLE, GWL_STYLE, HHOOK, HOOKPROC,
    HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNORMAL, WINDOWS_HOOK_ID, WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_SYSMENU,
};

/// A window of ours, destroyed when dropped.
pub struct Window(HWND);

impl Window {
    /// Creates a hidden window of `class`, which this module registered.
    pub fn create(
        class: PCWSTR,
        title: PCWSTR,
        ex_style: WINDOW_EX_STYLE,
        style: WINDOW_STYLE,
    ) -> Result<Window> {
        let instance = unsafe { GetModuleHandleW(None) }
            .ok()
            .map(|m| HINSTANCE(m.0));
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style, class, title, style, 0, 0, 0, 0, None, None, instance, None,
            )
        }?;
        Ok(Window(hwnd))
    }

    pub fn hwnd(&self) -> HWND {
        self.0
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// A hook procedure of ours, unhooked when dropped.
pub struct Hook(HHOOK);

impl Hook {
    /// Hooks `proc` in as a low-level hook of `kind` (WH_MOUSE_LL or
    /// WH_KEYBOARD_LL), called on this thread for input anywhere.
    pub fn low_level(kind: WINDOWS_HOOK_ID, proc: HOOKPROC) -> Option<Hook> {
        unsafe { SetWindowsHookExW(kind, proc, None, 0) }
            .ok()
            .map(Hook)
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

/// A WinEvent hook of ours, unhooked when dropped.
pub struct EventHook(HWINEVENTHOOK);

impl EventHook {
    /// Has `proc` called on this thread whenever `event` happens anywhere.
    pub fn new(event: u32, proc: WINEVENTPROC) -> Option<EventHook> {
        let hook =
            unsafe { SetWinEventHook(event, event, None, proc, 0, 0, WINEVENT_OUTOFCONTEXT) };
        (!hook.is_invalid()).then_some(EventHook(hook))
    }
}

impl Drop for EventHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWinEvent(self.0);
        }
    }
}

/// Has `hwnd` sent WM_HOTKEY with `id` whenever `vk` is pressed with
/// `modifiers`, whichever app has the keyboard. Fails if another app, or
/// Windows, has that combination already.
pub fn register_hotkey(hwnd: HWND, id: i32, modifiers: HOT_KEY_MODIFIERS, vk: u32) -> Result<()> {
    unsafe { RegisterHotKey(Some(hwnd), id, modifiers, vk) }
}

/// Gives the combination registered as `id` back to the app with the keyboard.
pub fn unregister_hotkey(hwnd: HWND, id: i32) {
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), id);
    }
}

/// Sends `hwnd` WM_TIMER with `id` in `ms`, and every `ms` after until
/// it's killed. Setting it again starts it over.
pub fn set_timer(hwnd: HWND, id: usize, ms: u32) {
    unsafe {
        SetTimer(Some(hwnd), id, ms, None);
    }
}

/// Stops `hwnd`'s timer `id`, if it's running.
pub fn kill_timer(hwnd: HWND, id: usize) {
    unsafe {
        let _ = KillTimer(Some(hwnd), id);
    }
}

/// Limits where `hwnd` can be clicked, and drawn, to `rect`, in window
/// pixels; `redraw` repaints it straight away.
pub fn set_region(hwnd: HWND, rect: RECT, redraw: bool) {
    unsafe {
        let region = CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom);
        // The window owns the region once it's set; until then, we do.
        if SetWindowRgn(hwnd, Some(region), redraw) == 0 {
            let _ = DeleteObject(region.into());
        }
    }
}

/// A system tooltip showing `text` over an area of `owner`, initially empty.
/// `tool` tells it apart from the owner's other tooltips. The owner destroys
/// it along with itself.
pub fn create_tooltip(owner: HWND, text: &str, tool: usize) -> Option<HWND> {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&controls);
        // Always: the card never becomes the active window.
        let tooltip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            None,
            WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(owner),
            None,
            None,
            None,
        )
        .ok()?;
        // The tooltip keeps its own copy.
        let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let info = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            // Watches the owner's mouse messages itself.
            uFlags: TTF_SUBCLASS,
            hwnd: owner,
            uId: tool,
            lpszText: PWSTR(wide.as_mut_ptr()),
            ..Default::default()
        };
        SendMessageW(
            tooltip,
            TTM_ADDTOOLW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
        Some(tooltip)
    }
}

/// Moves the area of `owner` that `tooltip`'s `tool` shows over to `rect`,
/// in client pixels. An empty one hides it.
pub fn set_tooltip_rect(tooltip: HWND, owner: HWND, tool: usize, rect: RECT) {
    let info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        hwnd: owner,
        uId: tool,
        rect,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tooltip,
            TTM_NEWTOOLRECTW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
        if rect.right <= rect.left {
            SendMessageW(tooltip, TTM_POP, None, None);
        }
    }
}

/// Changes what `tooltip` says over `owner`'s `tool`.
pub fn set_tooltip_text(tooltip: HWND, owner: HWND, tool: usize, text: &str) {
    // The tooltip keeps its own copy.
    let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        hwnd: owner,
        uId: tool,
        lpszText: PWSTR(wide.as_mut_ptr()),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tooltip,
            TTM_UPDATETIPTEXTW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
    }
}

/// Turns a flyout, which is never activated and has no taskbar button,
/// into a window of its own that has both, still above everything, or
/// back. It's hidden and shown again meanwhile, as that's when the taskbar
/// looks at it.
pub fn set_standalone(hwnd: HWND, standalone: bool) {
    let flyout = (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as isize;
    let own = WS_EX_APPWINDOW.0 as isize;
    // A system menu, for Close on the taskbar and Alt+Space.
    let menu = WS_SYSMENU.0 as isize;
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let (ex, style) = if standalone {
            (ex & !flyout | own, style | menu)
        } else {
            (ex & !own | flyout, style & !menu)
        };
        let _ = ShowWindow(hwnd, SW_HIDE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        );
    }
}

/// Whether `hwnd` is activated when clicked, as a flyout isn't.
pub fn activates(hwnd: HWND) -> bool {
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    ex & WS_EX_NOACTIVATE.0 as isize == 0
}

/// Sets the title the taskbar shows for `hwnd`.
pub fn set_title(hwnd: HWND, title: &str) {
    unsafe {
        let _ = SetWindowTextW(hwnd, &HSTRING::from(title));
    }
}

/// Milliseconds since Windows started.
pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// The pointer's position, in screen pixels.
pub fn cursor_pos() -> POINT {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    pt
}

/// The point a mouse message carries in its `lparam`.
pub fn lparam_point(lparam: LPARAM) -> (i32, i32) {
    (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32)
}

/// Asks for WM_MOUSELEAVE once the pointer leaves `hwnd`.
pub fn track_mouse_leave(hwnd: HWND) {
    let mut track = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    unsafe {
        let _ = TrackMouseEvent(&mut track);
    }
}

/// The name of `hwnd`'s window class.
pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..len])
}

/// Opens `target` (a URL or a file) as the shell would, or runs `program`
/// with `parameters`. Returns whether it started.
fn shell_execute(program: &HSTRING, parameters: Option<&HSTRING>) -> bool {
    let parameters = parameters.map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr()));
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            program,
            parameters,
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // Anything above 32 is success.
    result.0 as isize > 32
}

/// Opens `target`, a URL or a file, in the app the user has for it.
pub fn shell_open(target: &HSTRING) -> bool {
    shell_execute(target, None)
}

/// Runs `program` on `argument`.
pub fn shell_run(program: &HSTRING, argument: &HSTRING) -> bool {
    shell_execute(program, Some(argument))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_signed_points() {
        let pack = |x: i16, y: i16| LPARAM(((y as u16 as isize) << 16) | x as u16 as isize);
        assert_eq!(lparam_point(pack(10, 20)), (10, 20));
        // On a monitor left of or above the main one.
        assert_eq!(lparam_point(pack(-5, -300)), (-5, -300));
    }
}
