//! Ties everything together on the UI thread: hotkey → selection → lookup → card.

pub(crate) mod edit;
mod picking;
mod procs;

use std::cell::{Cell, RefCell};
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, SetProcessWorkingSetSize,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Compositor;

use crate::autostart;
use crate::config::{self, Config};
use crate::dictionary::{self, Definition, Detail, Lookup, NormalizeError};
use crate::dicts;
use crate::hotkey;
use crate::menu::{Command, Menu};
use crate::offline;
use crate::online;
use crate::popup::{self, Popup, Transition};
use crate::render::{self, Body, Side, Target, TranslationCard, PICKER_ROWS};
use crate::searchbox::MAX_SUGGESTIONS;
use crate::selection::{self, Capturer, Selection};
use crate::settings::{self, Change, Settings, Values};
use crate::theme::Theme;
use crate::translate::{self, Translation};
use crate::tray::{self, Tray};
use crate::ui;
use crate::win::{self, Window};
use edit::{Input, LineEdit, Motion};
use picking::Picking;
use procs::{focus_hook, main_proc, menu_proc, mouse_hook, popup_proc, settings_proc};

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
const WM_APP_CAPTURE: u32 = WM_APP + 6;
const WM_APP_CLICK: u32 = WM_APP + 7;
const WM_APP_BACK: u32 = WM_APP + 8;
const WM_APP_CLOSE_SETTINGS: u32 = WM_APP + 9;

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
/// How many languages a notch of the mouse wheel scrolls their list by.
const WHEEL_ROWS: f32 = 3.0;
/// How many words "back" remembers.
const HISTORY_SIZE: usize = 50;
const SEARCH_PLACEHOLDER: &str = "Type a word to look it up";
const SEARCH_TITLE: &str = "No text selected";
/// Opened again from the card itself, where nothing was meant to be selected.
const SEARCH_AGAIN_TITLE: &str = "Look up another word";
const SEARCH_TOO_LONG: &str = "Try a single word or a short phrase";
/// Suggestions start with this many letters typed: fewer match too much.
const SUGGEST_AFTER: usize = 2;
/// Under the field when the bundled dictionary has nothing to suggest.
const NO_SUGGESTIONS: &str = "No matches in the offline dictionary";
const NO_SUGGESTIONS_ONLINE: &str = "No offline matches \u{2014} Enter searches online";

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    // Read by the low-level mouse hook, which must not touch APP.
    static CARD_RECT: Cell<RECT> = const { Cell::new(RECT { left: 0, top: 0, right: 0, bottom: 0 }) };
    static MSG_HWND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    // When the card last appeared (win::now_ms), read by the focus hook.
    static SHOWN_AT: Cell<u64> = const { Cell::new(0) };
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
    language: Option<String>,
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
type TranslationKey = (String, String, String);

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
    senses: Vec<(String, Option<String>)>,
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
    from: String,
    to: String,
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
    /// Online answers, most recently used last.
    cache: Vec<(Query, Lookup)>,
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
    /// Translations, most recently used last.
    translations: Vec<(TranslationKey, Translation)>,
    /// Whether Ctrl+T is registered.
    translate_key: bool,
    /// Whether Backspace and Alt+Left are registered.
    back_keys: bool,
    /// Whether Tab and Shift+Tab are registered.
    link_keys: bool,
    /// Whether Enter is registered.
    open_key: bool,
    /// The search box, while the card shows one.
    search: Option<Search>,
    /// The window the search box took the keyboard from.
    return_focus: Option<HWND>,
    /// The dictionaries being downloaded, one at most for each language.
    installing: Vec<Installing>,
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
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        let icon = LoadIconW(Some(instance), PCWSTR(tray::ICON_ID as usize as *const u16))
            .unwrap_or_default();
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

        // Selection capture runs on its own thread: it sleeps while waiting
        // for keys to be released and for the clipboard to change. That
        // thread also owns the clipboard after a restore, so it must pump
        // messages (other apps send WM_DESTROYCLIPBOARD to the owner).
        let (ready, thread_id) = channel::<u32>();
        let (tx, inbox) = channel();
        let events = Events {
            tx,
            hwnd: hwnd.0 as isize,
        };
        let capture_events = events.clone();
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
                    capture_events.send(Event::Captured(Captured { id, selection }));
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
            settings: None,
            menu_closed_at: 0,
            swallow_tray_release: false,
            tray,
            taskbar_created,
            capture_thread,
            inbox,
            events,
            request: 0,
            cache: Vec::new(),
            local: None,
            current: None,
            history: Vec::new(),
            translatable: None,
            translating: None,
            translations: Vec::new(),
            translate_key: false,
            back_keys: false,
            link_keys: false,
            open_key: false,
            search: None,
            return_focus: None,
            installing: Vec::new(),
            hotkey_ok: false,
            mouse_hook: None,
            focus_hook: None,
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
            app.tray
                .notify("Settings not loaded, using the defaults", &e);
        } else if let Some(e) = dictionary_error {
            app.tray.notify("Dictionary not loaded", &e);
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
                "Shortcut unavailable",
                &format!(
                    "{} is already used by another app. Choose another one in Settings.",
                    self.config.hotkey
                ),
            ),
            Err(Unbound::Invalid(e)) => self.tray.notify("Invalid shortcut", &e),
        }
    }

    fn bind_hotkey(&mut self) -> Result<(), Unbound> {
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_LOOKUP);
        }
        self.hotkey_ok = false;
        let hk = hotkey::parse(&self.config.hotkey).map_err(Unbound::Invalid)?;
        let modifiers = hk.modifiers | MOD_NOREPEAT;
        unsafe { RegisterHotKey(Some(self.hwnd), HOTKEY_LOOKUP, modifiers, hk.vk) }
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
            // Never takes the focus: the taskbar's overflow flyout closes
            // as soon as something else is activated.
            let window =
                Window::create(MENU_CLASS, w!("WinDict"), FLYOUT_EX_STYLE, WS_POPUP).ok()?;
            match Menu::new(window, self.compositor.clone(), self.theme()) {
                Ok(m) => self.menu = Some(m),
                Err(e) => {
                    self.tray
                        .notify("WinDict couldn't draw its menu", &e.message());
                    return None;
                }
            }
        }
        self.menu.as_mut()
    }

    fn ensure_settings(&mut self, theme: Theme, values: Values) -> Option<&mut Settings> {
        if self.settings.is_none() {
            let window = Window::create(
                SETTINGS_CLASS,
                w!("WinDict Settings"),
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
                WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            )
            .ok()?;
            match Settings::new(window, self.compositor.clone(), theme, values) {
                Ok(s) => self.settings = Some(s),
                Err(e) => {
                    self.tray
                        .notify("WinDict couldn't draw its settings", &e.message());
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
                self.open_search(SEARCH_AGAIN_TITLE, None)
            };
        }
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

    /// Handles what the capture and lookup threads sent.
    fn on_events(&mut self) {
        while let Ok(event) = self.inbox.try_recv() {
            match event {
                Event::Captured(captured) => self.on_selection(captured),
                Event::Looked(looked) => self.on_result(looked),
                Event::Corrected(corrected) => self.on_corrected(corrected),
                Event::Installing(language, percent) => self.on_installing(language, percent),
                Event::Installed(language, result) => self.on_installed(language, result),
                Event::Translated(translated) => self.on_translated(translated),
            }
        }
    }

    fn on_selection(&mut self, captured: Captured) {
        if captured.id != self.request {
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
                let detail = "Select a single word or a short phrase.";
                return self.show_message(anchor, "That\u{2019}s a lot of text", detail, true);
            }
            Err(NormalizeError::Empty) => return self.open_search(SEARCH_TITLE, anchor),
        };
        let query = Query {
            word,
            language: None,
        };
        self.lookup(query, Place::New(anchor_or_cursor(anchor)));
    }

    /// Nothing was selected: opens the card as a search box, for text that
    /// can't be selected (in an image, a protected PDF, a game). It opens at
    /// `anchor`, the text caret, when there is one.
    fn open_search(&mut self, title: &'static str, anchor: Option<RECT>) {
        self.current = None;
        self.history.clear();
        (self.translatable, self.translating) = (None, None);
        self.sync_back_keys();
        let previous = unsafe { GetForegroundWindow() };
        self.dismiss_menu();
        let theme = self.theme();
        let Some(popup) = self.ensure_popup() else {
            return;
        };
        // With no caret to open at (an image, a game), the middle of the
        // screen the app is on.
        let anchor = anchor_or_centre(anchor, previous);
        if let Err(e) = popup.show_search(anchor, theme, title, SEARCH_PLACEHOLDER) {
            self.tray
                .notify("WinDict couldn't draw its window", &e.message());
            return;
        }
        let hwnd = popup.hwnd;
        self.on_shown(false);
        if !selection::take_foreground(hwnd) {
            let detail = format!(
                "Select a word in any app, then press {}.",
                self.config.hotkey
            );
            return self.show_message(None, "Nothing selected", &detail, true);
        }
        // Opened again from the card itself, it keeps the original app.
        if previous != hwnd {
            self.return_focus = Some(previous);
        }
        self.search = Some(Search {
            title,
            ..Search::default()
        });
        self.sync_link_keys();
    }

    /// A character typed into the card (WM_CHAR).
    fn on_char(&mut self, unit: u16) {
        if self.picking().is_some() {
            return self.type_language(unit);
        }
        let Some(hwnd) = self.popup.as_ref().filter(|p| p.visible).map(|p| p.hwnd) else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        match search.edit.type_unit(unit) {
            Input::Edited => {}
            Input::Enter => {
                let picked = self.popup.as_ref().and_then(|p| p.suggestion());
                return self.submit_search(picked);
            }
            Input::Paste => {
                let pasted = selection::clipboard_text(hwnd).unwrap_or_default();
                search.edit.paste(&pasted);
            }
            Input::Nothing => return,
        }
        self.refresh_search();
    }

    /// A key pressed in the card (WM_KEYDOWN): Up, Down, Tab and Shift+Tab
    /// pick a suggestion; Left, Right, Home and End move the caret, by
    /// words with Ctrl; Delete removes what's after it.
    fn on_key(&mut self, key: VIRTUAL_KEY) {
        if self.picking().is_some() {
            return self.language_key(key);
        }
        let (Some(search), Some(popup)) = (self.search.as_mut(), self.popup.as_mut()) else {
            return;
        };
        let ctrl = hotkey::is_held(VK_CONTROL);
        let down = match key {
            VK_DOWN => Some(true),
            VK_UP => Some(false),
            VK_TAB => Some(!hotkey::is_held(VK_SHIFT)),
            _ => None,
        };
        if let Some(down) = down {
            let _ = popup.step_suggestion(down);
            return;
        }
        let edit = &mut search.edit;
        let motion = match key {
            VK_LEFT if ctrl => Motion::WordBack,
            VK_LEFT => Motion::CharBack,
            VK_RIGHT if ctrl => Motion::WordForward,
            VK_RIGHT => Motion::CharForward,
            VK_HOME => Motion::Start,
            VK_END => Motion::End,
            VK_DELETE => {
                let to = if ctrl {
                    Motion::WordForward
                } else {
                    Motion::CharForward
                };
                if edit.delete(to) {
                    self.refresh_search();
                }
                return;
            }
            _ => return,
        };
        if edit.move_caret(motion) {
            let _ = popup.set_query(edit.text(), edit.caret_utf16());
        }
    }

    /// Shows what's typed in the search box, and the words it could be the
    /// start of.
    fn refresh_search(&mut self) {
        let (Some(search), Some(popup)) = (self.search.as_mut(), self.popup.as_mut()) else {
            return;
        };
        // Typing again answers the complaint: the heading goes back.
        if std::mem::take(&mut search.error) {
            let _ = popup.set_search_title(search.title);
        }
        let _ = popup.set_query(search.edit.text(), search.edit.caret_utf16());
        let typed = search.edit.text().trim_start().to_lowercase();
        let typed_chars = typed.chars().count();
        let suggest = typed_chars >= SUGGEST_AFTER;
        let mut multilingual = false;
        search.suggestions = if suggest {
            let words = search.words.get_or_insert_with(offline::Words::load);
            multilingual = words.multilingual();
            words.suggest(&typed, MAX_SUGGESTIONS)
        } else {
            Vec::new()
        };
        // Suggestions come from the bundled dictionary only: say so when it
        // has nothing, and whether Enter will still look further.
        let note =
            (suggest && search.suggestions.is_empty()).then_some(if self.config.online_lookup {
                NO_SUGGESTIONS_ONLINE
            } else {
                NO_SUGGESTIONS
            });
        // With several languages, each says which it's in.
        let items: Vec<_> = search
            .suggestions
            .iter()
            .map(|s| {
                let badge = multilingual.then_some(s.language.as_str());
                (s.word.as_str(), s.base.as_deref(), s.typed, badge)
            })
            .collect();
        let _ = popup.set_suggestions(&items, note);
        self.on_resized();
    }

    /// Enter in the search box, or a click on a suggestion: the card turns
    /// into the definition of the `picked` suggestion, in its language when
    /// there are several, or of what's typed, in all of them.
    fn submit_search(&mut self, picked: Option<usize>) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let picked_row = picked.filter(|&i| i < search.suggestions.len());
        let picked = picked_row.map(|i| &search.suggestions[i]);
        let text = picked.map_or(search.edit.text(), |s| s.word.as_str());
        let multilingual = search.words.as_ref().is_some_and(|w| w.multilingual());
        let language = picked.filter(|_| multilingual).map(|s| s.language.clone());
        let complaint = match dictionary::normalize(text) {
            Ok(word) => {
                self.search = None;
                // The heading grows out of the word that was picked.
                if let Some(popup) = self.popup.as_mut() {
                    let _ = popup.pick_suggestion(picked_row);
                }
                let query = Query { word, language };
                return self.lookup(query, Place::Same(Transition::Replace));
            }
            Err(NormalizeError::Empty) => None,
            Err(NormalizeError::TooLong) if !search.error => {
                search.error = true;
                Some(SEARCH_TOO_LONG)
            }
            Err(NormalizeError::TooLong) => None,
        };
        if let Some(popup) = self.popup.as_mut() {
            let _ = popup.shake_search();
            if let Some(complaint) = complaint {
                let _ = popup.set_search_title(complaint);
            }
        }
    }

    /// Looks `query` up and shows the answer at `place`. The bundled
    /// dictionary answers instantly; the online one fills gaps.
    fn lookup(&mut self, query: Query, place: Place) {
        let detail = Detail {
            senses: self.config.max_definitions,
            synonyms: self.config.max_synonyms,
            examples: self.config.show_examples,
        };
        let word = query.word.clone();
        self.current = Some(query.clone());
        self.local = offline::lookup(&word, detail).and_then(|def| query.within(def));
        let online_key = Query {
            word: match &self.local {
                Some(def) => def.word.to_lowercase(),
                None => word.clone(),
            },
            language: query.language,
        };
        let online = self.cached(&online_key);

        match (&self.local, online) {
            (Some(local), online) => {
                let order = offline::languages();
                let shown = dictionary::merge(local.clone(), online.as_ref(), &order);
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

        let (id, events) = (self.request, self.events.clone());
        // In the languages of the dictionaries in use, which is English when
        // the chosen ones couldn't be read, or the one picked.
        let languages = match &online_key.language {
            Some(language) => vec![language.clone()],
            None => offline::languages(),
        };
        std::thread::spawn(move || {
            let result = online::lookup(&online_key.word, detail, &languages);
            events.send(Event::Looked(Looked {
                id,
                query: online_key,
                result,
            }));
        });
    }

    /// A cached online answer (a definition, or a definite "not found").
    fn cached(&mut self, key: &Query) -> Option<Lookup> {
        let hit = self.cache.iter().position(|(q, _)| q == key)?;
        let entry = self.cache.remove(hit);
        let result = entry.1.clone();
        self.cache.push(entry);
        Some(result)
    }

    fn on_result(&mut self, looked: Looked) {
        // Remember definite answers; retry failures (e.g. offline) next time.
        if !matches!(looked.result, Lookup::Failed(_)) {
            self.cache.retain(|(q, _)| *q != looked.query);
            self.cache
                .push((looked.query.clone(), looked.result.clone()));
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
                // What's new: a pronunciation, parts of speech, languages.
                let measure = |def: &Definition| {
                    let meanings: usize = def.sections().map(|s| s.meanings.len()).sum();
                    (def.phonetic.clone(), meanings, def.others.len())
                };
                let before = measure(&local);
                let order = offline::languages();
                let merged = dictionary::merge(local, Some(&looked.result), &order);
                if measure(&merged) != before {
                    // Another language's part is new content, perhaps above
                    // what's being read: it fades in rather than snapping.
                    let transition = if merged.others.len() != before.2 {
                        Transition::Replace
                    } else {
                        Transition::Refine
                    };
                    let place = Place::Same(transition);
                    self.present(place, &looked.query.word, &Lookup::Found(merged));
                }
            }
            None => {
                let place = Place::Same(Transition::Replace);
                self.present(place, &looked.query.word, &looked.result);
            }
        }
    }

    /// The card's title, pronunciation and body for a lookup of `word`,
    /// offering `correction` when it wasn't found or couldn't be looked up.
    fn card_for<'a>(
        word: &'a str,
        result: &'a Lookup,
        correction: Option<&'a str>,
    ) -> (&'a str, Option<&'a str>, Body<'a>) {
        match result {
            Lookup::Found(def) => (&def.word, def.phonetic.as_deref(), Body::Definition(def)),
            Lookup::NotFound => (
                word,
                None,
                Body::Message {
                    title: "No definition found",
                    detail: match correction {
                        Some(_) => "This word isn\u{2019}t in the dictionary.",
                        None => "This word isn\u{2019}t in the dictionary. Check the spelling?",
                    },
                    suggestion: correction,
                },
            ),
            Lookup::Failed(error) => (
                word,
                None,
                Body::Message {
                    title: "Couldn\u{2019}t look that up",
                    detail: error,
                    suggestion: correction,
                },
            ),
        }
    }

    /// Shows `result` for `word` at `place`. A word that wasn't found, or
    /// couldn't be looked up, is shown once the word it was likely a typo of
    /// is found, on a thread of its own: that reads the whole word list (a
    /// few tens of ms), dropped again straight after.
    fn present(&mut self, place: Place, word: &str, result: &Lookup) {
        if matches!(result, Lookup::Found(_)) {
            return self.present_corrected(place, word, result, None);
        }
        let (id, events) = (self.request, self.events.clone());
        let (word, result) = (word.to_string(), result.clone());
        std::thread::spawn(move || {
            let correction = offline::Words::load().correct(&word).map(str::to_string);
            events.send(Event::Corrected(Corrected {
                id,
                place,
                word,
                result,
                correction,
            }));
        });
    }

    fn on_corrected(&mut self, c: Corrected) {
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        if c.id != self.request || (matches!(c.place, Place::Same(_)) && !visible) {
            return;
        }
        self.present_corrected(c.place, &c.word, &c.result, c.correction.as_deref());
    }

    fn present_corrected(
        &mut self,
        place: Place,
        word: &str,
        result: &Lookup,
        correction: Option<&str>,
    ) {
        let (title, phonetic, body) = Self::card_for(word, result, correction);
        self.present_body(place, title, phonetic, body);
    }

    fn present_body(&mut self, place: Place, title: &str, phonetic: Option<&str>, body: Body) {
        self.translatable = match &body {
            Body::Definition(def) => Some(Translatable::of(def)),
            _ => None,
        };
        self.translating = None;
        match place {
            Place::New(anchor) => self.show_body(anchor, title, phonetic, body, false),
            Place::Same(transition) => {
                let back = !self.history.is_empty();
                let translate = self.can_translate();
                let Some(popup) = self.popup.as_mut() else {
                    return;
                };
                let _ = popup.set_back(back);
                let _ = popup.set_translate(translate);
                if popup
                    .set_content(title, phonetic, body, transition)
                    .is_err()
                {
                    return;
                }
                self.on_resized();
                self.sync_link_keys();
            }
        }
    }

    fn show_message(&mut self, anchor: Option<RECT>, title: &str, detail: &str, auto_hide: bool) {
        (self.translatable, self.translating) = (None, None);
        let anchor = anchor_or_cursor(anchor);
        self.show_body(
            anchor,
            "WinDict",
            None,
            Body::Message {
                title,
                detail,
                suggestion: None,
            },
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
        let translate = self.can_translate();
        let Some(popup) = self.ensure_popup() else {
            return;
        };
        let _ = popup.set_translate(translate);
        if let Err(e) = popup.show(anchor, theme, title, phonetic, body) {
            self.tray
                .notify("WinDict couldn't draw its window", &e.message());
            return;
        }
        self.on_shown(auto_hide);
    }

    /// The card just appeared, or turned into something else in place.
    fn on_shown(&mut self, auto_hide: bool) {
        let hwnd = self.hwnd;
        self.on_resized();
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_HIDE);
            let _ = KillTimer(Some(hwnd), TIMER_TEARDOWN);
            let _ = KillTimer(Some(hwnd), TIMER_AUTOHIDE);
            if auto_hide {
                SetTimer(Some(hwnd), TIMER_AUTOHIDE, AUTOHIDE_MS, None);
            }
        }
        SHOWN_AT.set(win::now_ms());
        self.install_dismiss_hooks();
        self.sync_link_keys();
        unsafe {
            SetTimer(Some(hwnd), TIMER_COVERED, COVERED_CHECK_MS, None);
        }
    }

    /// The card changed size: clicks outside it now are those outside the
    /// new size, and its clickable area shrinks once the resize has played.
    fn on_resized(&mut self) {
        let Some(popup) = self.popup.as_ref() else {
            return;
        };
        CARD_RECT.set(popup.card_rect());
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SETTLE, SETTLE_MS, None);
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
            SHOWN_AT.set(win::now_ms());
            selection::close_flyout();
        }
    }

    /// Shows a synonym's definition in the card, remembering the way back.
    /// A card in one language stays in it.
    fn follow(&mut self, word: &str) {
        let Ok(word) = dictionary::normalize(word) else {
            return;
        };
        let language = self.current.as_ref().and_then(|q| q.language.clone());
        let query = Query { word, language };
        if self.current.as_ref() == Some(&query) {
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
        self.lookup(query, Place::Same(Transition::Replace));
        self.sync_back_keys();
    }

    /// Goes back to the word shown before the last synonym was followed, or
    /// from a translation to the definition it was of.
    fn back(&mut self) {
        if !self.popup.as_ref().is_some_and(|p| p.visible) {
            return;
        }
        let query = match self.translating.take() {
            Some(_) => self.current.clone(),
            None => self.history.pop(),
        };
        let Some(query) = query else {
            return;
        };
        self.request += 1;
        self.lookup(query, Place::Same(Transition::Replace));
        self.sync_back_keys();
    }

    /// Backspace and Alt+Left go back, but only while there's somewhere to
    /// go: otherwise they belong to the app underneath.
    fn sync_back_keys(&mut self) {
        // Backspace edits what's typed in the list of languages.
        let picking = self.translating.as_ref().is_some_and(|t| t.picking.is_some());
        let wanted = (!self.history.is_empty() || self.translating.is_some()) && !picking;
        set_hotkeys(self.hwnd, BACK_KEYS, &mut self.back_keys, wanted);
    }

    /// Whether the card shows a definition it can turn into a translation.
    fn can_translate(&self) -> bool {
        self.config.online_lookup && self.translatable.is_some() && self.translating.is_none()
    }

    /// Ctrl+T: turns the definition into a translation, or back.
    fn toggle_translation(&mut self) {
        if self.translating.is_some() {
            self.back();
        } else {
            self.translate();
        }
    }

    /// Turns the card into a translation of its word: from the language
    /// it's in to the other one in use, or as the settings say when that
    /// can't be told.
    fn translate(&mut self) {
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        if !visible || !self.can_translate() {
            return;
        }
        let Some(about) = self.translatable.clone() else {
            return;
        };
        let found: Vec<&str> = about.senses.iter().map(|(l, _)| l.as_str()).collect();
        let defaults = (
            self.config.translate_from.as_str(),
            self.config.translate_to.as_str(),
        );
        let direction = translate::pick(&found, &self.config.languages, defaults);
        let asking = !direction.sure;
        self.translating = Some(Translating {
            about,
            from: direction.from,
            to: direction.to,
            asking,
            picking: None,
            result: None,
            waiting: false,
            focus: None,
            shown: false,
        });
        // Answers still on their way are for the definition.
        self.request += 1;
        if !asking {
            self.fetch_translation();
        }
        let focus = asking.then_some(Target::Confirm);
        self.show_translation(Transition::Replace, focus);
        self.sync_back_keys();
    }

    /// Translates the word the way the card says, from the cache when it can.
    fn fetch_translation(&mut self) {
        let Some(t) = self.translating.as_ref() else {
            return;
        };
        let key = (t.about.word.clone(), t.from.clone(), t.to.clone());
        let context = t
            .about
            .senses
            .iter()
            .find(|(language, _)| *language == t.from)
            .and_then(|(_, sense)| sense.clone());
        let hit = self.translations.iter().position(|(k, _)| *k == key);
        let cached = hit.map(|i| {
            let entry = self.translations.remove(i);
            let translation = entry.1.clone();
            self.translations.push(entry);
            translation
        });
        let Some(t) = self.translating.as_mut() else {
            return;
        };
        t.result = cached.map(Ok);
        t.waiting = t.result.is_none();
        if !t.waiting {
            return;
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_TRANSLATING, TRANSLATING_WAIT_MS, None);
        }
        self.request += 1;
        let (id, events) = (self.request, self.events.clone());
        let email = self.config.mymemory_email.clone();
        std::thread::spawn(move || {
            let (word, from, to) = &key;
            let result = translate::translate(word, from, to, context.as_deref(), &email);
            events.send(Event::Translated(Translated { id, key, result }));
        });
    }

    fn on_translated(&mut self, translated: Translated) {
        // Remember translations; retry failures (e.g. offline) next time.
        if let Ok(translation) = &translated.result {
            self.translations.retain(|(k, _)| *k != translated.key);
            self.translations
                .push((translated.key.clone(), translation.clone()));
            if self.translations.len() > TRANSLATION_CACHE_SIZE {
                self.translations.remove(0);
            }
        }
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        if translated.id != self.request || !visible {
            return;
        }
        let Some(t) = self.translating.as_mut() else {
            return;
        };
        t.result = Some(translated.result);
        t.waiting = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TRANSLATING);
        }
        let focus = t.focus.take();
        let transition = t.transition();
        self.show_translation(transition, focus);
    }

    /// No quick answer: shows the translation on its way.
    fn stop_waiting(&mut self) {
        let Some(t) = self.translating.as_mut().filter(|t| t.waiting) else {
            return;
        };
        t.waiting = false;
        let focus = t.focus.take();
        let transition = t.transition();
        self.show_translation(transition, focus);
    }

    /// Shows the translation as it stands, the focus on `focus`, or where
    /// it was.
    fn show_translation(&mut self, transition: Transition, focus: Option<Target>) {
        let Some(t) = self.translating.as_mut() else {
            return;
        };
        if t.waiting && !t.shown {
            // The definition stays a moment longer.
            t.focus = focus.or(t.focus.take());
            return;
        }
        let Some(popup) = self.popup.as_mut().filter(|p| p.visible) else {
            return;
        };
        // The list has the keyboard while it's open.
        let focus = match t.picking {
            Some(_) => None,
            None => focus.or_else(|| popup.focused_link().cloned()),
        };
        let state = match (&t.result, t.asking) {
            (_, true) => render::Translated::Asking,
            (None, false) if t.waiting => render::Translated::Waiting,
            (None, false) => render::Translated::Loading,
            (Some(Ok(translation)), false) => render::Translated::Done(translation),
            (Some(Err(error)), false) => render::Translated::Failed(error),
        };
        let picker = t.picking.as_ref().map(|p| render::Picker {
            side: p.side,
            query: p.query.text(),
            caret: p.query.caret_utf16(),
            matches: &p.matches,
            first: p.first,
            highlight: p.highlight,
        });
        let card = TranslationCard {
            word: &t.about.word,
            from: &t.from,
            to: &t.to,
            state,
            picker,
        };
        let _ = popup.set_back(true);
        let _ = popup.set_translate(false);
        let body = Body::Translation(card);
        if popup
            .set_content(&t.about.word, None, body, transition)
            .is_err()
        {
            return;
        }
        t.shown = true;
        if t.picking.is_some() {
            let _ = popup.unfocus();
        } else if let Some(target) = focus {
            let _ = popup.focus_target(&target);
        }
        self.on_resized();
        self.sync_link_keys();
        self.sync_back_keys();
    }

    /// The list a language is being picked from, if it's open, and the
    /// language that side has now.
    fn picking(&mut self) -> Option<(&mut Picking, &str)> {
        let t = self.translating.as_mut()?;
        let picking = t.picking.as_mut()?;
        let current = match picking.side {
            Side::From => &t.from,
            Side::To => &t.to,
        };
        Some((picking, current))
    }

    /// A character typed while the list of languages is open: narrows it
    /// down, or, with Enter, picks the highlighted one.
    fn type_language(&mut self, unit: u16) {
        let hwnd = self.popup.as_ref().map(|p| p.hwnd);
        let Some((picking, current)) = self.picking() else {
            return;
        };
        match picking.query.type_unit(unit) {
            Input::Edited => {}
            Input::Enter => {
                if let Some(code) = picking.highlighted() {
                    self.steer_translation(Target::Pick(code.to_string()));
                }
                return;
            }
            Input::Paste => {
                let pasted = hwnd.and_then(selection::clipboard_text).unwrap_or_default();
                picking.query.paste(&pasted);
            }
            Input::Nothing => return,
        }
        let current = current.to_string();
        picking.refilter(&current);
        self.show_translation(Transition::Refine, None);
    }

    /// A key pressed while the list of languages is open: Up, Down, Tab,
    /// Shift+Tab, Page Up and Page Down move the highlight; the others move
    /// the caret, as in the search box.
    fn language_key(&mut self, key: VIRTUAL_KEY) {
        let ctrl = hotkey::is_held(VK_CONTROL);
        let Some((picking, current)) = self.picking() else {
            return;
        };
        let page = PICKER_ROWS as isize;
        let step = match key {
            VK_DOWN => Some(1),
            VK_UP => Some(-1),
            VK_TAB if hotkey::is_held(VK_SHIFT) => Some(-1),
            VK_TAB => Some(1),
            VK_NEXT => Some(page),
            VK_PRIOR => Some(-page),
            _ => None,
        };
        if let Some(by) = step {
            picking.step(by);
            return self.show_translation(Transition::Refine, None);
        }
        let edit = &mut picking.query;
        let changed = match key {
            VK_LEFT if ctrl => edit.move_caret(Motion::WordBack),
            VK_LEFT => edit.move_caret(Motion::CharBack),
            VK_RIGHT if ctrl => edit.move_caret(Motion::WordForward),
            VK_RIGHT => edit.move_caret(Motion::CharForward),
            VK_HOME => edit.move_caret(Motion::Start),
            VK_END => edit.move_caret(Motion::End),
            VK_DELETE => {
                let to = if ctrl {
                    Motion::WordForward
                } else {
                    Motion::CharForward
                };
                if edit.delete(to) {
                    let current = current.to_string();
                    picking.refilter(&current);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.show_translation(Transition::Refine, None);
        }
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
        if self.picking().is_some_and(|(picking, _)| picking.highlight(&code)) {
            self.show_translation(Transition::Refine, None);
        }
    }

    /// The mouse wheel: scrolls the list of languages while it's open, or
    /// else the card.
    fn on_wheel(&mut self, delta: i32) {
        if let Some((picking, _)) = self.picking() {
            let rows = -(delta as f32 / 120.0 * WHEEL_ROWS).round() as isize;
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

    /// A click, or Enter, on the translation card.
    fn steer_translation(&mut self, target: Target) {
        let Some(t) = self.translating.as_mut() else {
            return;
        };
        let mut focus = None;
        match target {
            // Open the list, or close it again.
            Target::Language(side) => {
                let open = t.picking.take().map(|p| p.side);
                if open == Some(side) {
                    focus = Some(Target::Language(side));
                } else {
                    let current = match side {
                        Side::From => &t.from,
                        Side::To => &t.to,
                    };
                    t.picking = Some(Picking::new(side, current));
                    self.take_keyboard();
                }
                return self.show_translation(Transition::Refine, focus);
            }
            Target::Swap => {
                t.picking = None;
                std::mem::swap(&mut t.from, &mut t.to);
            }
            // The other side's language swaps them round.
            Target::Pick(language) => {
                let Some(side) = t.picking.take().map(|p| p.side) else {
                    return;
                };
                focus = Some(Target::Language(side));
                let (picked, other) = match side {
                    Side::From => (&mut t.from, &mut t.to),
                    Side::To => (&mut t.to, &mut t.from),
                };
                if language == *picked {
                    return self.show_translation(Transition::Refine, focus);
                }
                if language == *other {
                    std::mem::swap(picked, other);
                } else {
                    *picked = language;
                }
            }
            Target::Confirm => t.asking = false,
            _ => return,
        }
        // Until the way is confirmed, only the offer changes.
        if !t.asking {
            self.request += 1;
            self.fetch_translation();
        }
        self.show_translation(Transition::Refine, focus);
    }

    /// Tab and Shift+Tab move between the card's links, and Enter opens the
    /// one they reached, but only while there's one to go to: otherwise the
    /// keys belong to the app underneath. The search box reads its own keys.
    fn sync_link_keys(&mut self) {
        // As does the list of languages.
        let picking = self.translating.as_ref().is_some_and(|t| t.picking.is_some());
        let popup = self
            .popup
            .as_ref()
            .filter(|p| p.visible && self.search.is_none() && !picking);
        let tab = popup.is_some_and(|p| p.has_links());
        let enter = popup.is_some_and(|p| p.focused_link().is_some());
        // Ctrl+T, likewise, only while there's a translation to go to or from.
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        let translate = visible && (self.can_translate() || self.translating.is_some());
        self.set_link_keys(tab, enter);
        set_hotkeys(self.hwnd, TRANSLATE_KEY, &mut self.translate_key, translate);
    }

    fn set_link_keys(&mut self, tab: bool, enter: bool) {
        set_hotkeys(self.hwnd, LINK_KEYS, &mut self.link_keys, tab);
        set_hotkeys(self.hwnd, OPEN_KEY, &mut self.open_key, enter);
    }

    /// Tab or Shift+Tab: rings the next or previous link.
    fn step_link(&mut self, forward: bool) {
        if let Some(popup) = self.popup.as_mut().filter(|p| p.visible) {
            let _ = popup.step_link(forward);
        }
        self.sync_link_keys();
    }

    /// Enter: opens the link Tab moved to.
    fn open_focused_link(&mut self) {
        let target = self
            .popup
            .as_ref()
            .filter(|p| p.visible)
            .and_then(|p| p.focused_link())
            .cloned();
        if let Some(target) = target {
            self.open_link(target);
        }
    }

    fn open_link(&mut self, target: Target) {
        match target {
            Target::Word(word) => self.follow(&word),
            Target::Url(url) => {
                win::shell_open(&HSTRING::from(url));
                self.dismiss();
            }
            Target::Translate => self.translate(),
            Target::Language(_) | Target::Swap | Target::Pick(_) | Target::Confirm => {
                self.steer_translation(target)
            }
        }
    }

    fn dismiss(&mut self) {
        self.history.clear();
        self.translating = None;
        self.sync_back_keys();
        self.set_link_keys(false, false);
        set_hotkeys(self.hwnd, TRANSLATE_KEY, &mut self.translate_key, false);
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
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_MENU_TEARDOWN, IDLE_TEARDOWN_MS, None);
                }
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
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_TEARDOWN, IDLE_TEARDOWN_MS, None);
                }
            }
            // Settings are rarely open: the window goes as soon as it's hidden.
            TIMER_SETTINGS_HIDE if self.settings.as_ref().is_some_and(|s| !s.visible) => {
                if let Some(mut s) = self.settings.take() {
                    s.finish_hide();
                }
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
        }
    }

    // -----------------------------------------------------------------------
    // Tray

    /// A button went down on the tray icon. If that's what closes the menu
    /// (the click-away dismissal gets there first), the release that follows
    /// must not open it again.
    fn on_tray_press(&mut self) {
        let closing = self.menu.as_ref().is_some_and(|m| m.visible)
            || win::now_ms().saturating_sub(self.menu_closed_at) < MENU_REOPEN_GUARD_MS;
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
        if let Err(e) = menu.show(win::cursor_pos(), theme, &hotkey, autostart::is_enabled()) {
            self.tray
                .notify("WinDict couldn't draw its menu", &e.message());
            return;
        }
        CARD_RECT.set(menu.card_rect());
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_MENU_HIDE);
            let _ = KillTimer(Some(self.hwnd), TIMER_MENU_TEARDOWN);
        }
        SHOWN_AT.set(win::now_ms());
        self.install_dismiss_hooks();
    }

    /// Starts closing the tray menu, if it's open.
    fn dismiss_menu(&mut self) {
        let Some(menu) = self.menu.as_mut().filter(|m| m.visible) else {
            return;
        };
        let _ = menu.begin_hide();
        self.menu_closed_at = win::now_ms();
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

    fn run_menu_command(&mut self, command: Command) {
        self.dismiss();
        self.on_command(command);
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
                    Err(e) => {
                        return self
                            .tray
                            .notify("Settings not reloaded, nothing changed", &e)
                    }
                }
                self.cache.clear();
                self.register_hotkey();
                self.refresh_settings();
                if let Err(e) = offline::set_languages(&self.config.languages) {
                    self.tray.notify("Dictionary not loaded", &e);
                } else if self.hotkey_ok {
                    self.tray.notify(
                        "Settings reloaded",
                        &format!("Look up words with {}.", self.config.hotkey),
                    );
                }
            }
            Command::Quit => unsafe {
                self.remove_dismiss_hooks();
                PostQuitMessage(0);
            },
        }
    }

    // -----------------------------------------------------------------------
    // Settings window

    /// What the settings window shows: the settings, with the languages
    /// being downloaded for them ticked already.
    fn settings_values(&self) -> Values {
        let mut config = self.config.clone();
        for installing in self.installing.iter().filter(|i| i.wanted && !i.quiet) {
            let language = installing.download.language;
            if !config.languages.iter().any(|l| l == language) {
                config.languages.push(language.to_owned());
            }
        }
        config.languages.sort_by_key(|l| language_rank(l));
        Values {
            config,
            autostart: autostart::is_enabled(),
        }
    }

    /// Opens the settings window, or brings it back to the front.
    fn open_settings(&mut self) {
        if let Some(hwnd) = self.settings.as_ref().filter(|s| s.visible).map(|s| s.hwnd) {
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
            }
            selection::take_foreground(hwnd);
            return;
        }
        self.dismiss();
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SETTINGS_HIDE);
        }
        let (theme, values) = (self.theme(), self.settings_values());
        let Some(settings) = self.ensure_settings(theme, values.clone()) else {
            return;
        };
        if let Err(e) = settings.show(win::cursor_pos(), theme, values) {
            self.tray
                .notify("WinDict couldn't draw its settings", &e.message());
            return;
        }
        selection::take_foreground(settings.hwnd);
    }

    fn close_settings(&mut self) {
        let Some(settings) = self.settings.as_mut().filter(|s| s.visible) else {
            return;
        };
        let recording = settings.recording();
        let _ = settings.begin_hide();
        // Recording had put the shortcut aside.
        if recording {
            self.register_hotkey();
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SETTINGS_HIDE, HIDE_DONE_MS, None);
        }
    }

    /// The settings changed from elsewhere (the tray menu, or the file): the
    /// window shows them as they are now.
    fn refresh_settings(&mut self) {
        let (theme, values) = (self.theme(), self.settings_values());
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.refresh(theme, values);
        }
    }

    /// Runs `f` on the open settings window, and carries out what it asks for.
    fn in_settings(
        &mut self,
        f: impl FnOnce(&mut Settings) -> windows::core::Result<Option<Change>>,
    ) {
        let change = self
            .settings
            .as_mut()
            .filter(|s| s.visible)
            .and_then(|s| f(s).ok().flatten());
        if let Some(change) = change {
            self.on_setting(change);
        }
    }

    /// Carries out a change made in the settings window, and saves it.
    fn on_setting(&mut self, change: Change) {
        match change {
            Change::Close => return self.close_settings(),
            Change::OpenConfig => return open_config_file(),
            Change::Autostart(on) => return autostart::set(on),
            // The shortcut being replaced mustn't fire as it's pressed to keep it.
            Change::RecordingStarted => unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_LOOKUP);
                return;
            },
            Change::RecordingStopped => return self.register_hotkey(),
            Change::Hotkey(hotkey) => {
                let previous = std::mem::replace(&mut self.config.hotkey, hotkey);
                if let Err(unbound) = self.bind_hotkey() {
                    self.config.hotkey = previous;
                    self.register_hotkey();
                    let why = match &unbound {
                        Unbound::Taken => settings::SHORTCUT_TAKEN,
                        Unbound::Invalid(e) => e,
                    };
                    if let Some(settings) = self.settings.as_mut() {
                        let _ = settings.reject_hotkey(&self.config.hotkey, why);
                    }
                    return;
                }
            }
            Change::Theme(mode) => {
                self.config.theme = mode;
                let theme = self.theme();
                if let Some(settings) = self.settings.as_mut() {
                    let _ = settings.retheme(theme);
                }
            }
            Change::Language(language, on) => return self.tick_language(language, on),
            Change::MaxDefinitions(n) => self.config.max_definitions = n,
            Change::MaxSynonyms(n) => self.config.max_synonyms = n,
            Change::ShowExamples(on) => self.config.show_examples = on,
            Change::OnlineLookup(on) => {
                self.config.online_lookup = on;
                // The translate button needs the internet, like the answers.
                let translate = self.can_translate();
                if let Some(popup) = self.popup.as_mut() {
                    let _ = popup.set_translate(translate);
                }
                self.sync_link_keys();
            }
            Change::UiAutomation(on) => self.config.use_ui_automation = on,
            // These don't change any answer: there's nothing to forget.
            Change::TranslateFrom(code) => return self.save_setting(|c| c.translate_from = code),
            Change::TranslateTo(code) => return self.save_setting(|c| c.translate_to = code),
            Change::MyMemoryEmail(email) => {
                return self.save_setting(|c| c.mymemory_email = email);
            }
        }
        // Online answers were cut down to the old limits, examples and all.
        self.cache.clear();
        self.save_setting(|_| {});
    }

    /// Changes the config with `change`, and saves it.
    fn save_setting(&mut self, change: impl FnOnce(&mut Config)) {
        change(&mut self.config);
        if let Err(e) = config::save(&self.config) {
            self.tray.notify("Settings not saved", &e);
        }
    }

    /// Looks words up in `language` too from now on, or no longer: English
    /// at once, another once its dictionary is downloaded, if it isn't
    /// already. The last language left stays.
    fn tick_language(&mut self, language: String, on: bool) {
        if on {
            if let Some(download) = dicts::find(&language) {
                return self.install(download, false);
            }
            if !self.config.languages.contains(&language) {
                self.config.languages.push(language);
            }
        } else {
            for installing in self.installing.iter_mut() {
                if installing.download.language == language {
                    installing.wanted = false;
                }
            }
            self.show_download(&language, None);
            if self.config.languages.len() > 1 {
                self.config.languages.retain(|l| *l != language);
            }
        }
        self.use_languages();
    }

    /// Makes sure `download` is in place, fetching it if it isn't, then
    /// looks words up in its language too. Asked for again while it
    /// downloads, the download under way carries on.
    fn install(&mut self, download: &'static dicts::Download, quiet: bool) {
        let language = download.language;
        if let Some(installing) = self
            .installing
            .iter_mut()
            .find(|i| i.download.language == language)
        {
            installing.wanted = true;
            installing.quiet &= quiet;
            return;
        }
        self.installing.push(Installing {
            download,
            quiet,
            wanted: true,
        });
        let events = self.events.clone();
        std::thread::spawn(move || {
            let result = dicts::install(download, |percent| {
                events.send(Event::Installing(language, percent));
            });
            events.send(Event::Installed(language, result));
        });
    }

    fn on_installing(&mut self, language: &str, percent: u32) {
        let shown = self
            .installing
            .iter()
            .any(|i| i.download.language == language && i.wanted && !i.quiet);
        if !shown {
            return;
        }
        let note = format!("Downloading\u{2026} {percent}%");
        self.show_download(language, Some((&note, false)));
    }

    fn on_installed(&mut self, language: &str, result: Result<(), String>) {
        let Some(at) = self
            .installing
            .iter()
            .position(|i| i.download.language == language)
        else {
            return;
        };
        let installing = self.installing.remove(at);
        if !installing.wanted {
            return;
        }
        let download = installing.download;
        match result {
            Ok(()) => {
                self.show_download(language, None);
                let used = offline::languages().iter().any(|l| l == language);
                if !self.config.languages.iter().any(|l| l == language) {
                    self.config.languages.push(language.to_owned());
                } else if used {
                    // Already in use, as it was at start.
                    return;
                }
                self.use_languages();
            }
            // At start, the one there is (if any) is used as it is.
            Err(e) if installing.quiet => {
                if !offline::languages().iter().any(|l| l == language) {
                    let title = format!("{} not downloaded", download.name);
                    self.tray.notify(&title, &e);
                }
            }
            Err(e) => {
                self.show_languages();
                self.show_download(language, Some((&e, true)));
            }
        }
    }

    /// Looks words up in the languages of the setting, in order of
    /// preference, and saves it.
    fn use_languages(&mut self) {
        self.config.languages.sort_by_key(|l| language_rank(l));
        if let Err(e) = offline::set_languages(&self.config.languages) {
            self.tray.notify("Dictionary not loaded", &e);
        }
        // Online answers are for the languages there were.
        self.cache.clear();
        if let Err(e) = config::save(&self.config) {
            self.tray.notify("Settings not saved", &e);
        }
        self.show_languages();
    }

    /// Ticks the languages used, and those being downloaded, in the
    /// settings window.
    fn show_languages(&mut self) {
        let languages = self.settings_values().config.languages;
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.show_languages(&languages);
        }
    }

    /// Says how `language`'s download is going in the settings window.
    fn show_download(&mut self, language: &str, status: Option<(&str, bool)>) {
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.show_download(language, status);
        }
    }

    /// The system's colors changed: the settings window follows them.
    fn on_colors_changed(&mut self) {
        let theme = self.theme();
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.retheme(theme);
        }
    }
}

/// Where `language` comes in the order of preference: that of the
/// settings' list, then any it doesn't know.
fn language_rank(language: &str) -> usize {
    dicts::languages()
        .position(|(code, _)| code == language)
        .unwrap_or(usize::MAX)
}

/// Registers `keys` on `hwnd`, or unregisters them, as `wanted`; `on` says
/// which they are.
fn set_hotkeys(hwnd: HWND, keys: Keys, on: &mut bool, wanted: bool) {
    if *on == wanted {
        return;
    }
    *on = wanted;
    for &(id, modifiers, vk) in keys {
        unsafe {
            let _ = if wanted {
                RegisterHotKey(Some(hwnd), id, modifiers, vk.0 as u32)
            } else {
                UnregisterHotKey(Some(hwnd), id)
            };
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
