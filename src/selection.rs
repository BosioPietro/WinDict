//! Reading the text the user has selected in whatever app has focus.
//!
//! First we ask UI Automation (clean: no clipboard side effects, and it tells
//! us where the selection is on screen). Apps that don't expose a text pattern
//! get the classic fallback: simulate Ctrl+C, read the clipboard, restore it.

use std::thread::sleep;
use std::time::{Duration, Instant};

use windows::core::{w, Interface};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::*;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Marks input we inject so it can be recognised.
const INJECTED: usize = 0x5744_4943; // "WDIC"

const CF_TEXT_ID: u32 = 1;
const CF_DIB_ID: u32 = 8;
const CF_OEMTEXT_ID: u32 = 7;
const CF_UNICODETEXT_ID: u32 = 13;
const CF_HDROP_ID: u32 = 15;
const CF_LOCALE_ID: u32 = 16;
const CF_DIBV5_ID: u32 = 17;

pub struct Selection {
    pub text: String,
    /// Screen rectangle of the selection, when known.
    pub anchor: Option<RECT>,
}

pub struct Capturer {
    uia: Option<IUIAutomation>,
    clipboard_owner: HWND,
}

impl Capturer {
    /// Must be called on the thread that will call [`Capturer::capture`].
    pub fn new(use_ui_automation: bool) -> Capturer {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia = if use_ui_automation {
                create_uia()
            } else {
                None
            };
            let clipboard_owner = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("WinDict clipboard"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0)),
                None,
            )
            .unwrap_or_default();
            Capturer {
                uia,
                clipboard_owner,
            }
        }
    }

    pub fn capture(&self) -> Option<Selection> {
        let foreground = unsafe { GetForegroundWindow() };
        if let Some(selection) = self.via_ui_automation() {
            return Some(selection);
        }
        // In terminals Ctrl+C without a selection interrupts the running program.
        if is_terminal(foreground) {
            return None;
        }
        self.via_clipboard()
            .map(|text| Selection { text, anchor: None })
    }

    fn via_ui_automation(&self) -> Option<Selection> {
        let uia = self.uia.as_ref()?;
        unsafe {
            let element = uia.GetFocusedElement().ok()?;
            let pattern: IUIAutomationTextPattern =
                element.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
            let ranges = pattern.GetSelection().ok()?;
            if ranges.Length().ok()? < 1 {
                return None;
            }
            let range = ranges.GetElement(0).ok()?;
            let text = range.GetText(512).ok()?.to_string();
            if text.trim().is_empty() {
                return None;
            }
            let anchor = bounding_rect(&range);
            Some(Selection { text, anchor })
        }
    }

    fn via_clipboard(&self) -> Option<String> {
        release_modifiers();
        let saved = SavedClipboard::take(self.clipboard_owner);
        let before = unsafe { GetClipboardSequenceNumber() };
        send_copy();

        let deadline = Instant::now() + Duration::from_millis(600);
        let mut changed = false;
        while Instant::now() < deadline {
            sleep(Duration::from_millis(10));
            if unsafe { GetClipboardSequenceNumber() } != before {
                changed = true;
                break;
            }
        }
        let text = if changed {
            // Some apps write several formats one after another.
            sleep(Duration::from_millis(25));
            read_clipboard_text(self.clipboard_owner)
        } else {
            None
        };
        if changed {
            saved.restore(self.clipboard_owner);
        }
        text.filter(|t| !t.trim().is_empty())
    }
}

fn create_uia() -> Option<IUIAutomation> {
    unsafe {
        if let Ok(uia) =
            CoCreateInstance::<_, IUIAutomation2>(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
        {
            // Don't let a hung app stall us.
            let _ = uia.SetConnectionTimeout(800);
            let _ = uia.SetTransactionTimeout(800);
            return uia.cast().ok();
        }
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    }
}

fn bounding_rect(range: &IUIAutomationTextRange) -> Option<RECT> {
    unsafe {
        let array = range.GetBoundingRectangles().ok()?;
        if array.is_null() {
            return None;
        }
        let mut result = None;
        let lower = SafeArrayGetLBound(array, 1).unwrap_or(0);
        let upper = SafeArrayGetUBound(array, 1).unwrap_or(-1);
        let count = (upper - lower + 1).max(0) as usize;
        let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
        if count >= 4 && SafeArrayAccessData(array, &mut data).is_ok() {
            let values = std::slice::from_raw_parts(data as *const f64, count);
            let mut rect: Option<RECT> = None;
            for r in values.as_chunks::<4>().0 {
                let (l, t, w, h) = (r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32);
                if w <= 0 && h <= 0 {
                    continue;
                }
                rect = Some(match rect {
                    None => RECT {
                        left: l,
                        top: t,
                        right: l + w,
                        bottom: t + h,
                    },
                    Some(u) => RECT {
                        left: u.left.min(l),
                        top: u.top.min(t),
                        right: u.right.max(l + w),
                        bottom: u.bottom.max(t + h),
                    },
                });
            }
            let _ = SafeArrayUnaccessData(array);
            result = rect;
        }
        let _ = SafeArrayDestroy(array);
        result
    }
}

fn is_terminal(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    let class = String::from_utf16_lossy(&buf[..len]);
    matches!(
        class.as_str(),
        "ConsoleWindowClass"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
            | "mintty"
            | "VirtualConsoleClass"
            | "PuTTY"
    )
}

// ---------------------------------------------------------------------------
// Keyboard injection

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let extended = matches!(vk, VK_RMENU | VK_RCONTROL | VK_LWIN | VK_RWIN);
    let mut flags = if up {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED,
            },
        },
    }
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

const MODIFIERS: [VIRTUAL_KEY; 8] = [
    VK_LMENU,
    VK_RMENU,
    VK_LWIN,
    VK_RWIN,
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LCONTROL,
    VK_RCONTROL,
];

/// The hotkey's modifiers are probably still held; Ctrl+C must not become
/// Ctrl+Alt+C. Give the user a moment to let go, then release them for them.
fn release_modifiers() {
    let deadline = Instant::now() + Duration::from_millis(350);
    while MODIFIERS.iter().any(|&vk| is_down(vk)) && Instant::now() < deadline {
        sleep(Duration::from_millis(10));
    }
    let held: Vec<VIRTUAL_KEY> = MODIFIERS
        .iter()
        .copied()
        .filter(|&vk| is_down(vk))
        .collect();
    if held.is_empty() {
        return;
    }
    // A throwaway key press keeps a lone Alt/Win release from opening a menu
    // or the Start menu.
    let mut inputs = vec![key(VIRTUAL_KEY(0xFF), false), key(VIRTUAL_KEY(0xFF), true)];
    inputs.extend(held.iter().map(|&vk| key(vk, true)));
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn send_copy() {
    let inputs = [
        key(VK_LCONTROL, false),
        key(VIRTUAL_KEY('C' as u16), false),
        key(VIRTUAL_KEY('C' as u16), true),
        key(VK_LCONTROL, true),
    ];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

// ---------------------------------------------------------------------------
// Clipboard

fn open_clipboard(owner: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return true;
        }
        sleep(Duration::from_millis(15));
    }
    false
}

fn read_clipboard_text(owner: HWND) -> Option<String> {
    if !open_clipboard(owner) {
        return None;
    }
    let text = unsafe {
        GetClipboardData(CF_UNICODETEXT_ID).ok().and_then(|handle| {
            let global = HGLOBAL(handle.0);
            let ptr = GlobalLock(global) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let max = GlobalSize(global) / 2;
            let slice = std::slice::from_raw_parts(ptr, max);
            let len = slice.iter().position(|&c| c == 0).unwrap_or(max);
            let text = String::from_utf16_lossy(&slice[..len]);
            let _ = GlobalUnlock(global);
            Some(text)
        })
    };
    unsafe {
        let _ = CloseClipboard();
    }
    text
}

/// A copy of the memory-backed formats on the clipboard.
struct SavedClipboard {
    formats: Vec<(u32, Vec<u8>)>,
}

impl SavedClipboard {
    fn take(owner: HWND) -> SavedClipboard {
        let mut formats = Vec::new();
        if !open_clipboard(owner) {
            return SavedClipboard { formats };
        }
        unsafe {
            let mut format = EnumClipboardFormats(0);
            let mut total = 0usize;
            while format != 0 {
                if is_restorable(format) {
                    if let Ok(handle) = GetClipboardData(format) {
                        let global = HGLOBAL(handle.0);
                        let size = GlobalSize(global);
                        let ptr = if size > 0 {
                            GlobalLock(global) as *const u8
                        } else {
                            std::ptr::null()
                        };
                        if !ptr.is_null() {
                            if total + size <= 32 * 1024 * 1024 {
                                formats
                                    .push((format, std::slice::from_raw_parts(ptr, size).to_vec()));
                                total += size;
                            }
                            let _ = GlobalUnlock(global);
                        }
                    }
                }
                format = EnumClipboardFormats(format);
            }
            let _ = CloseClipboard();
        }
        SavedClipboard { formats }
    }

    fn restore(self, owner: HWND) {
        if !open_clipboard(owner) {
            return;
        }
        unsafe {
            let _ = EmptyClipboard();
            for (format, data) in &self.formats {
                set_data(*format, data);
            }
            if !self.formats.is_empty() {
                // Keep the restored copy out of clipboard history (Win+V).
                let exclude =
                    RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
                if exclude != 0 {
                    set_data(exclude, &[0, 0, 0, 0]);
                }
            }
            let _ = CloseClipboard();
        }
    }
}

unsafe fn set_data(format: u32, data: &[u8]) {
    unsafe {
        let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) else {
            return;
        };
        let ptr = GlobalLock(global) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(Some(global));
            return;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        let _ = GlobalUnlock(global);
        if SetClipboardData(format, Some(HANDLE(global.0))).is_err() {
            let _ = GlobalFree(Some(global));
        }
    }
}

fn is_restorable(format: u32) -> bool {
    match format {
        CF_TEXT_ID | CF_OEMTEXT_ID | CF_UNICODETEXT_ID | CF_LOCALE_ID | CF_DIB_ID | CF_DIBV5_ID
        | CF_HDROP_ID => true,
        0xC000..=0xFFFF => {
            // Registered formats are memory-backed, except OLE's private plumbing.
            let mut buf = [0u16; 64];
            let len = unsafe { GetClipboardFormatNameW(format, &mut buf) } as usize;
            let name = String::from_utf16_lossy(&buf[..len]);
            !matches!(
                name.as_str(),
                "DataObject" | "Ole Private Data" | "OwnerLink" | "ObjectLink" | "Native"
            )
        }
        _ => false,
    }
}
