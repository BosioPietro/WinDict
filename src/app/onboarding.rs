//! The introduction shown the first time WinDict starts (see `intro`): a
//! tour of the shortcuts, each with a card to show it, and, in the
//! installed copy, the offer to show the card above Start and Search (see
//! `installed::set_ui_access`).

use super::*;
use crate::installed::UiAccessError;
use crate::intro::{Action, Intro, Options, Shortcuts, Step};

/// What "Show me" looks up: in the dictionary that comes with WinDict, a
/// noun and a verb, with synonyms and examples for both. And what it
/// translates, short enough for one line.
const EXAMPLE_WORD: &str = "gleam";
const EXAMPLE_PASSAGE: &str = "I'll keep you in the loop";

impl App {
    /// At start, when the introduction hasn't been shown yet.
    pub(super) fn start_onboarding(&mut self) {
        self.open_intro(Step::Lookup);
    }

    /// Opens the introduction on `start`. Should it fail, the notification
    /// WinDict showed before it had one shows, and it's done.
    fn open_intro(&mut self, start: Step) -> bool {
        let theme = self.theme();
        let steps = Step::all(installed::ui_access_available());
        let (shortcuts, options) = self.intro_state();
        let shown = self.ensure_intro(theme).is_some_and(|intro| {
            intro
                .show(win::cursor_pos(), theme, steps, start, shortcuts, options)
                .is_ok()
        });
        match self.intro.as_ref().filter(|_| shown) {
            Some(intro) => {
                selection::take_foreground(intro.hwnd);
                true
            }
            None => {
                self.intro = None;
                if self.hotkey_ok {
                    self.tray.notify(
                        tr().running,
                        &fill(tr().running_detail, &[("hotkey", &self.config.hotkey)]),
                    );
                }
                self.finish_onboarding();
                false
            }
        }
    }

    /// Once the introduction is done or closed: it isn't shown again.
    pub(super) fn finish_onboarding(&mut self) {
        self.save_setting(|config| config.onboarded = true);
        if let Some(intro) = self.intro.as_mut().filter(|i| i.visible) {
            let _ = intro.begin_hide();
            win::set_timer(self.hwnd, TIMER_INTRO_HIDE, HIDE_DONE_MS);
        }
    }

    /// The shortcuts, and what the steps offer, as things stand.
    fn intro_state(&self) -> (Shortcuts, Options) {
        let shortcuts = Shortcuts {
            lookup: self.config.hotkey(Shortcut::Lookup).to_owned(),
            translate: self.config.hotkey(Shortcut::Translate).to_owned(),
        };
        let options = Options {
            shortcut_taken: !self.hotkey_ok,
            online: self.config.online_lookup,
            ui_access: installed::ui_access_on(),
            autostart: autostart::is_enabled(),
        };
        (shortcuts, options)
    }

    /// Shows the introduction what changed elsewhere, while it's open: in
    /// the settings, or the tray menu.
    pub(super) fn refresh_intro(&mut self) {
        let (shortcuts, options) = self.intro_state();
        if let Some(intro) = self.intro.as_mut().filter(|i| i.visible) {
            let _ = intro.update(shortcuts, options);
        }
    }

    /// Runs `f` on the introduction, while it's open, and carries out what
    /// it asks for.
    pub(super) fn in_intro(
        &mut self,
        f: impl FnOnce(&mut Intro) -> windows::core::Result<Option<Action>>,
    ) {
        let action = self
            .intro
            .as_mut()
            .filter(|i| i.visible)
            .and_then(|i| f(i).ok().flatten());
        match action {
            Some(Action::Close) => self.finish_onboarding(),
            Some(Action::ShowMe(step)) => self.show_example(step),
            Some(Action::ChooseShortcut) => self.open_settings(),
            Some(Action::UiAccess(on)) => self.ask_ui_access(on),
            Some(Action::Autostart(on)) => {
                autostart::set(on);
                self.refresh_settings();
            }
            None => {}
        }
    }

    /// Opens a card beside the introduction as `step`'s shortcut would, on
    /// an example.
    fn show_example(&mut self, step: Step) {
        let Some(intro) = self.intro.as_ref() else {
            return;
        };
        let anchor = popup::anchor_beside(intro.card_rect(), self.config.zoom_factor());
        self.dismiss();
        match step {
            Step::Translate => self.translate_selection(EXAMPLE_PASSAGE, Some(anchor), false),
            _ => {
                self.new_request();
                let query = Query {
                    word: EXAMPLE_WORD.into(),
                    language: None,
                };
                self.lookup(query, Place::New(anchor));
            }
        }
    }

    /// Turns UI access on or off, which restarts WinDict, on a thread of its
    /// own: it waits for Windows to ask, and for the script that does it.
    /// It's said how that went once WinDict is back: on the introduction's
    /// step, if it was turned there.
    pub(super) fn ask_ui_access(&mut self, on: bool) {
        self.save_setting(|config| config.ui_access_asked = Some(on));
        let events = self.events.clone();
        std::thread::spawn(move || {
            if let Err(error) = installed::set_ui_access(on) {
                events.send(Event::UiAccess(error));
            }
        });
    }

    /// UI access stayed as it was: the switch goes back, saying why.
    pub(super) fn on_ui_access_failed(&mut self, error: UiAccessError) {
        self.save_setting(|config| config.ui_access_asked = None);
        let note = match error {
            UiAccessError::Cancelled => tr().intro_ui_access_cancelled.to_owned(),
            // Only the installed copy offers it.
            UiAccessError::NotInstalled => tr().ui_access_unchanged.to_owned(),
            UiAccessError::Failed(e) => fill(tr().intro_ui_access_failed, &[("error", &e)]),
        };
        let on = installed::ui_access_on();
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.settle_ui_access(on, &note, true);
        }
        if let Some(intro) = self.intro.as_mut().filter(|i| i.visible) {
            let _ = intro.settle_ui_access(on, Some((note, true)));
        }
    }

    /// At start, once WinDict has restarted for UI access to be turned
    /// `on` or off: back on the introduction's step, if it was turned there,
    /// or else in the settings, under the switch; failing that, in a
    /// notification.
    pub(super) fn after_ui_access(&mut self, on: bool) {
        self.save_setting(|config| config.ui_access_asked = None);
        let ui_access = installed::ui_access_on();
        let failed = ui_access != on;
        let (title, detail) = match (failed, on) {
            (false, true) => (tr().ui_access_on, tr().ui_access_on_detail),
            (false, false) => (tr().ui_access_off, tr().ui_access_off_detail),
            (true, _) => (tr().ui_access_unchanged, tr().ui_access_unchanged_detail),
        };
        if !self.config.onboarded {
            if self.open_intro(Step::AboveStart) {
                if let Some(intro) = self.intro.as_mut() {
                    let _ = intro.settle_ui_access(ui_access, Some((detail.to_owned(), failed)));
                }
                return;
            }
        } else {
            self.open_settings();
            if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
                let _ = settings.settle_ui_access(ui_access, detail, failed);
                return;
            }
        }
        self.tray.notify(title, detail);
    }

    fn ensure_intro(&mut self, theme: Theme) -> Option<&mut Intro> {
        if self.intro.is_none() {
            let title = HSTRING::from(tr().intro_title);
            let window = Window::create(
                INTRO_CLASS,
                PCWSTR(title.as_ptr()),
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
                WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
            )
            .ok()?;
            self.intro = Intro::new(window, self.compositor.clone(), theme).ok();
        }
        self.intro.as_mut()
    }
}
