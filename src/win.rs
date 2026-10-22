//! Safe wrappers for the Win32 calls several parts of the app make, so
//! they're written, and their errors handled, in one place.

use windows::core::{w, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, POINT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::UI::Accessibility::{
    SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK, WINEVENTPROC,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClassNameW, GetCursorPos, KillTimer, SetTimer,
    SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, HOOKPROC, SW_SHOWNORMAL, WINDOWS_HOOK_ID,
    WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT,
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
