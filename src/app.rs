//! Ties everything together on the UI thread: hotkey → selection → lookup → card.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::channel;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MonitorFromRect, ScreenToClient, MONITOR_DEFAULTTONULL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::autostart;
use crate::config::{self, Config};
use crate::dictionary::{self, Definition, Lookup, NormalizeError};
use crate::hotkey;
use crate::menu::Menu;
use crate::offline;
use crate::popup::{self, Popup, Transition};
use crate::render::{Body, Target};
use crate::selection::{self, Capturer, Selection};
use crate::theme::Theme;
use crate::tray::{self, Tray};

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_SELECTION: u32 = WM_APP + 2;
const WM_APP_RESULT: u32 = WM_APP + 3;
const WM_APP_DISMISS: u32 = WM_APP + 4;
const WM_APP_WHEEL: u32 = WM_APP + 5;
const WM_APP_CAPTURE: u32 = WM_APP + 6;
const WM_APP_CLICK: u32 = WM_APP + 7;
const WM_APP_BACK: u32 = WM_APP + 8;
/// Not in the windows crate's message set (it lives with the common controls).
const WM_MOUSELEAVE: u32 = 0x02A3;

const HOTKEY_LOOKUP: i32 = 1;
const HOTKEY_ESCAPE: i32 = 2;
const HOTKEY_BACK: i32 = 3;
const HOTKEY_BACK_ALT: i32 = 4;

const TIMER_HIDE: usize = 1;
const TIMER_SETTLE: usize = 2;
const TIMER_AUTOHIDE: usize = 3;
const TIMER_TEARDOWN: usize = 4;
const TIMER_COVERED: usize = 5;
const TIMER_MENU_HIDE: usize = 6;
const TIMER_MENU_TEARDOWN: usize = 7;

/// Free the GPU resources after this long without a lookup.
const IDLE_TEARDOWN_MS: u32 = 3 * 60 * 1000;
/// Messages ("Nothing selected") close on their own after this long.
const AUTOHIDE_MS: u32 = 3500;
/// How soon after showing to check whether Start/Search covers the card.
const COVERED_CHECK_MS: u32 = 150;
/// A click that lands this soon after the tray menu closed was the one that closed it.
const MENU_REOPEN_GUARD_MS: u64 = 300;
/// Hide the window once the fade-out has played, with a little slack.
const HIDE_DONE_MS: u32 = popup::HIDE_MS as u32 + 30;
/// Shrink the hit-test region once a resize animation has played.
const SETTLE_MS: u32 = popup::RESIZE_MS as u32 + 60;
const CACHE_SIZE: usize = 64;
/// How many words "back" remembers.
const HISTORY_SIZE: usize = 50;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    // Read by the low-level mouse hook, which must not touch APP.
    static CARD_RECT: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    static MSG_HWND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    // When the card last appeared (GetTickCount64), read by the focus hook.
    static SHOWN_AT: Cell<u64> = const { Cell::new(0) };
}

struct Captured {
    id: u64,
    selection: Option<Selection>,
}

struct Looked {
    id: u64,
    word: String,
    result: Lookup,
}

/// Where a lookup's answer goes.
#[derive(Clone, Copy)]
enum Place {
    /// A new card next to the selection.
    New(RECT),
    /// The open card, in place of what it shows.
    Same,
}

pub struct App {
    config: Config,
    hwnd: HWND,
    compositor: Compositor,
    popup: Option<Popup>,
    menu: Option<Menu>,
    /// When the tray menu last started closing (GetTickCount64).
    menu_closed_at: u64,
    /// The press on the tray icon that closed the menu; its release mustn't reopen it.
    swallow_tray_release: bool,
    tray: Tray,
    taskbar_created: u32,
    capture_thread: u32,
    request: u64,
    /// Online answers, most recently used last.
    cache: Vec<(String, Lookup)>,
    /// The offline entry for the current request, if any.
    local: Option<Definition>,
    /// The word the card shows, as it was looked up.
    current: Option<String>,
    /// Words the card showed before synonyms were followed, most recent last.
    history: Vec<String>,
    /// Whether Backspace and Alt+Left are registered.
    back_keys: bool,
    hotkey_ok: bool,
    mouse_hook: Option<HHOOK>,
    focus_hook: Option<HWINEVENTHOOK>,
}

pub fn run(compositor: Compositor) -> windows::core::Result<()> {
    let (config, first_run, config_error) = match config::load() {
        Ok((config, first_run)) => (config, first_run, None),
        Err(e) => (Config::default(), false, Some(e)),
    };
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        let class = WNDCLASSW {
            lpfnWndProc: Some(main_proc),
            hInstance: instance,
            lpszClassName: w!("WinDict.Main"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let popup_class = WNDCLASSW {
            lpfnWndProc: Some(popup_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("WinDict.Popup"),
            ..Default::default()
        };
        RegisterClassW(&popup_class);
        let menu_class = WNDCLASSW {
            lpfnWndProc: Some(menu_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("WinDict.Menu"),
            ..Default::default()
        };
        RegisterClassW(&menu_class);

        // A hidden top-level window (not message-only: it must receive the
        // TaskbarCreated broadcast).
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("WinDict.Main"),
            w!("WinDict"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?;
        MSG_HWND.set(hwnd);

        let tray = Tray::new(hwnd, WM_APP_TRAY);
        let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));

        // Selection capture runs on its own thread: it sleeps while waiting
        // for keys to be released and for the clipboard to change. That
        // thread also owns the clipboard after a restore, so it must pump
        // messages (other apps send WM_DESTROYCLIPBOARD to the owner).
        let (ready, thread_id) = channel::<u32>();
        let notify = hwnd.0 as isize;
        std::thread::spawn(move || {
            let capturer = Capturer::new();
            let mut msg = MSG::default();
            // Create the message queue before announcing ourselves.
            let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
            let _ = ready.send(GetCurrentThreadId());
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.hwnd.is_invalid() && msg.message == WM_APP_CAPTURE {
                    let id = msg.wParam.0 as u64;
                    let use_ui_automation = msg.lParam.0 != 0;
                    let selection = capturer.capture(use_ui_automation);
                    post(
                        notify,
                        WM_APP_SELECTION,
                        Box::new(Captured { id, selection }),
                    );
                } else {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
        let capture_thread = thread_id.recv().unwrap_or(0);

        let mut app = App {
            config,
            hwnd,
            compositor,
            popup: None,
            menu: None,
            menu_closed_at: 0,
            swallow_tray_release: false,
            tray,
            taskbar_created,
            capture_thread,
            request: 0,
            cache: Vec::new(),
            local: None,
            current: None,
            history: Vec::new(),
            back_keys: false,
            hotkey_ok: false,
            mouse_hook: None,
            focus_hook: None,
        };
        app.register_hotkey();
        if let Some(e) = config_error {
            app.tray
                .notify("Settings not loaded, using the defaults", &e);
        } else if first_run && app.hotkey_ok {
            app.tray.notify(
                "WinDict is running",
                &format!("Select a word anywhere and press {}.", app.config.hotkey),
            );
        }
        APP.with(|a| *a.borrow_mut() = Some(app));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        APP.with(|a| {
            if let Some(app) = a.borrow_mut().take() {
                app.tray.remove();
            }
        });
    }
    Ok(())
}

/// Sends a boxed payload to the UI thread.
fn post<T>(hwnd: isize, msg: u32, payload: Box<T>) {
    let ptr = Box::into_raw(payload);
    unsafe {
        if PostMessageW(Some(HWND(hwnd as _)), msg, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    }
}

/// Runs `f` on the app state. Returns `None`, dropping the event, if the state
/// is already borrowed: that only happens when a handler pumps messages (a
/// modal loop, some COM calls), so handlers that do must release it first,
/// as the tray menu does.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

impl App {
    fn register_hotkey(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_LOOKUP);
        }
        match hotkey::parse(&self.config.hotkey) {
            Ok(hk) => {
                self.hotkey_ok = unsafe {
                    RegisterHotKey(
                        Some(self.hwnd),
                        HOTKEY_LOOKUP,
                        hk.modifiers | MOD_NOREPEAT,
                        hk.vk,
                    )
                }
                .is_ok();
                if self.hotkey_ok {
                    self.tray
                        .set_tip(&format!("WinDict - {}", self.config.hotkey));
                } else {
                    self.tray.notify(
                        "Shortcut unavailable",
                        &format!(
                            "{} is already used by another app. Choose another one in Settings.",
                            self.config.hotkey
                        ),
                    );
                }
            }
            Err(e) => {
                self.hotkey_ok = false;
                self.tray.notify("Invalid shortcut", &e);
            }
        }
    }

    fn theme(&self) -> Theme {
        Theme::current(self.config.theme)
    }

    fn ensure_popup(&mut self) -> Option<&mut Popup> {
        if self.popup.is_none() {
            let created = unsafe {
                CreateWindowExW(
                    WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    w!("WinDict.Popup"),
                    w!("WinDict"),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0)),
                    None,
                )
            };
            let hwnd = created.ok()?;
            match Popup::new(hwnd, self.compositor.clone(), self.theme()) {
                Ok(p) => self.popup = Some(p),
                Err(e) => {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    self.tray
                        .notify("WinDict couldn't draw its window", &e.message());
                    return None;
                }
            }
        }
        self.popup.as_mut()
    }

    fn ensure_menu(&mut self) -> Option<&mut Menu> {
        if self.menu.is_none() {
            let created = unsafe {
                // Never takes the focus: the taskbar's overflow flyout closes
                // as soon as something else is activated.
                CreateWindowExW(
                    WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    w!("WinDict.Menu"),
                    w!("WinDict"),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0)),
                    None,
                )
            };
            let hwnd = created.ok()?;
            match Menu::new(hwnd, self.compositor.clone(), self.theme()) {
                Ok(m) => self.menu = Some(m),
                Err(e) => {
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                    self.tray
                        .notify("WinDict couldn't draw its menu", &e.message());
                    return None;
                }
            }
        }
        self.menu.as_mut()
    }

    // -----------------------------------------------------------------------
    // Flow

    fn on_hotkey(&mut self) {
        self.request += 1;
        unsafe {
            let _ = PostThreadMessageW(
                self.capture_thread,
                WM_APP_CAPTURE,
                WPARAM(self.request as usize),
                // Sent with every request so "Reload settings" applies it.
                LPARAM(self.config.use_ui_automation as isize),
            );
        }
    }

    fn on_selection(&mut self, captured: Captured) {
        if captured.id != self.request {
            return;
        }
        self.current = None;
        self.history.clear();
        self.sync_back_keys();
        let hotkey = self.config.hotkey.clone();
        let (text, anchor) = match captured.selection {
            Some(s) => (s.text, s.anchor),
            None => (String::new(), None),
        };
        let word = match dictionary::normalize(&text) {
            Ok(word) => word,
            Err(NormalizeError::TooLong) => {
                let detail = "Select a single word or a short phrase.";
                return self.show_message(anchor, "That\u{2019}s a lot of text", detail, true);
            }
            Err(NormalizeError::Empty) => {
                let detail = format!("Select a word in any app, then press {hotkey}.");
                return self.show_message(None, "Nothing selected", &detail, true);
            }
        };
        self.lookup(word, Place::New(anchor_or_cursor(anchor)));
    }

    /// Looks `word` up and shows the answer at `place`. The bundled
    /// dictionary answers instantly; the online one fills gaps.
    fn lookup(&mut self, word: String, place: Place) {
        let (max_defs, max_syn) = (self.config.max_definitions, self.config.max_synonyms);
        self.current = Some(word.clone());
        self.local = offline::lookup(&word, max_defs, max_syn);
        let online_key = match &self.local {
            Some(def) => def.word.to_lowercase(),
            None => word.clone(),
        };
        let online = self.cached(&online_key);

        match (&self.local, online) {
            (Some(local), online) => {
                let shown = dictionary::merge(local.clone(), online.as_ref());
                let title = shown.word.clone();
                self.present_body(
                    place,
                    &title,
                    shown.phonetic.as_deref(),
                    Body::Definition(&shown),
                );
                if online.is_some() || !self.config.online_lookup {
                    return;
                }
            }
            (None, Some(online)) => return self.present(place, &word, &online),
            (None, None) if !self.config.online_lookup => {
                return self.present(place, &word, &Lookup::NotFound);
            }
            (None, None) => self.present_body(place, &word, None, Body::Loading),
        }

        let (id, notify) = (self.request, self.hwnd.0 as isize);
        std::thread::spawn(move || {
            let result = dictionary::lookup_online(&online_key, max_defs, max_syn);
            post(
                notify,
                WM_APP_RESULT,
                Box::new(Looked {
                    id,
                    word: online_key,
                    result,
                }),
            );
        });
    }

    /// A cached online answer (a definition, or a definite "not found").
    fn cached(&mut self, key: &str) -> Option<Lookup> {
        let hit = self.cache.iter().position(|(w, _)| w == key)?;
        let entry = self.cache.remove(hit);
        let result = entry.1.clone();
        self.cache.push(entry);
        Some(result)
    }

    fn on_result(&mut self, looked: Looked) {
        // Remember definite answers; retry failures (e.g. offline) next time.
        if !matches!(looked.result, Lookup::Failed(_)) {
            self.cache.retain(|(w, _)| *w != looked.word);
            self.cache
                .push((looked.word.clone(), looked.result.clone()));
            if self.cache.len() > CACHE_SIZE {
                self.cache.remove(0);
            }
        }
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        if looked.id != self.request || !visible {
            return;
        }
        match self.local.clone() {
            // Already showing the offline entry: refine it in place, if the
            // online one adds anything. Errors are irrelevant here.
            Some(local) => {
                let before = (local.phonetic.clone(), local.meanings.len());
                let merged = dictionary::merge(local, Some(&looked.result));
                if (merged.phonetic.clone(), merged.meanings.len()) != before {
                    self.update(&looked.word, &Lookup::Found(merged), Transition::Refine);
                }
            }
            None => self.update(&looked.word, &looked.result, Transition::Replace),
        }
    }

    /// The card's title, pronunciation and body for a lookup of `word`.
    fn card_for<'a>(word: &'a str, result: &'a Lookup) -> (&'a str, Option<&'a str>, Body<'a>) {
        match result {
            Lookup::Found(def) => (&def.word, def.phonetic.as_deref(), Body::Definition(def)),
            Lookup::NotFound => (
                word,
                None,
                Body::Message {
                    title: "No definition found",
                    detail: "This word isn\u{2019}t in the dictionary. Check the spelling?",
                },
            ),
            Lookup::Failed(error) => (
                word,
                None,
                Body::Message {
                    title: "Couldn\u{2019}t look that up",
                    detail: error,
                },
            ),
        }
    }

    fn present(&mut self, place: Place, word: &str, result: &Lookup) {
        let (title, phonetic, body) = Self::card_for(word, result);
        self.present_body(place, title, phonetic, body);
    }

    fn present_body(&mut self, place: Place, title: &str, phonetic: Option<&str>, body: Body) {
        match place {
            Place::New(anchor) => self.show_body(anchor, title, phonetic, body, false),
            Place::Same => {
                let back = !self.history.is_empty();
                if let Some(popup) = self.popup.as_mut() {
                    let _ = popup.set_back(back);
                }
                self.update_body(title, phonetic, body, Transition::Replace);
            }
        }
    }

    fn update(&mut self, word: &str, result: &Lookup, transition: Transition) {
        let (title, phonetic, body) = Self::card_for(word, result);
        self.update_body(title, phonetic, body, transition);
    }

    fn update_body(
        &mut self,
        title: &str,
        phonetic: Option<&str>,
        body: Body,
        transition: Transition,
    ) {
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        if popup
            .set_content(title, phonetic, body, transition)
            .is_err()
        {
            return;
        }
        CARD_RECT.set(popup.card_rect());
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SETTLE, SETTLE_MS, None);
        }
    }

    fn show_message(&mut self, anchor: Option<RECT>, title: &str, detail: &str, auto_hide: bool) {
        let anchor = anchor_or_cursor(anchor);
        self.show_body(
            anchor,
            "WinDict",
            None,
            Body::Message { title, detail },
            auto_hide,
        );
    }

    fn show_body(
        &mut self,
        anchor: RECT,
        title: &str,
        phonetic: Option<&str>,
        body: Body,
        auto_hide: bool,
    ) {
        self.dismiss_menu();
        let theme = self.theme();
        let hwnd = self.hwnd;
        let Some(popup) = self.ensure_popup() else {
            return;
        };
        if let Err(e) = popup.show(anchor, theme, title, phonetic, body) {
            self.tray
                .notify("WinDict couldn't draw its window", &e.message());
            return;
        }
        CARD_RECT.set(popup.card_rect());
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_HIDE);
            let _ = KillTimer(Some(hwnd), TIMER_TEARDOWN);
            let _ = KillTimer(Some(hwnd), TIMER_AUTOHIDE);
            if auto_hide {
                SetTimer(Some(hwnd), TIMER_AUTOHIDE, AUTOHIDE_MS, None);
            }
        }
        SHOWN_AT.set(unsafe { GetTickCount64() });
        self.install_dismiss_hooks();
        unsafe {
            SetTimer(Some(hwnd), TIMER_COVERED, COVERED_CHECK_MS, None);
        }
    }

    /// Safety net: if Start/Search still covers the card, close it.
    fn uncover(&mut self) {
        if selection::has_ui_access() {
            return;
        }
        let Some(popup) = self.popup.as_ref().filter(|p| p.visible) else {
            return;
        };
        let r = popup.card_rect();
        let center = POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        };
        let top = unsafe { GetAncestor(WindowFromPoint(center), GA_ROOT) };
        if top != popup.hwnd && selection::is_shell_flyout(top) {
            SHOWN_AT.set(unsafe { GetTickCount64() });
            selection::close_flyout();
        }
    }

    /// Shows a synonym's definition in the card, remembering the way back.
    fn follow(&mut self, word: &str) {
        let Ok(word) = dictionary::normalize(word) else {
            return;
        };
        if self.current.as_ref() == Some(&word) {
            return;
        }
        if let Some(current) = self.current.take() {
            self.history.push(current);
            if self.history.len() > HISTORY_SIZE {
                self.history.remove(0);
            }
        }
        // Answers still on their way are for the word we're leaving.
        self.request += 1;
        self.lookup(word, Place::Same);
        self.sync_back_keys();
    }

    /// Goes back to the word shown before the last synonym was followed.
    fn back(&mut self) {
        if !self.popup.as_ref().is_some_and(|p| p.visible) {
            return;
        }
        let Some(word) = self.history.pop() else {
            return;
        };
        self.request += 1;
        self.lookup(word, Place::Same);
        self.sync_back_keys();
    }

    /// Backspace and Alt+Left go back, but only while there's somewhere to
    /// go: otherwise they belong to the app underneath.
    fn sync_back_keys(&mut self) {
        let wanted = !self.history.is_empty();
        if wanted == self.back_keys {
            return;
        }
        self.back_keys = wanted;
        unsafe {
            if wanted {
                let _ =
                    RegisterHotKey(Some(self.hwnd), HOTKEY_BACK, MOD_NOREPEAT, VK_BACK.0 as u32);
                let _ = RegisterHotKey(
                    Some(self.hwnd),
                    HOTKEY_BACK_ALT,
                    MOD_ALT | MOD_NOREPEAT,
                    VK_LEFT.0 as u32,
                );
            } else {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_BACK);
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_BACK_ALT);
            }
        }
    }

    fn dismiss(&mut self) {
        self.history.clear();
        self.sync_back_keys();
        self.remove_dismiss_hooks();
        self.dismiss_menu();
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        if !popup.visible {
            return;
        }
        let _ = popup.begin_hide();
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_AUTOHIDE);
            SetTimer(Some(self.hwnd), TIMER_HIDE, HIDE_DONE_MS, None);
        }
    }

    fn on_timer(&mut self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), id);
        }
        match id {
            TIMER_AUTOHIDE => self.dismiss(),
            TIMER_COVERED => self.uncover(),
            TIMER_SETTLE => {
                if let Some(p) = self.popup.as_mut() {
                    p.settle();
                }
            }
            TIMER_MENU_HIDE => {
                if let Some(m) = self.menu.as_mut() {
                    m.finish_hide();
                }
                trim_working_set();
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_MENU_TEARDOWN, IDLE_TEARDOWN_MS, None);
                }
            }
            TIMER_MENU_TEARDOWN if self.menu.as_ref().is_some_and(|m| !m.visible) => {
                if let Some(m) = self.menu.take() {
                    let hwnd = m.hwnd;
                    drop(m);
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                trim_working_set();
            }
            TIMER_HIDE => {
                if let Some(p) = self.popup.as_mut() {
                    p.finish_hide();
                }
                trim_working_set();
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_TEARDOWN, IDLE_TEARDOWN_MS, None);
                }
            }
            TIMER_TEARDOWN if self.popup.as_ref().is_some_and(|p| !p.visible) => {
                if let Some(p) = self.popup.take() {
                    let hwnd = p.hwnd;
                    drop(p);
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
                trim_working_set();
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Light dismiss: Esc, a click outside the card, or switching apps.

    fn install_dismiss_hooks(&mut self) {
        unsafe {
            let _ = RegisterHotKey(
                Some(self.hwnd),
                HOTKEY_ESCAPE,
                MOD_NOREPEAT,
                VK_ESCAPE.0 as u32,
            );
            if self.mouse_hook.is_none() {
                self.mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0).ok();
            }
            if self.focus_hook.is_none() {
                let hook = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(focus_hook),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                self.focus_hook = (!hook.is_invalid()).then_some(hook);
            }
        }
    }

    fn remove_dismiss_hooks(&mut self) {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_ESCAPE);
            if let Some(h) = self.mouse_hook.take() {
                let _ = UnhookWindowsHookEx(h);
            }
            if let Some(h) = self.focus_hook.take() {
                let _ = UnhookWinEvent(h);
            }
        }
    }

    fn on_click(&mut self, x: i32, y: i32) {
        if self.popup.as_ref().is_some_and(|p| p.back_at(x, y)) {
            return self.back();
        }
        let Some(target) = self.popup.as_ref().and_then(|p| p.link_at(x, y)).cloned() else {
            return;
        };
        match target {
            Target::Word(word) => self.follow(&word),
            Target::Url(url) => {
                unsafe {
                    ShellExecuteW(
                        None,
                        w!("open"),
                        &HSTRING::from(url),
                        PCWSTR::null(),
                        PCWSTR::null(),
                        SW_SHOWNORMAL,
                    );
                }
                self.dismiss();
            }
        }
    }

    // -----------------------------------------------------------------------
    // Tray

    /// A button went down on the tray icon. If that's what closes the menu
    /// (the click-away dismissal gets there first), the release that follows
    /// must not open it again.
    fn on_tray_press(&mut self) {
        let closing = self.menu.as_ref().is_some_and(|m| m.visible)
            || unsafe { GetTickCount64() }.saturating_sub(self.menu_closed_at)
                < MENU_REOPEN_GUARD_MS;
        self.swallow_tray_release = closing;
    }

    fn on_tray_release(&mut self) {
        if std::mem::take(&mut self.swallow_tray_release) {
            return;
        }
        self.open_menu();
    }

    fn open_menu(&mut self) {
        if self.menu.as_ref().is_some_and(|m| m.visible) {
            return;
        }
        self.dismiss();
        let (theme, hotkey) = (self.theme(), self.config.hotkey.clone());
        let Some(menu) = self.ensure_menu() else {
            return;
        };
        let mut cursor = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        if let Err(e) = menu.show(cursor, theme, &hotkey, autostart::is_enabled()) {
            self.tray
                .notify("WinDict couldn't draw its menu", &e.message());
            return;
        }
        CARD_RECT.set(menu.card_rect());
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_MENU_HIDE);
            let _ = KillTimer(Some(self.hwnd), TIMER_MENU_TEARDOWN);
        }
        SHOWN_AT.set(unsafe { GetTickCount64() });
        self.install_dismiss_hooks();
    }

    /// Starts closing the tray menu, if it's open.
    fn dismiss_menu(&mut self) {
        let Some(menu) = self.menu.as_mut().filter(|m| m.visible) else {
            return;
        };
        let _ = menu.begin_hide();
        self.menu_closed_at = unsafe { GetTickCount64() };
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_MENU_HIDE, HIDE_DONE_MS, None);
        }
    }

    fn menu_hover(&mut self, point: Option<(i32, i32)>) {
        if let Some(menu) = self.menu.as_mut().filter(|m| m.visible) {
            let _ = menu.hover(point);
        }
    }

    fn menu_click(&mut self, x: i32, y: i32) {
        let command = self
            .menu
            .as_ref()
            .filter(|m| m.visible)
            .and_then(|m| m.command_at(x, y));
        if let Some(command) = command {
            self.run_menu_command(command);
        }
    }

    fn menu_key(&mut self, key: VIRTUAL_KEY) {
        let Some(menu) = self.menu.as_mut().filter(|m| m.visible) else {
            return;
        };
        let _ = match key {
            VK_DOWN => menu.step(true),
            VK_UP => menu.step(false),
            VK_HOME => menu.jump(false),
            VK_END => menu.jump(true),
            VK_RETURN | VK_SPACE => {
                if let Some(command) = menu.selected_command() {
                    self.run_menu_command(command);
                }
                return;
            }
            VK_ESCAPE => return self.dismiss(),
            _ => return,
        };
    }

    fn run_menu_command(&mut self, command: u32) {
        self.dismiss();
        self.on_command(command);
    }

    fn on_command(&mut self, command: u32) {
        match command {
            tray::ID_AUTOSTART => autostart::set(!autostart::is_enabled()),
            tray::ID_SETTINGS => open_settings(),
            tray::ID_RELOAD => {
                match config::load() {
                    Ok((config, _)) => self.config = config,
                    Err(e) => {
                        return self
                            .tray
                            .notify("Settings not reloaded, nothing changed", &e)
                    }
                }
                self.cache.clear();
                self.register_hotkey();
                if self.hotkey_ok {
                    self.tray.notify(
                        "Settings reloaded",
                        &format!("Look up words with {}.", self.config.hotkey),
                    );
                }
            }
            tray::ID_QUIT => unsafe {
                self.remove_dismiss_hooks();
                PostQuitMessage(0);
            },
            _ => {}
        }
    }
}

fn anchor_or_cursor(anchor: Option<RECT>) -> RECT {
    let usable = anchor.filter(|r| unsafe {
        r.right > r.left
            && r.bottom > r.top
            && r.bottom - r.top < 160
            && !MonitorFromRect(r, MONITOR_DEFAULTTONULL).is_invalid()
    });
    usable.unwrap_or_else(|| {
        let mut pt = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut pt);
        }
        // Leave room for the pointer itself.
        RECT {
            left: pt.x,
            top: pt.y - 4,
            right: pt.x + 1,
            bottom: pt.y + 18,
        }
    })
}

fn open_settings() {
    let path = HSTRING::from(config::path().as_os_str());
    unsafe {
        let result = ShellExecuteW(
            None,
            w!("open"),
            &path,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if result.0 as isize <= 32 {
            ShellExecuteW(
                None,
                w!("open"),
                w!("notepad.exe"),
                &path,
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }
}

fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

// ---------------------------------------------------------------------------
// Window and hook procedures

unsafe extern "system" fn main_proc(
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
                _ => None,
            };
            LRESULT(0)
        }
        WM_APP_SELECTION => {
            let captured = unsafe { Box::from_raw(lparam.0 as *mut Captured) };
            with_app(|app| app.on_selection(*captured));
            LRESULT(0)
        }
        WM_APP_RESULT => {
            let looked = unsafe { Box::from_raw(lparam.0 as *mut Looked) };
            with_app(|app| app.on_result(*looked));
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
            let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
            with_app(|app| app.on_click(x, y));
            LRESULT(0)
        }
        WM_APP_BACK => {
            with_app(|app| app.back());
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

unsafe extern "system" fn popup_proc(
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
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
            let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
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
        // A hand over the links.
        WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            let mut pt = POINT::default();
            let over_link =
                unsafe { GetCursorPos(&mut pt).is_ok() && ScreenToClient(hwnd, &mut pt).as_bool() }
                    && with_app(|app| {
                        app.popup
                            .as_ref()
                            .is_some_and(|p| p.link_at(pt.x, pt.y).is_some())
                    })
                    .unwrap_or(false);
            if over_link {
                unsafe {
                    if let Ok(hand) = LoadCursorW(None, IDC_HAND) {
                        SetCursor(Some(hand));
                    }
                }
                LRESULT(1)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        // We size ourselves for the target monitor's DPI already.
        WM_DPICHANGED => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Mouse messages carry client coordinates.
    let (x, y) = (lparam.0 as i16 as i32, (lparam.0 >> 16) as i16 as i32);
    match msg {
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to drop the highlight when the pointer goes.
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut track);
            }
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

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
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

unsafe extern "system" fn focus_hook(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    // Ignore the foreground change from closing Start/Search just before showing.
    if unsafe { GetTickCount64() }.saturating_sub(SHOWN_AT.get()) < 500 {
        return;
    }
    unsafe {
        let _ = PostMessageW(Some(MSG_HWND.get()), WM_APP_DISMISS, WPARAM(0), LPARAM(0));
    }
}
