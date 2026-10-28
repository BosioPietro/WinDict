//! Ties everything together on the UI thread: hotkey → selection → lookup → card.
//!
//! This file holds the state, the hotkey, the timers and the dismissal of the
//! card; the rest of `impl App` is split by flow into the submodules:
//! `search`, `lookup`, `translating`, `links`, `pinning`, `zooming`,
//! `tray_menu`, `prefs`, `updating` and `onboarding`.
//! `procs` has the window procedures, `edit` and `picking` the pure text and
//! list logic.

pub(crate) mod edit;
mod links;
mod lookup;
mod onboarding;
pub(crate) mod picking;
mod pinning;
mod prefs;
mod procs;
mod recent;
mod search;
mod state;
mod translating;
mod tray_menu;
mod updating;
mod zooming;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::autostart;
use crate::config::{self, Config, SelectionMode, Shortcut, SHORTCUTS};
use crate::dictionary::{self, Definition, Detail, Lookup, NormalizeError};
use crate::dicts;
use crate::hotkey;
use crate::i18n::{self, fill, tr};
use crate::installed;
use crate::jumplist;
use crate::lang::LangCode;
use crate::menu::{Command, Menu};
use crate::offline;
use crate::online;
use crate::popup::{self, CardButton, Popup, Transition};
use crate::render::{self, Body, Renderer, Side, Target, TranslationCard, PICKER_ROWS};
use crate::searchbox::MAX_SUGGESTIONS;
use crate::selection::{self, Capturer};
use crate::settings::{Change, Settings, Values};
use crate::speech::{Speaker, Voices};
use crate::theme::Theme;
use crate::translate::{self, Translation};
use crate::tray::{self, Tray};
use crate::ui;
use crate::update::{self, Release, Replacing};
use crate::win::{self, Window};
use edit::{Input, LineEdit, Motion};
use picking::Picking;
use procs::{
    focus_hook, intro_proc, keyboard_hook, main_proc, menu_proc, mouse_hook, popup_proc,
    settings_proc,
};
use recent::Recent;
use state::{
    Captured, Card, Corrected, Event, Events, Installing, LanguagePair, Languages, Looked, Place,
    Query, Search, Translatable, Translated, Translating, TranslationKey,
};

const MAIN_CLASS: PCWSTR = w!("WinDict.Main");
const POPUP_CLASS: PCWSTR = w!("WinDict.Popup");
const MENU_CLASS: PCWSTR = w!("WinDict.Menu");
const SETTINGS_CLASS: PCWSTR = w!("WinDict.Settings");
const INTRO_CLASS: PCWSTR = w!("WinDict.Intro");
/// The card and the menu: drawn by the compositor, above everything, and
/// never activated by a click.
const FLYOUT_EX_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(
    WS_EX_NOREDIRECTIONBITMAP.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0 | WS_EX_NOACTIVATE.0,
);

const WM_APP_TRAY: u32 = WM_APP + 1;
/// Something arrived in the app's [`Events`] channel.
const WM_APP_EVENT: u32 = WM_APP + 2;
const WM_APP_DISMISS: u32 = WM_APP + 4;
const WM_APP_WHEEL: u32 = WM_APP + 5;
/// Read the selection: the request in `wparam`, and how in `lparam`, as
/// `CAPTURE_*` flags.
const WM_APP_CAPTURE: u32 = WM_APP + 6;
const CAPTURE_UI_AUTOMATION: isize = 1;
const CAPTURE_SKIP_CLIPBOARD: isize = 2;
const WM_APP_CLICK: u32 = WM_APP + 7;
const WM_APP_BACK: u32 = WM_APP + 8;
const WM_APP_CLOSE_SETTINGS: u32 = WM_APP + 9;
/// A key the keyboard hook caught while a shortcut is recorded: the key in
/// `wparam`, the modifiers held after it in the low word of `lparam`, and
/// whether it went down in the next bit.
const WM_APP_RECORD_KEY: u32 = WM_APP + 10;
/// A card's window was asked to close: the window in `wparam`.
const WM_APP_CLOSE_CARD: u32 = WM_APP + 11;
/// Enter on a card's pin button: the window in `wparam`.
const WM_APP_PIN: u32 = WM_APP + 12;
/// Ctrl and the mouse wheel on a card: the wheel's delta in `wparam`, the
/// window in `lparam`.
const WM_APP_ZOOM: u32 = WM_APP + 13;
/// The introduction was asked to close.
const WM_APP_CLOSE_INTRO: u32 = WM_APP + 14;
/// A task in the Jump List: the shortcut whose box to open in `wparam`, as
/// [`task_param`] makes it. Posted by the WinDict started for it.
const WM_APP_TASK: u32 = WM_APP + 15;

const HOTKEY_LOOKUP: i32 = 1;
const HOTKEY_ESCAPE: i32 = 2;
const HOTKEY_BACK: i32 = 3;
const HOTKEY_BACK_ALT: i32 = 4;
const HOTKEY_NEXT_LINK: i32 = 5;
const HOTKEY_PREV_LINK: i32 = 6;
const HOTKEY_OPEN_LINK: i32 = 7;
const HOTKEY_TRANSLATE: i32 = 8;
const HOTKEY_SCROLL_UP: i32 = 9;
const HOTKEY_SCROLL_DOWN: i32 = 10;
const HOTKEY_COPY: i32 = 11;
const HOTKEY_ZOOM_IN: i32 = 12;
const HOTKEY_ZOOM_IN_PAD: i32 = 13;
const HOTKEY_ZOOM_OUT: i32 = 14;
const HOTKEY_ZOOM_OUT_PAD: i32 = 15;
const HOTKEY_ZOOM_RESET: i32 = 16;
const HOTKEY_ZOOM_RESET_PAD: i32 = 17;
const HOTKEY_TRANSLATE_SELECTION: i32 = 18;
const HOTKEY_MENU_UP: i32 = 19;
const HOTKEY_MENU_DOWN: i32 = 20;
const HOTKEY_MENU_HOME: i32 = 21;
const HOTKEY_MENU_END: i32 = 22;
const HOTKEY_MENU_ENTER: i32 = 23;
const HOTKEY_MENU_SPACE: i32 = 24;

/// The id `shortcut` is registered with.
fn hotkey_id(shortcut: Shortcut) -> i32 {
    match shortcut {
        Shortcut::Lookup => HOTKEY_LOOKUP,
        Shortcut::Translate => HOTKEY_TRANSLATE_SELECTION,
    }
}

/// Hotkeys registered only while the card has a use for them, as (id,
/// modifiers, key).
type Keys = &'static [(i32, HOT_KEY_MODIFIERS, VIRTUAL_KEY)];

/// One of those sets of [`Keys`], and whether it's registered.
struct KeySet {
    keys: Keys,
    on: bool,
}

impl KeySet {
    fn new(keys: Keys) -> KeySet {
        KeySet { keys, on: false }
    }

    /// Registers the keys on `hwnd`, or unregisters them, as `wanted`.
    fn set(&mut self, hwnd: HWND, wanted: bool) {
        if self.on == wanted {
            return;
        }
        self.on = wanted;
        for &(id, modifiers, vk) in self.keys {
            if wanted {
                let _ = win::register_hotkey(hwnd, id, modifiers, vk.0 as u32);
            } else {
                win::unregister_hotkey(hwnd, id);
            }
        }
    }
}
const BACK_KEYS: Keys = &[
    (HOTKEY_BACK, MOD_NOREPEAT, VK_BACK),
    (
        HOTKEY_BACK_ALT,
        HOT_KEY_MODIFIERS(MOD_ALT.0 | MOD_NOREPEAT.0),
        VK_LEFT,
    ),
];
/// Held down, Tab keeps moving, as it does everywhere else.
const LINK_KEYS: Keys = &[
    (HOTKEY_NEXT_LINK, HOT_KEY_MODIFIERS(0), VK_TAB),
    (HOTKEY_PREV_LINK, MOD_SHIFT, VK_TAB),
];
/// Held down, Up and Down keep scrolling.
const SCROLL_KEYS: Keys = &[
    (HOTKEY_SCROLL_UP, HOT_KEY_MODIFIERS(0), VK_UP),
    (HOTKEY_SCROLL_DOWN, HOT_KEY_MODIFIERS(0), VK_DOWN),
];
/// The tray menu's, which never takes the keyboard: held down, Up and
/// Down keep moving.
const MENU_KEYS: Keys = &[
    (HOTKEY_MENU_UP, HOT_KEY_MODIFIERS(0), VK_UP),
    (HOTKEY_MENU_DOWN, HOT_KEY_MODIFIERS(0), VK_DOWN),
    (HOTKEY_MENU_HOME, MOD_NOREPEAT, VK_HOME),
    (HOTKEY_MENU_END, MOD_NOREPEAT, VK_END),
    (HOTKEY_MENU_ENTER, MOD_NOREPEAT, VK_RETURN),
    (HOTKEY_MENU_SPACE, MOD_NOREPEAT, VK_SPACE),
];

/// The key a hotkey of [`MENU_KEYS`] is for.
fn menu_key_of(id: i32) -> Option<VIRTUAL_KEY> {
    MENU_KEYS.iter().find(|k| k.0 == id).map(|k| k.2)
}
const OPEN_KEY: Keys = &[(HOTKEY_OPEN_LINK, MOD_NOREPEAT, VK_RETURN)];
const TRANSLATE_KEY: Keys = &[(
    HOTKEY_TRANSLATE,
    HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_NOREPEAT.0),
    VIRTUAL_KEY(b'T' as u16),
)];
const COPY_KEY: Keys = &[(
    HOTKEY_COPY,
    HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_NOREPEAT.0),
    VIRTUAL_KEY(b'C' as u16),
)];
/// Held down, Ctrl+plus and Ctrl+minus keep zooming.
const ZOOM_KEYS: Keys = &[
    (HOTKEY_ZOOM_IN, MOD_CONTROL, VK_OEM_PLUS),
    (HOTKEY_ZOOM_IN_PAD, MOD_CONTROL, VK_ADD),
    (HOTKEY_ZOOM_OUT, MOD_CONTROL, VK_OEM_MINUS),
    (HOTKEY_ZOOM_OUT_PAD, MOD_CONTROL, VK_SUBTRACT),
    (
        HOTKEY_ZOOM_RESET,
        HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_NOREPEAT.0),
        VIRTUAL_KEY(b'0' as u16),
    ),
    (
        HOTKEY_ZOOM_RESET_PAD,
        HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_NOREPEAT.0),
        VK_NUMPAD0,
    ),
];

const TIMER_HIDE: usize = 1;
const TIMER_SETTLE: usize = 2;
const TIMER_AUTOHIDE: usize = 3;
const TIMER_TEARDOWN: usize = 4;
const TIMER_COVERED: usize = 5;
const TIMER_MENU_HIDE: usize = 6;
const TIMER_MENU_TEARDOWN: usize = 7;
const TIMER_SETTINGS_HIDE: usize = 8;
const TIMER_TRANSLATING: usize = 9;
const TIMER_UPDATE_CHECK: usize = 10;
const TIMER_CLOSED: usize = 11;
const TIMER_INTRO_HIDE: usize = 12;

/// Free the GPU resources after this long without a lookup.
const IDLE_TEARDOWN_MS: u32 = 3 * 60 * 1000;
/// Messages ("That’s a lot of text") close on their own after this long.
const AUTOHIDE_MS: u32 = 3500;
/// How soon after showing to check whether Start/Search covers the card.
const COVERED_CHECK_MS: u32 = 150;
/// Foreground changes this soon after the card appears are from closing
/// Start/Search for it, not the user switching apps.
const FOREGROUND_GRACE_MS: u64 = 500;
/// A click that lands this soon after the tray menu closed was the one that closed it.
const MENU_REOPEN_GUARD_MS: u64 = 300;
/// Hide the window once the fade-out has played, with a little slack.
const HIDE_DONE_MS: u32 = ui::HIDE_MS as u32 + 30;
/// Shrink the hit-test region once a resize animation has played.
const SETTLE_MS: u32 = ui::RESIZE_MS as u32 + 60;
/// How long the card waits for a translation before showing it on its way.
const TRANSLATING_WAIT_MS: u32 = 400;
const CACHE_SIZE: usize = 64;
const TRANSLATION_CACHE_SIZE: usize = 32;
/// How many words "back" remembers.
const HISTORY_SIZE: usize = 50;
/// Suggestions start with this many letters typed: fewer match too much.
const SUGGEST_AFTER: usize = 2;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    // Read by the low-level mouse hook, which must not touch APP.
    static CARD_RECT: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    static MSG_HWND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    // The message Explorer broadcasts when it restarts, outside APP so it's
    // known even while APP is in use.
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    // Messages to the main window that came while APP was in use, posted
    // again once it's free.
    static MISSED: RefCell<Vec<(u32, WPARAM, LPARAM)>> = const { RefCell::new(Vec::new()) };
    // When the card last appeared (win::now_ms), read by the focus hook.
    static SHOWN_AT: Cell<u64> = const { Cell::new(0) };
    // Where the pointer last moved over the card, on screen (GetMessagePos).
    static POINTER_AT: Cell<Option<u32>> = const { Cell::new(None) };
    // The card the left button went down in, off the scrollbar: only there
    // does it coming up click.
    static PRESSED_IN: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    // The settings window while it records a shortcut, read by the keyboard hook.
    static RECORDING_IN: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    // The keys the keyboard hook kept from Windows that are still down, by
    // virtual key, so that their repeats and releases are kept too.
    static SWALLOWED: Cell<[u64; 4]> = const { Cell::new([0; 4]) };
}

pub struct App {
    config: Config,
    hwnd: HWND,
    compositor: Compositor,
    /// What every window draws with, made when the first one's drawn.
    renderer: Option<Rc<Renderer>>,
    /// The card that opens for a lookup and closes on a click away, and
    /// while a pinned card's window is being handled, that one.
    card: Card,
    /// The cards pinned open, each in its own window.
    pinned: Vec<Card>,
    /// Cards unpinned or closed, fading out before their windows go.
    closing: Vec<Popup>,
    menu: Option<Menu>,
    /// The settings window, while it's open.
    settings: Option<Settings>,
    /// The introduction, while it's open.
    intro: Option<crate::intro::Intro>,
    /// When the tray menu last started closing (win::now_ms).
    menu_closed_at: u64,
    /// The press on the tray icon that closed the menu; its release mustn't reopen it.
    swallow_tray_release: bool,
    tray: Tray,
    capture_thread: u32,
    /// What the capture and lookup threads sent, and how they send it.
    inbox: Receiver<Event>,
    events: Events,
    /// The last request's id: each lookup, capture and translation has its own.
    requests: u64,
    /// Online answers: definitions, and definite "not found"s.
    cache: Recent<Query, Lookup>,
    translations: Recent<TranslationKey, Translation>,
    /// The voices installed, once a card has needed them, until it's gone.
    voices: Option<Voices>,
    speaker: Speaker,
    /// Ctrl+T.
    translate_key: KeySet,
    /// Backspace and Alt+Left.
    back_keys: KeySet,
    /// Tab and Shift+Tab.
    link_keys: KeySet,
    /// Enter.
    open_key: KeySet,
    /// Up and Down.
    scroll_keys: KeySet,
    /// Ctrl+C.
    copy_key: KeySet,
    /// The tray menu's arrows, Home, End, Enter and Space.
    menu_keys: KeySet,
    /// Ctrl+plus, Ctrl+minus and Ctrl+0.
    zoom_keys: KeySet,
    /// How far the wheel has turned with Ctrl held, towards the next step
    /// of zoom.
    zoom_wheel: i32,
    /// The dictionaries being downloaded, one at most for each language.
    installing: Vec<Installing>,
    /// A newer version, once one has been found.
    update: Option<Release>,
    /// Whether it's being installed.
    updating: bool,
    hotkey_ok: bool,
    mouse_hook: Option<win::Hook>,
    focus_hook: Option<win::EventHook>,
    /// Catches keys before Windows does, while a shortcut is recorded.
    keyboard_hook: Option<win::Hook>,
}

/// Hands `task`, from the Jump List, to the WinDict running already.
pub fn forward(task: Shortcut) {
    unsafe {
        // The running copy may be starting still, its window not made yet.
        let mut tries = 0;
        let hwnd = loop {
            match FindWindowW(MAIN_CLASS, None) {
                Ok(hwnd) => break hwnd,
                Err(_) if tries < 20 => tries += 1,
                Err(_) => return,
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        // Started from Start, this process may bring a window to the front:
        // the box it opens is the running one's.
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid);
        let _ = PostMessageW(Some(hwnd), WM_APP_TASK, task_param(task), LPARAM(0));
    }
}

fn task_param(task: Shortcut) -> WPARAM {
    WPARAM(match task {
        Shortcut::Lookup => 0,
        Shortcut::Translate => 1,
    })
}

/// `task` is what the Jump List started this process for, if it did.
pub fn run(
    compositor: Compositor,
    replacing: Option<Replacing>,
    task: Option<Shortcut>,
) -> windows::core::Result<()> {
    // A first run is told by `onboarded` instead.
    let (config, config_error) = match config::load() {
        Ok((config, _first_run)) => (config, None),
        Err(e) => (Config::default(), Some(e)),
    };
    i18n::set(config.ui_language);
    ui::set_motion(config.animated());
    autostart::sync();
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        register_classes(instance)?;

        // A hidden top-level window (not message-only: it must receive the
        // TaskbarCreated broadcast).
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            MAIN_CLASS,
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
        TASKBAR_CREATED.set(taskbar_created);
        // Explorer runs without administrator rights: were WinDict running
        // with them, Windows would keep the broadcast from it.
        if taskbar_created != 0 {
            let _ = ChangeWindowMessageFilterEx(hwnd, taskbar_created, MSGFLT_ALLOW, None);
        }
        // Nor the Jump List's tasks from a WinDict without UI access.
        let _ = ChangeWindowMessageFilterEx(hwnd, WM_APP_TASK, MSGFLT_ALLOW, None);
        let _ = jumplist::set();

        let (tx, inbox) = channel();
        let events = Events {
            tx,
            hwnd: hwnd.0 as isize,
        };
        let capture_thread = spawn_capture_thread(events.clone());

        let mut app = App {
            config,
            hwnd,
            compositor,
            renderer: None,
            card: Card::default(),
            pinned: Vec::new(),
            closing: Vec::new(),
            menu: None,
            settings: None,
            intro: None,
            menu_closed_at: 0,
            swallow_tray_release: false,
            tray,
            capture_thread,
            inbox,
            events,
            requests: 0,
            cache: Recent::new(CACHE_SIZE),
            translations: Recent::new(TRANSLATION_CACHE_SIZE),
            voices: None,
            speaker: Speaker::default(),
            translate_key: KeySet::new(TRANSLATE_KEY),
            back_keys: KeySet::new(BACK_KEYS),
            link_keys: KeySet::new(LINK_KEYS),
            open_key: KeySet::new(OPEN_KEY),
            scroll_keys: KeySet::new(SCROLL_KEYS),
            copy_key: KeySet::new(COPY_KEY),
            menu_keys: KeySet::new(MENU_KEYS),
            zoom_keys: KeySet::new(ZOOM_KEYS),
            zoom_wheel: 0,
            installing: Vec::new(),
            update: None,
            updating: false,
            hotkey_ok: false,
            mouse_hook: None,
            focus_hook: None,
            keyboard_hook: None,
        };
        app.register_hotkeys();
        let dictionary_error = app.load_dictionaries();
        app.schedule_update_check(updating::FIRST_CHECK_MS);
        // However the new version came: from the menu, the setup or WinGet.
        let version = env!("CARGO_PKG_VERSION");
        let updated = replacing.is_some() || app.config.version != version;
        if app.config.version != version && config_error.is_none() {
            app.save_setting(|config| config.version = version.into());
        }
        if let Some(replacing) = replacing {
            std::thread::spawn(move || replacing.remove_old());
        }
        if let Some(e) = config_error {
            app.tray.notify(tr().settings_not_loaded, &e);
        } else if let Some(e) = dictionary_error {
            app.tray.notify(tr().dictionary_not_loaded, &e);
        } else if let Some(on) = app.config.ui_access_asked {
            app.after_ui_access(on);
        } else if updated {
            app.tray.notify(
                tr().updated,
                &fill(
                    tr().updated_detail,
                    &[("version", &env!("CARGO_PKG_VERSION"))],
                ),
            );
        } else if !app.config.onboarded {
            app.start_onboarding();
        }
        APP.with(|a| *a.borrow_mut() = Some(app));
        if let Some(task) = task {
            let _ = PostMessageW(Some(hwnd), WM_APP_TASK, task_param(task), LPARAM(0));
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        APP.with(|a| {
            if let Some(mut app) = a.borrow_mut().take() {
                app.tray.remove();
            }
        });
    }
    Ok(())
}

/// Registers the window classes of the main, card, menu, settings and
/// introduction windows.
unsafe fn register_classes(instance: HINSTANCE) -> windows::core::Result<()> {
    let arrow = LoadCursorW(None, IDC_ARROW)?;
    let icon = tray::window_icon(instance);
    let classes: [(PCWSTR, WNDPROC, HCURSOR, HICON); 5] = [
        (
            MAIN_CLASS,
            Some(main_proc),
            HCURSOR::default(),
            HICON::default(),
        ),
        // Pinned, the card has a taskbar button, which shows the icon.
        (POPUP_CLASS, Some(popup_proc), arrow, icon),
        (MENU_CLASS, Some(menu_proc), arrow, HICON::default()),
        (SETTINGS_CLASS, Some(settings_proc), arrow, icon),
        (INTRO_CLASS, Some(intro_proc), arrow, icon),
    ];
    for (name, proc, cursor, icon) in classes {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: proc,
            hInstance: instance,
            hCursor: cursor,
            hIcon: icon,
            lpszClassName: name,
            ..Default::default()
        });
    }
    Ok(())
}

/// Starts the thread that captures selections, and returns its id.
///
/// Selection capture runs on its own thread: it sleeps while waiting
/// for keys to be released and for the clipboard to change. That
/// thread also owns the clipboard after a restore, so it must pump
/// messages (other apps send WM_DESTROYCLIPBOARD to the owner).
fn spawn_capture_thread(events: Events) -> u32 {
    let (ready, thread_id) = channel::<u32>();
    std::thread::spawn(move || unsafe {
        let capturer = Capturer::new();
        let mut msg = MSG::default();
        // Create the message queue before announcing ourselves.
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let _ = ready.send(GetCurrentThreadId());
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_APP_CAPTURE {
                let id = msg.wParam.0 as u64;
                let use_ui_automation = msg.lParam.0 & CAPTURE_UI_AUTOMATION != 0;
                let use_clipboard = msg.lParam.0 & CAPTURE_SKIP_CLIPBOARD == 0;
                let selection = capturer.capture(use_ui_automation, use_clipboard, || {
                    events.send(Event::Copying(id))
                });
                events.send(Event::Captured(Captured { id, selection }));
            } else {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
    thread_id.recv().unwrap_or(0)
}

/// Runs `f` on the app state. Returns `None`, dropping the event, if the state
/// is already borrowed: that only happens when a handler pumps messages (a
/// modal loop, some COM calls, a message sent to one of WinDict's windows).
/// What the main window missed meanwhile is posted again once it's free.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let r = APP.with(|a| {
        let mut a = a.try_borrow_mut().ok()?;
        let app = a.as_mut()?;
        let r = f(app);
        if render::take_device_lost() {
            app.recover_device();
        }
        Some(r)
    });
    if r.is_some() {
        for (msg, wparam, lparam) in MISSED.with(|m| m.take()) {
            unsafe {
                let _ = PostMessageW(Some(MSG_HWND.get()), msg, wparam, lparam);
            }
        }
    }
    r
}

/// Whether the app state is in use, by a handler further up the stack.
fn app_busy() -> bool {
    APP.with(|a| a.try_borrow_mut().is_err())
}

impl App {
    /// The card, while it's showing.
    fn visible_popup(&self) -> Option<&Popup> {
        self.card.popup.as_ref().filter(|p| p.visible)
    }

    fn visible_popup_mut(&mut self) -> Option<&mut Popup> {
        self.card.popup.as_mut().filter(|p| p.visible)
    }

    /// Registers the shortcuts, saying in a notification if one can't be.
    fn register_hotkeys(&mut self) {
        for shortcut in SHORTCUTS {
            self.register_hotkey(shortcut);
        }
    }

    /// Registers `shortcut`, saying in a notification if it can't be.
    fn register_hotkey(&mut self, shortcut: Shortcut) {
        match self.bind_hotkey(shortcut) {
            Ok(()) => {}
            Err(Unbound::Taken | Unbound::Clash) => self.tray.notify(
                tr().shortcut_unavailable,
                &fill(
                    tr().shortcut_unavailable_detail,
                    &[("hotkey", &self.config.hotkey(shortcut))],
                ),
            ),
            Err(Unbound::Invalid(e)) => self.tray.notify(tr().shortcut_invalid, &e),
        }
    }

    fn bind_hotkey(&mut self, shortcut: Shortcut) -> Result<(), Unbound> {
        let id = hotkey_id(shortcut);
        win::unregister_hotkey(self.hwnd, id);
        if shortcut == Shortcut::Lookup {
            self.hotkey_ok = false;
        }
        let hk = hotkey::parse(self.config.hotkey(shortcut)).map_err(Unbound::Invalid)?;
        let modifiers = hk.modifiers | MOD_NOREPEAT;
        win::register_hotkey(self.hwnd, id, modifiers, hk.vk).map_err(|_| Unbound::Taken)?;
        if shortcut == Shortcut::Lookup {
            self.hotkey_ok = true;
            self.sync_tip();
        }
        Ok(())
    }

    /// Lets go of every shortcut, as one is recorded, so that none fires
    /// as it's pressed.
    fn unregister_hotkeys(&mut self) {
        for shortcut in SHORTCUTS {
            win::unregister_hotkey(self.hwnd, hotkey_id(shortcut));
        }
    }

    fn theme(&self) -> Theme {
        Theme::current(self.config.theme)
    }

    /// What every window draws with, made the first time it's asked for.
    fn renderer(&mut self) -> windows::core::Result<Rc<Renderer>> {
        if let Some(renderer) = &self.renderer {
            return Ok(renderer.clone());
        }
        let renderer = Rc::new(Renderer::new(&self.compositor)?);
        self.renderer = Some(renderer.clone());
        Ok(renderer)
    }

    fn ensure_popup(&mut self) -> Option<&mut Popup> {
        if self.card.popup.is_none() {
            let window =
                Window::create(POPUP_CLASS, w!("WinDict"), FLYOUT_EX_STYLE, WS_POPUP).ok()?;
            let zoom = self.config.zoom_factor();
            let popup = self.renderer().and_then(|renderer| {
                Popup::new(
                    window,
                    self.compositor.clone(),
                    renderer,
                    self.theme(),
                    zoom,
                )
            });
            match popup {
                Ok(p) => self.card.popup = Some(p),
                Err(e) => {
                    self.tray.notify(tr().draw_window_failed, &e.message());
                    return None;
                }
            }
        }
        self.card.popup.as_mut()
    }

    fn ensure_menu(&mut self) -> Option<&mut Menu> {
        if self.menu.is_none() {
            // Never takes the focus: the taskbar's overflow flyout closes
            // as soon as something else is activated.
            let window =
                Window::create(MENU_CLASS, w!("WinDict"), FLYOUT_EX_STYLE, WS_POPUP).ok()?;
            let menu = self.renderer().and_then(|renderer| {
                Menu::new(window, self.compositor.clone(), renderer, self.theme())
            });
            match menu {
                Ok(m) => self.menu = Some(m),
                Err(e) => {
                    self.tray.notify(tr().draw_menu_failed, &e.message());
                    return None;
                }
            }
        }
        self.menu.as_mut()
    }

    fn ensure_settings(&mut self, theme: Theme, values: Values) -> Option<&mut Settings> {
        if self.settings.is_none() {
            let title = HSTRING::from(tr().settings_window_title);
            let window = Window::create(
                SETTINGS_CLASS,
                PCWSTR(title.as_ptr()),
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
                WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            )
            .ok()?;
            let settings = self.renderer().and_then(|renderer| {
                Settings::new(window, self.compositor.clone(), renderer, theme, values)
            });
            match settings {
                Ok(s) => self.settings = Some(s),
                Err(e) => {
                    self.tray.notify(tr().draw_settings_failed, &e.message());
                    return None;
                }
            }
        }
        self.settings.as_mut()
    }

    // -----------------------------------------------------------------------
    // Flow

    /// A shortcut was pressed: the selection is read, to look it up or,
    /// with the translation shortcut, to translate it.
    fn on_hotkey(&mut self, shortcut: Shortcut) {
        self.new_request();
        // The card has the keyboard (a search): there's no selection to
        // read, so open a fresh search box, or close the one that's open
        // for the same shortcut.
        let foreground = unsafe { GetForegroundWindow() };
        let focused = self
            .card
            .popup
            .as_ref()
            .is_some_and(|p| p.visible && foreground == p.hwnd);
        if focused {
            let purpose = self.card.search.as_ref().map(|s| s.purpose);
            return if purpose == Some(shortcut) {
                self.dismiss()
            } else {
                self.open_search(shortcut, None)
            };
        }
        // Nor is there in a pinned card.
        if self.is_pinned(foreground) {
            return self.open_search(shortcut, None);
        }
        self.card.to_translate = shortcut == Shortcut::Translate;
        // The capture may press Ctrl+C itself, which is the app's to read.
        self.copy_key.set(self.hwnd, false);
        self.card.capturing = true;
        unsafe {
            let _ = PostThreadMessageW(
                self.capture_thread,
                WM_APP_CAPTURE,
                WPARAM(self.card.request as usize),
                // Sent with every request so "Reload settings" applies them.
                LPARAM(capture_flags(&self.config)),
            );
        }
    }

    /// Handles what the capture and lookup threads sent.
    fn on_events(&mut self) {
        while let Ok(event) = self.inbox.try_recv() {
            match event {
                Event::Copying(id) => self.on_copying(id),
                Event::Captured(captured) => self.on_selection(captured),
                // The card waiting on it may be pinned.
                Event::Looked(looked) => self.with_request(looked.id, |app| app.on_result(looked)),
                Event::Corrected(c) => self.with_request(c.id, |app| app.on_corrected(c)),
                Event::Words(id, words) => self.on_words(id, words),
                Event::Installing(language, percent) => self.on_installing(language, percent),
                Event::Installed(language, result) => self.on_installed(language, result),
                Event::Translated(translated) => {
                    self.with_request(translated.id, |app| app.on_translated(translated))
                }
                Event::UpdateChecked(result) => self.on_update_checked(result),
                Event::Updated(result) => self.on_updated(result),
                Event::UiAccess(error) => self.on_ui_access_failed(error),
            }
        }
    }

    /// The selection is being copied, which can take up to a few hundred ms
    /// (or never come, when nothing is selected): open the card on its way,
    /// at the pointer, where a copied word would open it too.
    fn on_copying(&mut self, id: u64) {
        if id != self.card.request {
            return;
        }
        self.card.current = None;
        self.card.search = None;
        self.card.history.clear();
        self.card.clear_content();
        self.sync_back_keys();
        let status = match self.card.to_translate {
            true => tr().translating,
            false => tr().looking_up,
        };
        self.show_body(
            anchor_or_cursor(None),
            status,
            None,
            Body::Loading { status: true },
            false,
        );
        self.card.copying = self.visible_popup().is_some();
    }

    fn on_selection(&mut self, captured: Captured) {
        if captured.id != self.card.request {
            return;
        }
        self.card.capturing = false;
        // The card opened on its way turns into the answer in place, unless
        // it was closed meanwhile.
        let in_place = std::mem::take(&mut self.card.copying);
        let to_translate = std::mem::take(&mut self.card.to_translate);
        let visible = self.visible_popup().is_some();
        if in_place && !visible {
            return;
        }
        self.card.current = None;
        self.card.search = None;
        self.card.history.clear();
        self.card.translating = None;
        self.sync_back_keys();
        let (text, anchor) = match captured.selection {
            Some(s) => (s.text, s.anchor),
            None => (String::new(), None),
        };
        if to_translate {
            return self.translate_selection(&text, anchor, in_place);
        }
        let word = match dictionary::normalize(&text) {
            Ok(word) => word,
            Err(NormalizeError::TooLong) => {
                return self.show_message(anchor, tr().too_long, tr().too_long_detail, true);
            }
            Err(NormalizeError::Empty) => return self.open_search(Shortcut::Lookup, anchor),
        };
        let query = Query {
            word,
            language: None,
        };
        let place = if in_place {
            Place::Same(Transition::Replace)
        } else {
            Place::New(anchor_or_cursor(anchor))
        };
        self.lookup(query, place);
    }

    /// The pointer over the card: over the list of languages, it moves the
    /// highlight along.
    fn hover_card(&mut self, point: Option<(i32, i32)>) {
        let Some(popup) = self.card.popup.as_mut() else {
            return;
        };
        let _ = popup.hover(point);
        let Some(row) = point.and_then(|(x, y)| popup.list_row_at(x, y)) else {
            return;
        };
        let Some((picking, _)) = self.picking() else {
            return;
        };
        let code = picking.matches.get(row).copied();
        if code.is_some_and(|code| picking.highlight(code)) {
            let highlight = picking.highlight;
            if let Some(popup) = self.card.popup.as_mut() {
                let _ = popup.highlight_list(highlight);
            }
        }
    }

    /// The left button down on the card: on the scrollbar, it grabs the
    /// thumb or pages. Returns whether it was on it.
    fn press_card(&mut self, x: i32, y: i32) -> bool {
        self.card
            .popup
            .as_mut()
            .is_some_and(|p| p.press(x, y).unwrap_or(false))
    }

    /// The left button up on the card. Returns whether it had gone down on
    /// the scrollbar, and so isn't a click.
    fn release_card(&mut self) -> bool {
        self.card
            .popup
            .as_mut()
            .is_some_and(|p| p.release().unwrap_or(false))
    }

    /// The mouse wheel: scrolls the list of languages, when it's over it,
    /// or else the card.
    fn on_wheel(&mut self, delta: i32) {
        let Some(popup) = self.card.popup.as_mut() else {
            return;
        };
        if !popup.scroll_list(delta).unwrap_or(false) {
            let _ = popup.scroll_by(delta);
            return;
        }
        // The highlight moves to the row the pointer's on now.
        let pointer = popup.pointer();
        self.hover_card(pointer);
    }

    /// Up or Down: scrolls the card.
    fn scroll_line(&mut self, down: bool) {
        if let Some(popup) = self.visible_popup_mut() {
            let _ = popup.scroll_line(down);
        }
    }

    /// Esc: closes the list of languages, if it's open, or else the card,
    /// unless it's pinned.
    fn escape(&mut self) {
        if self.shut_picker().is_none() {
            self.dismiss();
        }
    }

    /// Gives the card the keyboard, to type into, until it closes and hands
    /// it back.
    fn take_keyboard(&mut self) {
        let Some(hwnd) = self.visible_popup().map(|p| p.hwnd) else {
            return;
        };
        let previous = unsafe { GetForegroundWindow() };
        if previous != hwnd && selection::take_foreground(hwnd) && self.card.return_focus.is_none()
        {
            self.card.return_focus = Some(previous);
        }
    }

    fn dismiss(&mut self) {
        // A pinned card closes only when told to.
        if self.card.pinned {
            return;
        }
        self.card.history.clear();
        self.card.translating = None;
        self.sync_back_keys();
        self.set_link_keys(false, false);
        self.scroll_keys.set(self.hwnd, false);
        self.translate_key.set(self.hwnd, false);
        self.copy_key.set(self.hwnd, false);
        self.zoom_keys.set(self.hwnd, false);
        self.remove_dismiss_hooks();
        self.dismiss_menu();
        self.card.search = None;
        // Hand the keyboard back to the app the search box took it from,
        // unless the user has moved on to another one already.
        if let Some(previous) = self.card.return_focus.take() {
            let focused = self
                .card
                .popup
                .as_ref()
                .is_some_and(|p| unsafe { GetForegroundWindow() } == p.hwnd);
            if focused {
                unsafe {
                    let _ = SetForegroundWindow(previous);
                }
            }
        }
        let Some(popup) = self.card.popup.as_mut() else {
            return;
        };
        if !popup.visible {
            return;
        }
        let _ = popup.begin_hide();
        win::kill_timer(self.hwnd, TIMER_AUTOHIDE);
        win::set_timer(self.hwnd, TIMER_HIDE, HIDE_DONE_MS);
    }

    fn on_timer(&mut self, id: usize) {
        win::kill_timer(self.hwnd, id);
        match id {
            TIMER_AUTOHIDE => self.dismiss(),
            TIMER_COVERED => self.uncover(),
            TIMER_UPDATE_CHECK => self.check_for_update(),
            TIMER_CLOSED => {
                self.closing.clear();
                trim_working_set();
            }
            TIMER_MENU_HIDE => {
                if let Some(m) = self.menu.as_mut() {
                    m.finish_hide();
                }
                trim_working_set();
                win::set_timer(self.hwnd, TIMER_MENU_TEARDOWN, IDLE_TEARDOWN_MS);
            }
            TIMER_MENU_TEARDOWN if self.menu.as_ref().is_some_and(|m| !m.visible) => {
                self.menu = None;
                trim_working_set();
            }
            TIMER_HIDE => {
                if let Some(p) = self.card.popup.as_mut() {
                    p.finish_hide();
                }
                trim_working_set();
                win::set_timer(self.hwnd, TIMER_TEARDOWN, IDLE_TEARDOWN_MS);
            }
            // Settings are rarely open: the window goes as soon as it's hidden.
            TIMER_SETTINGS_HIDE if self.settings.as_ref().is_some_and(|s| !s.visible) => {
                if let Some(mut s) = self.settings.take() {
                    s.finish_hide();
                }
                self.keyboard_hook = None;
                trim_working_set();
            }
            // Shown once: it goes as soon as it's hidden.
            TIMER_INTRO_HIDE if self.intro.as_ref().is_some_and(|i| !i.visible) => {
                if let Some(mut intro) = self.intro.take() {
                    intro.finish_hide();
                }
                trim_working_set();
            }
            TIMER_TEARDOWN if self.card.popup.as_ref().is_some_and(|p| !p.visible) => {
                self.card.popup = None;
                // Read afresh next time, so that voices installed meanwhile are found.
                self.voices = None;
                trim_working_set();
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Light dismiss: Esc, a click outside the card, or switching apps.

    fn install_dismiss_hooks(&mut self) {
        let _ = win::register_hotkey(self.hwnd, HOTKEY_ESCAPE, MOD_NOREPEAT, VK_ESCAPE.0 as u32);
        if self.mouse_hook.is_none() {
            self.mouse_hook = win::Hook::low_level(WH_MOUSE_LL, Some(mouse_hook));
        }
        if self.focus_hook.is_none() {
            self.focus_hook = win::EventHook::new(EVENT_SYSTEM_FOREGROUND, Some(focus_hook));
        }
    }

    fn remove_dismiss_hooks(&mut self) {
        win::unregister_hotkey(self.hwnd, HOTKEY_ESCAPE);
        (self.mouse_hook, self.focus_hook) = (None, None);
    }

    /// A click on the card in `hwnd`.
    fn on_click(&mut self, hwnd: HWND, x: i32, y: i32) {
        let button = self.popup_of(hwnd).and_then(|p| p.button_at(x, y));
        if button == Some(CardButton::Pin) {
            return self.toggle_pin(hwnd);
        }
        self.with_card(hwnd, |app| app.click(x, y));
    }

    fn click(&mut self, x: i32, y: i32) {
        let suggestion = self.card.popup.as_ref().and_then(|p| p.suggestion_at(x, y));
        if self.card.search.is_some() && suggestion.is_some() {
            return self.submit_search(suggestion);
        }
        match self.card.popup.as_ref().and_then(|p| p.button_at(x, y)) {
            Some(CardButton::Back) => return self.back(),
            Some(CardButton::Translate) => return self.translate(),
            Some(CardButton::Speak) => return self.speak(),
            Some(CardButton::Pin) | None => {}
        }
        if let Some(target) = self
            .card
            .popup
            .as_ref()
            .and_then(|p| p.link_at(x, y))
            .cloned()
        {
            self.open_link(target);
        } else if self.picking().is_some() {
            // A click away from the list shuts it.
            self.shut_picker();
        }
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Update => self.start_update(),
            Command::Autostart => {
                autostart::set(!autostart::is_enabled());
                self.refresh_settings();
                self.refresh_intro();
            }
            Command::Settings => self.open_settings(),
            Command::Reload => {
                match config::load() {
                    Ok((config, _)) => self.config = config,
                    Err(e) => return self.tray.notify(tr().settings_not_reloaded, &e),
                }
                self.cache.clear();
                self.apply_zoom();
                i18n::set(self.config.ui_language);
                let _ = jumplist::set();
                ui::set_motion(self.config.animated());
                self.register_hotkeys();
                self.schedule_update_check(updating::FIRST_CHECK_MS);
                self.refresh_settings();
                self.refresh_intro();
                if let Some(e) = self.load_dictionaries() {
                    self.tray.notify(tr().dictionary_not_loaded, &e);
                } else if self.hotkey_ok {
                    self.tray.notify(
                        tr().settings_reloaded,
                        &fill(
                            tr().settings_reloaded_detail,
                            &[("hotkey", &self.config.hotkey)],
                        ),
                    );
                }
            }
            Command::Quit => unsafe {
                self.remove_dismiss_hooks();
                PostQuitMessage(0);
            },
        }
    }
}

/// Why the shortcut couldn't be registered.
enum Unbound {
    Invalid(String),
    /// Another app has it.
    Taken,
    /// The other shortcut is the same keys.
    Clash,
}

/// Whether two shortcuts, as written, are the same keys.
fn same_keys(a: &str, b: &str) -> bool {
    matches!((hotkey::parse(a), hotkey::parse(b)), (Ok(a), Ok(b)) if a == b)
}

/// `anchor`, if a card can open next to it: some width, about a line tall,
/// on a screen.
fn usable_anchor(anchor: Option<RECT>) -> Option<RECT> {
    anchor.filter(|r| r.right > r.left && selection::is_line_sized(r))
}

/// How the settings say to read the selection, as `CAPTURE_*` flags.
fn capture_flags(config: &Config) -> isize {
    match config.selection {
        SelectionMode::Automatic => CAPTURE_UI_AUTOMATION,
        SelectionMode::Direct => CAPTURE_UI_AUTOMATION | CAPTURE_SKIP_CLIPBOARD,
        SelectionMode::Clipboard => 0,
    }
}

fn anchor_or_cursor(anchor: Option<RECT>) -> RECT {
    usable_anchor(anchor).unwrap_or_else(|| {
        let pt = win::cursor_pos();
        // Leave room for the pointer itself.
        RECT {
            left: pt.x,
            top: pt.y - 4,
            right: pt.x + 1,
            bottom: pt.y + 18,
        }
    })
}

/// `anchor`, or else the middle of the screen `window` is on, for a card
/// zoomed to `zoom`.
fn anchor_or_centre(anchor: Option<RECT>, window: HWND, zoom: f32) -> RECT {
    usable_anchor(anchor).unwrap_or_else(|| {
        let mut r = RECT::default();
        let near = if unsafe { GetWindowRect(window, &mut r) }.is_ok() {
            POINT {
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
            }
        } else {
            win::cursor_pos()
        };
        popup::centred_anchor(near, zoom)
    })
}

fn open_config_file() {
    let path = HSTRING::from(config::path().as_os_str());
    // No app is set for .json files: Notepad always is.
    if !win::shell_open(&path) {
        win::shell_run(&HSTRING::from("notepad.exe"), &path);
    }
}

fn trim_working_set() {
    unsafe {
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}
