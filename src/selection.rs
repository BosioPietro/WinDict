//! Reading the text the user has selected in whatever app has focus.
//!
//! First we ask UI Automation (clean: no clipboard side effects, and it tells
//! us where the selection is on screen). Apps that don't expose a text pattern
//! get the classic fallback: simulate Ctrl+C, read the clipboard, restore it.

use std::cell::OnceCell;
use std::time::{Duration, Instant};

use windows::core::{w, Interface, Owned, BOOL, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};
use windows::Win32::Security::{GetTokenInformation, TokenUIAccess, TOKEN_QUERY};
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::Threading::*;
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::win;

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

/// What UI Automation knows about the selection.
enum Found {
    Text(Selection),
    /// The focused field has the caret and nothing selected.
    Nothing,
    /// The app can't say (it doesn't support UI Automation's text pattern,
    /// or the caret isn't in the focused element).
    Unknown,
}

/// The first selected range of a text pattern and its text, if any.
fn selected_text(pattern: &IUIAutomationTextPattern) -> Option<(String, IUIAutomationTextRange)> {
    unsafe {
        let ranges = pattern.GetSelection().ok()?;
        if ranges.Length().ok()? < 1 {
            return None;
        }
        let range = ranges.GetElement(0).ok()?;
        let text = range.GetText(512).ok()?.to_string();
        Some((text, range))
    }
}

pub struct Capturer {
    /// Created on first use, as the setting can be turned on later.
    uia: OnceCell<Option<IUIAutomation>>,
    clipboard_owner: HWND,
}

impl Capturer {
    /// Must be called on the thread that will call [`Capturer::capture`].
    pub fn new() -> Capturer {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia = OnceCell::new();
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

    /// Reads the selection, calling `copying` first if it has to wait for a
    /// copy that may take a while to come, or never come. Without
    /// `use_clipboard`, only what UI Automation finds is read.
    pub fn capture(
        &self,
        use_ui_automation: bool,
        use_clipboard: bool,
        copying: impl FnOnce(),
    ) -> Option<Selection> {
        let foreground = unsafe { GetForegroundWindow() };
        // Start and Search sit in a z-order band above every normal window,
        // so the card would open hidden behind them. With UI access (the
        // signed install) the card can go above them instead.
        let flyout = !has_ui_access() && is_shell_flyout(foreground);
        let found = if use_ui_automation {
            self.via_ui_automation()
        } else {
            Found::Unknown
        };
        let known_empty = matches!(found, Found::Nothing);
        let mut selection = match found {
            Found::Text(selection) => Some(selection),
            Found::Nothing | Found::Unknown => None,
        };
        // When nothing is selected, a copy would never come: skip waiting
        // for one where UI Automation has already said so. In terminals
        // Ctrl+C without a selection interrupts the running program.
        if use_clipboard && selection.is_none() && !known_empty && !is_terminal(foreground) {
            // A card can't be shown over Start/Search until they're closed.
            if !flyout {
                copying();
            }
            selection = self
                .via_clipboard()
                .map(|text| Selection { text, anchor: None });
        }
        // Nothing selected: no text, but the caret, if there is one, is
        // where the search box should open.
        if selection.is_none() {
            selection = self.caret(use_ui_automation).map(|r| Selection {
                text: String::new(),
                anchor: Some(r),
            });
        }
        // Close Start/Search before the card appears.
        if flyout {
            close_flyout();
            // Let the closing animation finish before the card appears.
            wait(Duration::from_millis(200));
            if let Some(s) = selection.as_mut() {
                s.anchor = None;
            }
        }
        selection
    }

    fn via_ui_automation(&self) -> Found {
        let Some(uia) = self.uia.get_or_init(create_uia).as_ref() else {
            return Found::Unknown;
        };
        unsafe {
            let Ok(element) = uia.GetFocusedElement() else {
                return Found::Unknown;
            };
            let Ok(pattern) =
                element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            else {
                return Found::Unknown;
            };
            let text = selected_text(&pattern);
            if let Some((text, range)) = text.filter(|(t, _)| !t.trim().is_empty()) {
                let anchor = bounding_rect(&range);
                return Found::Text(Selection { text, anchor });
            }
            // Nothing selected here. Believe it only if the caret is here
            // too: otherwise the selection could be somewhere this field
            // doesn't know about.
            let caret_here = element
                .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
                .is_ok_and(|pattern| {
                    let mut active = BOOL(0);
                    pattern.GetCaretRange(&mut active).is_ok() && active.as_bool()
                });
            if caret_here {
                Found::Nothing
            } else {
                Found::Unknown
            }
        }
    }

    /// The text caret in the foreground app, in screen pixels.
    fn caret(&self, use_ui_automation: bool) -> Option<RECT> {
        // Some apps answer with a rectangle that isn't the caret (empty,
        // as tall as the window, off every screen): ask the next way then.
        let via_uia = if use_ui_automation {
            self.caret_via_ui_automation().filter(is_line_sized)
        } else {
            None
        };
        via_uia.or_else(|| caret_via_accessibility().filter(is_line_sized))
    }

    fn caret_via_ui_automation(&self) -> Option<RECT> {
        let uia = self.uia.get_or_init(create_uia).as_ref()?;
        unsafe {
            let element = uia.GetFocusedElement().ok()?;
            let pattern: IUIAutomationTextPattern2 =
                element.GetCurrentPatternAs(UIA_TextPattern2Id).ok()?;
            let mut active = BOOL(0);
            let caret = pattern.GetCaretRange(&mut active).ok()?;
            if !active.as_bool() {
                return None;
            }
            // Many apps measure the caret itself, as a line with no width;
            // others answer with nonsense (Windows Search puts it two
            // billion pixels up).
            if let Some(r) = bounding_rect(&caret).filter(is_line_sized) {
                return Some(RECT {
                    right: r.left + 1,
                    ..r
                });
            }
            // Measure the character before it, or the one after, and take
            // the edge it sits on. Line breaks don't count, as they're
            // measured oddly or not at all.
            let character = |endpoint, step| -> Option<RECT> {
                let range = caret.Clone().ok()?;
                range
                    .MoveEndpointByUnit(endpoint, TextUnit_Character, step)
                    .ok()?;
                let text = range.GetText(2).ok()?.to_string();
                if text.is_empty() || text.contains(['\r', '\n']) {
                    return None;
                }
                bounding_rect(&range).filter(is_line_sized)
            };
            if let Some(r) = character(TextPatternRangeEndpoint_Start, -1) {
                return Some(RECT {
                    left: r.right - 1,
                    ..r
                });
            }
            if let Some(r) = character(TextPatternRangeEndpoint_End, 1) {
                return Some(RECT {
                    right: r.left + 1,
                    ..r
                });
            }
            // An empty field has no characters to measure: open at the field
            // itself, if it's a single line (a search box, an address bar).
            element
                .CurrentBoundingRectangle()
                .ok()
                .filter(is_line_sized)
        }
    }

    fn via_clipboard(&self) -> Option<String> {
        release_modifiers();
        let saved = SavedClipboard::take(self.clipboard_owner);
        let before = unsafe { GetClipboardSequenceNumber() };
        send_copy();

        let deadline = Instant::now() + Duration::from_millis(400);
        let mut changed = false;
        while Instant::now() < deadline {
            wait(Duration::from_millis(10));
            if unsafe { GetClipboardSequenceNumber() } != before {
                changed = true;
                break;
            }
        }
        let text = if changed {
            // Some apps write several formats one after another.
            wait(Duration::from_millis(25));
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

/// Whether `r` could be a caret or a line of selected text: about a line
/// tall, and on a screen.
pub fn is_line_sized(r: &RECT) -> bool {
    let height = r.bottom - r.top;
    r.right >= r.left
        && height > 0
        && height < 160
        && !unsafe { MonitorFromRect(r, MONITOR_DEFAULTTONULL) }.is_invalid()
}

/// The caret as the foreground app reports it to accessibility tools: the
/// classic Win32 caret, or the one browsers and Office expose themselves.
fn caret_via_accessibility() -> Option<RECT> {
    unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(thread, &mut info).ok()?;
        let hwnd = [info.hwndCaret, info.hwndFocus]
            .into_iter()
            .find(|h| !h.is_invalid())?;
        let mut object: Option<IAccessible> = None;
        AccessibleObjectFromWindow(
            hwnd,
            OBJID_CARET.0 as u32,
            &IAccessible::IID,
            &mut object as *mut _ as *mut _,
        )
        .ok()?;
        let (mut left, mut top, mut width, mut height) = (0, 0, 0, 0);
        // The caret itself (CHILDID_SELF), not one of its children.
        let mut own = VARIANT::default();
        (*own.Anonymous.Anonymous).vt = VT_I4;
        object?
            .accLocation(&mut left, &mut top, &mut width, &mut height, &own)
            .ok()?;
        // A hidden caret reports an empty rectangle.
        (height > 0).then(|| RECT {
            left,
            top,
            right: left + width.max(1),
            bottom: top + height,
        })
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

/// Start, Search and other shell flyouts: they live in a z-order band above
/// every normal window, so the card can't be shown over them.
pub fn is_shell_flyout(hwnd: HWND) -> bool {
    let hwnd = unsafe { GetAncestor(hwnd, GA_ROOT) };
    // Packaged apps show up as ApplicationFrameWindow; a bare CoreWindow in
    // front is a shell surface (Start, Search, Action Center, Win+V...).
    if matches!(
        win::class_name(hwnd).as_str(),
        "Windows.UI.Core.CoreWindow" | "XamlExplorerHostIslandWindow"
    ) {
        return true;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return false;
    };
    let process = unsafe { Owned::new(process) };
    let mut buf = [0u16; 260];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            *process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
    }
    .is_ok();
    if !ok {
        return false;
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]).to_ascii_lowercase();
    let exe = path.rsplit('\\').next().unwrap_or_default();
    matches!(
        exe,
        "startmenuexperiencehost.exe"
            | "searchhost.exe"
            | "searchapp.exe"
            | "searchui.exe"
            | "shellexperiencehost.exe"
    )
}

/// Whether we run with UI access (the signed install), which puts our
/// top-most windows above Start and Search.
pub fn has_ui_access() -> bool {
    static UI_ACCESS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *UI_ACCESS.get_or_init(|| unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let token = Owned::new(token);
        let mut value = 0u32;
        let mut len = 0u32;
        let ok = GetTokenInformation(
            *token,
            TokenUIAccess,
            Some(&mut value as *mut u32 as *mut _),
            std::mem::size_of::<u32>() as u32,
            &mut len,
        )
        .is_ok();
        ok && value != 0
    })
}

/// Brings `hwnd` to the foreground, so it gets the keyboard. Windows only
/// lets the app that received the last input do that; the hotkey usually
/// makes that us, and when it doesn't, an input of our own does.
pub fn take_foreground(hwnd: HWND) -> bool {
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        // A key no keyboard has: it does nothing but count as input.
        let inputs = [key(VIRTUAL_KEY(0xFF), false), key(VIRTUAL_KEY(0xFF), true)];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        SetForegroundWindow(hwnd).as_bool()
    }
}

/// The text on the clipboard, if any.
pub fn clipboard_text(owner: HWND) -> Option<String> {
    read_clipboard_text(owner)
}

/// Closes Start/Search with Esc.
pub fn close_flyout() {
    // A still-held Ctrl/Alt from the hotkey would turn this into Ctrl+Alt+Esc,
    // which Start ignores.
    release_modifiers();
    let inputs = [key(VK_ESCAPE, false), key(VK_ESCAPE, true)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn is_terminal(hwnd: HWND) -> bool {
    matches!(
        win::class_name(hwnd).as_str(),
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
        wait(Duration::from_millis(10));
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

/// Sleeps for `duration` while answering messages sent to this thread's
/// windows. After a restore we own the clipboard, and an app that copies
/// sends us WM_DESTROYCLIPBOARD and waits for the reply: a plain sleep would
/// hold its copy back until we stopped waiting for it.
fn wait(duration: Duration) {
    let deadline = Instant::now() + duration;
    loop {
        let mut msg = MSG::default();
        // Delivers sent messages; posted ones stay queued for the thread's loop.
        unsafe {
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE | PM_QS_SENDMESSAGE);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        unsafe {
            MsgWaitForMultipleObjectsEx(
                None,
                left.as_millis().max(1) as u32,
                QS_SENDMESSAGE,
                MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(0),
            );
        }
    }
}

fn open_clipboard(owner: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            return true;
        }
        wait(Duration::from_millis(15));
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

/// Puts `text` on the clipboard, in place of what was there. Whether it did.
pub fn set_clipboard_text(owner: HWND, text: &str) -> bool {
    if !open_clipboard(owner) {
        return false;
    }
    let wide: Vec<u8> = text
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let set = unsafe {
        let emptied = EmptyClipboard().is_ok();
        if emptied {
            set_data(CF_UNICODETEXT_ID, &wide);
        }
        emptied
    };
    unsafe {
        let _ = CloseClipboard();
    }
    set
}

/// A copy of the memory-backed formats on the clipboard.
///
/// Formats held as GDI or OLE objects, and anything past 32 MB in total,
/// aren't saved: restoring after a copy loses them. Windows synthesizes the
/// common ones (CF_BITMAP from CF_DIB, for instance), so in practice that
/// is rare, and the alternative is reading no selection at all.
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
