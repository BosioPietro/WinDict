//! The introduction shown the first time WinDict starts: a tour of the card
//! and the shortcuts and, in the installed copy, the offer to show the card
//! above Start and Search (see `installed::set_ui_access`).
//!
//! Only the hooks are here so far. Until the introduction exists, the first
//! start shows the notification WinDict has always shown, and counts as
//! introduced.

use super::*;

impl App {
    /// At start, when the introduction hasn't been shown yet.
    pub(super) fn start_onboarding(&mut self) {
        if self.hotkey_ok {
            self.tray.notify(
                tr().running,
                &fill(tr().running_detail, &[("hotkey", &self.config.hotkey)]),
            );
        }
        self.finish_onboarding();
    }

    /// Once the introduction is done or closed: it isn't shown again.
    pub(super) fn finish_onboarding(&mut self) {
        self.save_setting(|config| config.onboarded = true);
    }
}
