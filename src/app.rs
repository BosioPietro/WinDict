//! Ties everything together on the UI thread: hotkey → selection → lookup → card.
//!
//! This file holds the state, the hotkey, the timers and the dismissal of the
//! card; the rest of `impl App` is split by flow into the submodules:
//! `search`, `lookup`, `translating`, `links`, `tray_menu` and `prefs`.
//! `procs` has the window procedures, `edit` and `picking` the pure text and
//! list logic.

pub(crate) mod edit;
mod links;
mod lookup;
pub(crate) mod picking;
mod prefs;
mod procs;
mod recent;
mod search;
mod translating;
mod tray_menu;

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{channel, Receiver, Sender};

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
use crate::config::{self, Config, SelectionMode};
use crate::dictionary::{self, Definition, Detail, Lookup, NormalizeError};
use crate::dicts;
use crate::hotkey;
use crate::i18n::{self, fill, tr};
use crate::lang::LangCode;
use crate::menu::{Command, Menu};
use crate::offline;
use crate::online;
use crate::popup::{self, Popup, Transition};
use crate::render::{self, Body, Side, Target, TranslationCard, PICKER_ROWS};
use crate::searchbox::MAX_SUGGESTIONS;
use crate::selection::{self, Capturer, Selection};
use crate::settings::{Change, Settings, Values};
use crate::theme::Theme;
use crate::translate::{self, Translation};
use crate::tray::{self, Tray};
use crate::ui;
use crate::win::{self, Window};
use edit::{Input, LineEdit, Motion};
use picking::Picking;
use procs::{
    focus_hook, keyboard_hook, main_proc, menu_proc, mouse_hook, popup_proc, settings_proc,
};
use recent::Recent;

const MAIN_CLASS: PCWSTR = w!("WinDict.Main");
const POPUP_CLASS: PCWSTR = w!("WinDict.Popup");
const MENU_CLASS: PCWSTR = w!("WinDict.Menu");
const SETTINGS_CLASS: PCWSTR = w!("WinDict.Settings");
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

const HOTKEY_LOOKUP: i32 = 1;
const HOTKEY_ESCAPE: i32 = 2;
const HOTKEY_BACK: i32 = 3;
const HOTKEY_BACK_ALT: i32 = 4;
const HOTKEY_NEXT_LINK: i32 = 5;
const HOTKEY_PREV_LINK: i32 = 6;
const HOTKEY_OPEN_LINK: i32 = 7;
const HOTKEY_TRANSLATE: i32 = 8;

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
const OPEN_KEY: Keys = &[(HOTKEY_OPEN_LINK, MOD_NOREPEAT, VK_RETURN)];
const TRANSLATE_KEY: Keys = &[(
    HOTKEY_TRANSLATE,
    HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_NOREPEAT.0),
    VIRTUAL_KEY(b'T' as u16),
)];

const TIMER_HIDE: usize = 1;
const TIMER_SETTLE: usize = 2;
const TIMER_AUTOHIDE: usize = 3;
const TIMER_TEARDOWN: usize = 4;
const TIMER_COVERED: usize = 5;
const TIMER_MENU_HIDE: usize = 6;
const TIMER_MENU_TEARDOWN: usize = 7;
const TIMER_SETTINGS_HIDE: usize = 8;
const TIMER_TRANSLATING: usize = 9;

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
    // When the card last appeared (win::now_ms), read by the focus hook.
    static SHOWN_AT: Cell<u64> = const { Cell::new(0) };
    // Where the pointer last moved over the card, on screen (GetMessagePos).
    static POINTER_AT: Cell<Option<u32>> = const { Cell::new(None) };
    // The settings window while it records a shortcut, read by the keyboard hook.
    static RECORDING_IN: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    // The keys the keyboard hook kept from Windows that are still down, by
    // virtual key, so that their repeats and releases are kept too.
    static SWALLOWED: Cell<[u64; 4]> = const { Cell::new([0; 4]) };
}

struct Captured {
    id: u64,
    selection: Option<Selection>,
}

struct Looked {
    id: u64,
    query: Query,
    result: Lookup,
}

/// A word to look up, in every language it's in, or only in the one it
/// was picked in from the search box's suggestions.
#[derive(Clone, PartialEq)]
struct Query {
    word: String,
    language: Option<LangCode>,
}

impl Query {
    /// `def`, or only its part in the language asked for.
    fn within(&self, def: Definition) -> Option<Definition> {
        match &self.language {
            Some(language) => def.only(language),
            None => Some(def),
        }
    }
}

/// A result that wasn't found, or couldn't be looked up, and the word it
/// was likely a typo of.
struct Corrected {
    id: u64,
    place: Place,
    word: String,
    result: Lookup,
    correction: Option<String>,
}

/// A word, from one language into another.
type TranslationKey = (String, LangCode, LangCode);

/// A translation, or why there's none, for the request it was asked with.
struct Translated {
    id: u64,
    key: TranslationKey,
    result: Result<Translation, String>,
}

/// What the definition on the card gives a translation of its word.
#[derive(Clone)]
struct Translatable {
    word: String,
    /// The languages it's in, each with the first thing the word means in
    /// it: context for a translation, should it need any.
    senses: Vec<(LangCode, Option<String>)>,
}

impl Translatable {
    fn of(def: &Definition) -> Translatable {
        let first = |section: &Definition| {
            let meaning = section.meanings.first()?;
            Some(meaning.senses.first()?.text.clone())
        };
        Translatable {
            word: def.word.clone(),
            senses: def
                .sections()
                .map(|s| (s.language.clone(), first(s)))
                .collect(),
        }
    }
}

/// The card turned into a translation of its word.
struct Translating {
    about: Translatable,
    from: LangCode,
    to: LangCode,
    /// The word is in both languages: which way waits on the user.
    asking: bool,
    /// The list a language is being picked from, when it's open.
    picking: Option<Picking>,
    /// `None` while it's on its way.
    result: Option<Result<Translation, String>>,
    /// The card waits a moment for it before showing it on its way, so a
    /// quick answer comes in one change rather than two.
    waiting: bool,
    /// Where the focus goes once the card stops waiting.
    focus: Option<Target>,
    /// Whether the card shows the translation yet, rather than the
    /// definition it was asked from.
    shown: bool,
}

impl Translating {
    /// How the card changes as the translation comes: from the definition
    /// it's another card; on the translation, only what's under the
    /// languages changes.
    fn transition(&self) -> Transition {
        if self.shown {
            Transition::Refine
        } else {
            Transition::Replace
        }
    }
}

/// A dictionary being downloaded, for the language it's for.
struct Installing {
    download: &'static dicts::Download,
    /// Fetched again at start, rather than ticked in the settings: only
    /// failing is told, and only when it leaves the language out.
    quiet: bool,
    /// Whether it's still wanted: one unticked as it downloads finishes,
    /// unused.
    wanted: bool,
}

/// What the capture, lookup and download threads hand the UI thread.
enum Event {
    /// The capture is waiting for a copy of the selection, for the request.
    Copying(u64),
    Captured(Captured),
    Looked(Looked),
    Corrected(Corrected),
    /// How far the download of a language's dictionary has got, in percent.
    Installing(&'static str, u32),
    Installed(&'static str, Result<(), String>),
    Translated(Translated),
}

/// Sends [`Event`]s to the UI thread. They go through a channel, and an
/// empty WM_APP_EVENT wakes the thread to read it: a message can't carry
/// them itself, as any app on the desktop can post one with any payload.
#[derive(Clone)]
struct Events {
    tx: Sender<Event>,
    /// The main window, as a number: HWND can't cross threads.
    hwnd: isize,
}

impl Events {
    fn send(&self, event: Event) {
        if self.tx.send(event).is_ok() {
            unsafe {
                let hwnd = HWND(self.hwnd as _);
                let _ = PostMessageW(Some(hwnd), WM_APP_EVENT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// What's typed in the search box.
#[derive(Default)]
struct Search {
    edit: LineEdit,
    title: &'static str,
    /// Whether the heading says why the last Enter did nothing.
    error: bool,
    /// The words offered under the field.
    suggestions: Vec<offline::Suggestion>,
    /// What they're picked from, read once something's typed.
    words: Option<offline::Words>,
}

/// Where a lookup's answer goes.
#[derive(Clone, Copy)]
enum Place {
    /// A new card next to the selection.
    New(RECT),
    /// The open card, in place of what it shows, changing as the
    /// transition says.
    Same(Transition),
}

pub struct App {
    config: Config,
    hwnd: HWND,
    compositor: Compositor,
    popup: Option<Popup>,
    menu: Option<Menu>,
    /// The settings window, while it's open.
    settings: Option<Settings>,
    /// When the tray menu last started closing (win::now_ms).
    menu_closed_at: u64,
    /// The press on the tray icon that closed the menu; its release mustn't reopen it.
    swallow_tray_release: bool,
    tray: Tray,
    taskbar_created: u32,
    capture_thread: u32,
    /// What the capture and lookup threads sent, and how they send it.
    inbox: Receiver<Event>,
    events: Events,
    request: u64,
    /// The card is open on its way, while the capture waits for a copy.
    copying: bool,
    /// Online answers: definitions, and definite "not found"s.
    cache: Recent<Query, Lookup>,
    /// The offline entry for the current request, if any.
    local: Option<Definition>,
    /// The word the card shows, as it was looked up.
    current: Option<Query>,
    /// Words the card showed before synonyms were followed, most recent last.
    history: Vec<Query>,
    /// The definition the card shows, if it does.
    translatable: Option<Translatable>,
    /// The translation the card turned into, if it did.
    translating: Option<Translating>,
    translations: Recent<TranslationKey, Translation>,
    /// Ctrl+T.
    translate_key: KeySet,
    /// Backspace and Alt+Left.
    back_keys: KeySet,
    /// Tab and Shift+Tab.
    link_keys: KeySet,
    /// Enter.
    open_key: KeySet,
    /// The search box, while the card shows one.
    search: Option<Search>,
    /// The window the search box took the keyboard from.
    return_focus: Option<HWND>,
    /// The dictionaries being downloaded, one at most for each language.
    installing: Vec<Installing>,
    hotkey_ok: bool,
    mouse_hook: Option<win::Hook>,
    focus_hook: Option<win::EventHook>,
    /// Catches keys before Windows does, while a shortcut is recorded.
    keyboard_hook: Option<win::Hook>,
}

pub fn run(compositor: Compositor) -> windows::core::Result<()> {
    let (config, first_run, config_error) = match config::load() {
        Ok((config, first_run)) => (config, first_run, None),
        Err(e) => (Config::default(), false, Some(e)),
    };
    i18n::set(config.ui_language);
    ui::set_motion(config.animated());
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
            popup: None,
            menu: None,
            settings: None,
            menu_closed_at: 0,
            swallow_tray_release: false,
            tray,
            taskbar_created,
            capture_thread,
            inbox,
            events,
            request: 0,
            copying: false,
            cache: Recent::new(CACHE_SIZE),
            local: None,
            current: None,
            history: Vec::new(),
            translatable: None,
            translating: None,
            translations: Recent::new(TRANSLATION_CACHE_SIZE),
            translate_key: KeySet::new(TRANSLATE_KEY),
            back_keys: KeySet::new(BACK_KEYS),
            link_keys: KeySet::new(LINK_KEYS),
            open_key: KeySet::new(OPEN_KEY),
            search: None,
            return_focus: None,
            installing: Vec::new(),
            hotkey_ok: false,
            mouse_hook: None,
            focus_hook: None,
            keyboard_hook: None,
        };
        app.register_hotkey();
        let mut dictionary_error = offline::set_languages(&app.config.languages).err();
        // A downloaded dictionary that's gone, or isn't the one this version
        // lists, is fetched again; failing to is told then.
        let languages = app.config.languages.clone();
        if languages
            .iter()
            .all(|l| l == "en" || dicts::find(l).is_some())
        {
            dictionary_error = None;
        }
        for download in languages.iter().filter_map(|l| dicts::find(l)) {
            app.install(download, true);
        }
        if let Some(e) = config_error {
            app.tray.notify(tr().settings_not_loaded, &e);
        } else if let Some(e) = dictionary_error {
            app.tray.notify(tr().dictionary_not_loaded, &e);
        } else if first_run && app.hotkey_ok {
            app.tray.notify(
                tr().running,
                &fill(tr().running_detail, &[("hotkey", &app.config.hotkey)]),
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

/// Registers the window classes of the main, card, menu and settings windows.
unsafe fn register_classes(instance: HINSTANCE) -> windows::core::Result<()> {
    let arrow = LoadCursorW(None, IDC_ARROW)?;
    let icon =
        LoadIconW(Some(instance), PCWSTR(tray::ICON_ID as usize as *const u16)).unwrap_or_default();
    let classes: [(PCWSTR, WNDPROC, HCURSOR, HICON); 4] = [
        (
            MAIN_CLASS,
            Some(main_proc),
            HCURSOR::default(),
            HICON::default(),
        ),
        (POPUP_CLASS, Some(popup_proc), arrow, HICON::default()),
        (MENU_CLASS, Some(menu_proc), arrow, HICON::default()),
        (SETTINGS_CLASS, Some(settings_proc), arrow, icon),
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
/// modal loop, some COM calls), so handlers that do must release it first,
/// as the tray menu does.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

impl App {
    /// Registers the shortcut, saying in a notification if it can't be.
    fn register_hotkey(&mut self) {
        match self.bind_hotkey() {
            Ok(()) => {}
            Err(Unbound::Taken) => self.tray.notify(
                tr().shortcut_unavailable,
                &fill(
                    tr().shortcut_unavailable_detail,
                    &[("hotkey", &self.config.hotkey)],
                ),
            ),
            Err(Unbound::Invalid(e)) => self.tray.notify(tr().shortcut_invalid, &e),
        }
    }

    fn bind_hotkey(&mut self) -> Result<(), Unbound> {
        win::unregister_hotkey(self.hwnd, HOTKEY_LOOKUP);
        self.hotkey_ok = false;
        let hk = hotkey::parse(&self.config.hotkey).map_err(Unbound::Invalid)?;
        let modifiers = hk.modifiers | MOD_NOREPEAT;
        win::register_hotkey(self.hwnd, HOTKEY_LOOKUP, modifiers, hk.vk)
            .map_err(|_| Unbound::Taken)?;
        self.hotkey_ok = true;
        self.tray
            .set_tip(&format!("WinDict - {}", self.config.hotkey));
        Ok(())
    }

    fn theme(&self) -> Theme {
        Theme::current(self.config.theme)
    }

    fn ensure_popup(&mut self) -> Option<&mut Popup> {
        if self.popup.is_none() {
            let window =
                Window::create(POPUP_CLASS, w!("WinDict"), FLYOUT_EX_STYLE, WS_POPUP).ok()?;
            match Popup::new(window, self.compositor.clone(), self.theme()) {
                Ok(p) => self.popup = Some(p),
                Err(e) => {
                    self.tray.notify(tr().draw_window_failed, &e.message());
                    return None;
                }
            }
        }
        self.popup.as_mut()
    }

    fn ensure_menu(&mut self) -> Option<&mut Menu> {
        if self.menu.is_none() {
            // Never takes the focus: the taskbar's overflow flyout closes
            // as soon as something else is activated.
            let window =
                Window::create(MENU_CLASS, w!("WinDict"), FLYOUT_EX_STYLE, WS_POPUP).ok()?;
            match Menu::new(window, self.compositor.clone(), self.theme()) {
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
            match Settings::new(window, self.compositor.clone(), theme, values) {
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

    fn on_hotkey(&mut self) {
        self.request += 1;
        self.copying = false;
        // The card has the keyboard (a search): there's no selection to
        // read, so open a fresh search box, or close the one that's open.
        let focused = self
            .popup
            .as_ref()
            .is_some_and(|p| p.visible && unsafe { GetForegroundWindow() } == p.hwnd);
        if focused {
            return if self.search.is_some() {
                self.dismiss()
            } else {
                self.open_search(tr().search_again_title, None)
            };
        }
        unsafe {
            let _ = PostThreadMessageW(
                self.capture_thread,
                WM_APP_CAPTURE,
                WPARAM(self.request as usize),
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
                Event::Looked(looked) => self.on_result(looked),
                Event::Corrected(corrected) => self.on_corrected(corrected),
                Event::Installing(language, percent) => self.on_installing(language, percent),
                Event::Installed(language, result) => self.on_installed(language, result),
                Event::Translated(translated) => self.on_translated(translated),
            }
        }
    }

    /// The selection is being copied, which can take up to a few hundred ms
    /// (or never come, when nothing is selected): open the card on its way,
    /// at the pointer, where a copied word would open it too.
    fn on_copying(&mut self, id: u64) {
        if id != self.request {
            return;
        }
        self.current = None;
        self.search = None;
        self.history.clear();
        (self.translatable, self.translating) = (None, None);
        self.sync_back_keys();
        self.show_body(
            anchor_or_cursor(None),
            "WinDict",
            None,
            Body::Loading,
            false,
        );
        self.copying = self.popup.as_ref().is_some_and(|p| p.visible);
    }

    fn on_selection(&mut self, captured: Captured) {
        if captured.id != self.request {
            return;
        }
        // The card opened on its way turns into the answer in place, unless
        // it was closed meanwhile.
        let in_place = std::mem::take(&mut self.copying);
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        if in_place && !visible {
            return;
        }
        self.current = None;
        self.search = None;
        self.history.clear();
        self.translating = None;
        self.sync_back_keys();
        let (text, anchor) = match captured.selection {
            Some(s) => (s.text, s.anchor),
            None => (String::new(), None),
        };
        let word = match dictionary::normalize(&text) {
            Ok(word) => word,
            Err(NormalizeError::TooLong) => {
                return self.show_message(anchor, tr().too_long, tr().too_long_detail, true);
            }
            Err(NormalizeError::Empty) => return self.open_search(tr().search_title, anchor),
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
        let Some(popup) = self.popup.as_mut() else {
            return;
        };
        let _ = popup.hover(point);
        let target = point.and_then(|(x, y)| popup.link_at(x, y)).cloned();
        let Some(Target::Pick(code)) = target else {
            return;
        };
        if self
            .picking()
            .is_some_and(|(picking, _)| picking.highlight(&code))
        {
            self.show_translation(Transition::Refine, None);
        }
    }

    /// The mouse wheel: scrolls the list of languages while it's open, or
    /// else the card.
    fn on_wheel(&mut self, delta: i32) {
        if let Some((picking, _)) = self.picking() {
            let rows = -(delta as f32 / 120.0 * ui::WHEEL_ROWS).round() as isize;
            if picking.scroll(rows) {
                self.show_translation(Transition::Refine, None);
            }
            return;
        }
        if let Some(popup) = self.popup.as_mut() {
            let _ = popup.scroll_by(delta);
        }
    }

    /// Esc: closes the list of languages, if it's open, or else the card.
    fn escape(&mut self) {
        let closed = self.translating.as_mut().and_then(|t| t.picking.take());
        match closed {
            Some(picking) => {
                let focus = Some(Target::Language(picking.side));
                self.show_translation(Transition::Refine, focus);
            }
            None => self.dismiss(),
        }
    }

    /// Gives the card the keyboard, to type into, until it closes and hands
    /// it back.
    fn take_keyboard(&mut self) {
        let Some(hwnd) = self.popup.as_ref().filter(|p| p.visible).map(|p| p.hwnd) else {
            return;
        };
        let previous = unsafe { GetForegroundWindow() };
        if previous != hwnd && selection::take_foreground(hwnd) && self.return_focus.is_none() {
            self.return_focus = Some(previous);
        }
    }

    fn dismiss(&mut self) {
        self.history.clear();
        self.translating = None;
        self.sync_back_keys();
        self.set_link_keys(false, false);
        self.translate_key.set(self.hwnd, false);
        self.remove_dismiss_hooks();
        self.dismiss_menu();
        self.search = None;
        // Hand the keyboard back to the app the search box took it from,
        // unless the user has moved on to another one already.
        if let Some(previous) = self.return_focus.take() {
            let focused = self
                .popup
                .as_ref()
                .is_some_and(|p| unsafe { GetForegroundWindow() } == p.hwnd);
            if focused {
                unsafe {
                    let _ = SetForegroundWindow(previous);
                }
            }
        }
        let Some(popup) = self.popup.as_mut() else {
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
            TIMER_TRANSLATING => self.stop_waiting(),
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
                win::set_timer(self.hwnd, TIMER_MENU_TEARDOWN, IDLE_TEARDOWN_MS);
            }
            TIMER_MENU_TEARDOWN if self.menu.as_ref().is_some_and(|m| !m.visible) => {
                self.menu = None;
                trim_working_set();
            }
            TIMER_HIDE => {
                if let Some(p) = self.popup.as_mut() {
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
            TIMER_TEARDOWN if self.popup.as_ref().is_some_and(|p| !p.visible) => {
                self.popup = None;
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

    fn on_click(&mut self, x: i32, y: i32) {
        let suggestion = self.popup.as_ref().and_then(|p| p.suggestion_at(x, y));
        if self.search.is_some() && suggestion.is_some() {
            return self.submit_search(suggestion);
        }
        if self.popup.as_ref().is_some_and(|p| p.back_at(x, y)) {
            return self.back();
        }
        if self.popup.as_ref().is_some_and(|p| p.translate_at(x, y)) {
            return self.translate();
        }
        if let Some(target) = self.popup.as_ref().and_then(|p| p.link_at(x, y)).cloned() {
            self.open_link(target);
        } else if self.picking().is_some() {
            // A click away from the list shuts it.
            if let Some(t) = self.translating.as_mut() {
                t.picking = None;
            }
            self.show_translation(Transition::Refine, None);
        }
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Autostart => {
                autostart::set(!autostart::is_enabled());
                self.refresh_settings();
            }
            Command::Settings => self.open_settings(),
            Command::Reload => {
                match config::load() {
                    Ok((config, _)) => self.config = config,
                    Err(e) => return self.tray.notify(tr().settings_not_reloaded, &e),
                }
                self.cache.clear();
                i18n::set(self.config.ui_language);
                ui::set_motion(self.config.animated());
                self.register_hotkey();
                self.refresh_settings();
                if let Err(e) = offline::set_languages(&self.config.languages) {
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

/// `anchor`, or else the middle of the screen `window` is on.
fn anchor_or_centre(anchor: Option<RECT>, window: HWND) -> RECT {
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
        popup::centred_anchor(near)
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
