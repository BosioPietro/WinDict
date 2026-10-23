//! The introduction shown the first time WinDict starts (see `intro`): a
//! tour of the shortcuts and, in the installed copy, the offer to show the
//! card above Start and Search (see `installed::set_ui_access`).

use super::*;
use crate::installed;
use crate::intro::{Action, Intro, Shortcuts, Step};

impl App {
    /// At start, when the introduction hasn't been shown yet.
    pub(super) fn start_onboarding(&mut self) {
        let theme = self.theme();
        let steps = Step::all(installed::ui_access_available());
        let shortcuts = Shortcuts {
            lookup: self.config.hotkey(Shortcut::Lookup).to_owned(),
            translate: self.config.hotkey(Shortcut::Translate).to_owned(),
        };
        let shown = self.ensure_intro(theme).is_some_and(|intro| {
            intro
                .show(win::cursor_pos(), theme, steps, shortcuts)
                .is_ok()
        });
        match self.intro.as_ref().filter(|_| shown) {
            Some(intro) => {
                selection::take_foreground(intro.hwnd);
            }
            // Without it, the notification WinDict showed before it had one.
            None => {
                self.intro = None;
                if self.hotkey_ok {
                    self.tray.notify(
                        tr().running,
                        &fill(tr().running_detail, &[("hotkey", &self.config.hotkey)]),
                    );
                }
                self.finish_onboarding();
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
            None => {}
        }
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
